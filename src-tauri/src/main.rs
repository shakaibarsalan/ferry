#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! ccvault - manage Claude Code chats across the local accounts on this Mac.

use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "windows")]
fn home() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default()
}
#[cfg(not(target_os = "windows"))]
fn home() -> String { std::env::var("HOME").unwrap_or_default() }

/// Compare by path components, never by string prefix: on Windows these paths
/// mix "/" and "\" depending on whether they came from a format! or from glob.
fn under(root: &str, p: &str) -> bool {
    Path::new(p).starts_with(Path::new(root))
}

/// Where the desktop app keeps its per-account state.
#[cfg(target_os = "macos")]
fn claude_dir() -> String { format!("{}/Library/Application Support/Claude", home()) }
/// The Microsoft Store build of Claude runs in an MSIX container that silently
/// redirects %APPDATA%\Claude to
/// %LOCALAPPDATA%\Packages\Claude_<publisher>\LocalCache\Roaming\Claude.
/// Only processes inside the package see the redirect, so a standalone Ferry
/// finds %APPDATA%\Claude empty. Look in both; if more than one has chats,
/// take the one written to most recently.
#[cfg(target_os = "windows")]
fn claude_dir() -> String {
    let appdata = std::env::var("APPDATA")
        .unwrap_or_else(|_| format!("{}/AppData/Roaming", home()));
    let local = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| format!("{}/AppData/Local", home()));
    let mut cands = vec![format!("{}\\Claude", appdata)];
    if let Ok(rd) = fs::read_dir(format!("{}\\Packages", local)) {
        let mut pkgs: Vec<String> = rd.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
            .map(|e| format!("{}\\LocalCache\\Roaming\\Claude", e.path().display()))
            .collect();
        pkgs.sort();
        cands.extend(pkgs);
    }
    let newest = |c: &String| -> Option<SystemTime> {
        walkdir::WalkDir::new(format!("{}\\claude-code-sessions", c)).max_depth(3)
            .into_iter().filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy();
                n.starts_with("local_") && n.ends_with(".json")
            })
            .filter_map(|e| e.metadata().ok()?.modified().ok())
            .max()
    };
    cands.iter()
        .filter(|c| Path::new(&format!("{}\\claude-code-sessions", c)).is_dir())
        .max_by_key(|c| newest(c))
        .cloned()
        .unwrap_or_else(|| cands[0].clone())
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn claude_dir() -> String { format!("{}/.config/Claude", home()) }

fn sess()  -> String { format!("{}/claude-code-sessions", claude_dir()) }
fn cfg()   -> String { format!("{}/config.json", claude_dir()) }
fn idb()   -> String { format!("{}/IndexedDB", claude_dir()) }
fn proj()  -> String { format!("{}/.claude/projects", home()) }
fn vault() -> String { format!("{}/.ferry", home()) }

/// Connector uuids and tool grants belong to the account that created them.
const ACCOUNT_SCOPED: [&str; 2] = ["remoteMcpServersConfig", "enabledMcpTools"];

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}
fn stamp() -> String {
    // yyyymmdd-hhmmss without pulling in chrono
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    // Windows has no `date -r`; spawning one would only flash a console window
    if cfg!(target_os = "windows") { return secs.to_string(); }
    let out = Command::new("date").args(["-r", &secs.to_string(), "+%Y%m%d-%H%M%S"]).output();
    out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
       .filter(|s| !s.is_empty()).unwrap_or_else(|| secs.to_string())
}
/// Current Claude Code rule, read out of the shipped CLI:
/// every character that is not [a-zA-Z0-9] becomes '-', runs are NOT collapsed.
fn enc_cwd(p: &str) -> String {
    p.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}
/// Older builds only replaced the separators and left dots alone.
fn enc_cwd_legacy(p: &str) -> String {
    p.chars().map(|c| if c=='/' || c=='\\' { '-' } else { c }).collect()
}

/// Resolve a chat's working directory to its transcript folder.
/// Folders written by different Claude Code versions coexist on disk, and very
/// long paths are truncated with a "-<hash>" suffix, so try each in turn.
fn project_dir(cwd: &str) -> String {
    let root = proj();
    let current = enc_cwd(cwd);
    for cand in [&current, &enc_cwd_legacy(cwd)] {
        let d = format!("{}/{}", root, cand);
        if Path::new(&d).is_dir() { return d; }
    }
    // truncated long path: "<prefix>-<base36 hash>"
    if let Ok(rd) = fs::read_dir(&root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some((prefix, _hash)) = name.rsplit_once('-') {
                if prefix.len() >= 24 && current.starts_with(prefix) {
                    return format!("{}/{}", root, name);
                }
            }
        }
    }
    format!("{}/{}", root, current)
}
fn read_json(p: &str) -> Option<Value> {
    fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
}
#[cfg(target_os = "macos")]
fn app_running() -> bool {
    Command::new("pgrep").args(["-f", "Claude.app/Contents/MacOS/Claude"])
        .output().map(|o| !o.stdout.is_empty()).unwrap_or(false)
}
/// The desktop app is Chromium: while it runs it holds <user data>\lockfile open
/// with no sharing, and Windows deletes the file when the process exits, crash
/// included. So "can't open it" means running. This replaces a PowerShell
/// process query that took ~2 s, flashed a console window, and only matched
/// the Store install.
#[cfg(target_os = "windows")]
fn app_running() -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match fs::File::open(format!("{}\\lockfile", claude_dir())) {
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
        Ok(_) => false,
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn app_running() -> bool { false }
fn guard() -> Result<(), String> {
    if app_running() { Err("Claude desktop is running. Quit it first, then retry.".into()) }
    else { Ok(()) }
}
fn snapshot(p: &str, tag: &str) {
    if !Path::new(p).exists() { return; }
    let name = Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string();
    let dir = format!("{}/snapshots", vault());
    let _ = fs::create_dir_all(&dir);
    let _ = fs::copy(p, format!("{}/{}-{}-{}", dir, stamp(), tag, name));
}
fn labels_path() -> String { format!("{}/labels.json", vault()) }
fn prefs_path()  -> String { format!("{}/prefs.json",  vault()) }
fn last_export_dir() -> String {
    read_json(&prefs_path())
        .and_then(|p| p["lastExportDir"].as_str().map(String::from))
        .filter(|d| Path::new(d).is_dir())
        .unwrap_or_else(|| format!("{}/Downloads", home()))
}
fn set_last_export_dir(d: &str) {
    let _ = fs::create_dir_all(vault());
    let mut p = read_json(&prefs_path()).unwrap_or_else(|| json!({}));
    p["lastExportDir"] = json!(d);
    let _ = fs::write(prefs_path(), serde_json::to_string_pretty(&p).unwrap());
}
fn labels() -> Value { read_json(&labels_path()).unwrap_or_else(|| json!({})) }

/// Only the currently logged-in account leaves a profile record; logout clears it.
fn profiles() -> Value {
    let mut out = json!({});
    let re = regex::bytes::Regex::new(
        // Claude's own UI shows display_name ("ahkamboh"), not full_name
        // ("Ali Hamza Kamboh"), so capture both and prefer the former.
        r"(?s)([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}).{0,24}?email_address.{0,4}?([A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,})(?:.{0,16}?full_name.{0,4}?([A-Za-z0-9 ._\-]{2,40}))?(?:.{0,16}?display_name.{0,4}?([A-Za-z0-9 ._\-]{2,40}))?"
    ).unwrap();
    for e in walkdir::WalkDir::new(idb()).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() { continue; }
        let Ok(b) = fs::read(e.path()) else { continue };
        for c in re.captures_iter(&b) {
            let uuid = String::from_utf8_lossy(&c[1]).to_string();
            let email = String::from_utf8_lossy(&c[2]).to_string();
            let pick = |i: usize| c.get(i)
                .map(|m| String::from_utf8_lossy(m.as_bytes()).trim().to_string())
                .filter(|x| !x.is_empty());
            let name = pick(4).or_else(|| pick(3)).unwrap_or_default();   // display_name, else full_name
            out[uuid] = json!({ "email": email, "name": name });
        }
    }
    known_profiles(&mut out);
    out
}

/// IndexedDB only knows the signed-in account, and Claude keeps it locked while
/// it runs. Claude Code's own config names its account too - "oauthAccount" in
/// ~/.claude.json and its backups - and account switchers such as claude-swap
/// keep one copy of that file per account. Every account seen in any of them is
/// remembered in ~/.ferry/profiles.json, so it keeps its name after sign-out.
/// Not Windows-specific: ~/.claude.json and claude-swap live in the same place
/// on macOS, and IndexedDB there only ever names the account you are signed in to.
fn known_profiles(found: &mut Value) {
    let h = home();
    let cache = format!("{}/profiles.json", vault());
    let mut known = read_json(&cache).filter(|v| v.is_object()).unwrap_or_else(|| json!({}));
    let before = known.clone();
    let mut files: Vec<PathBuf> = vec![PathBuf::from(format!("{}/.claude.json", h)),
                                       PathBuf::from(format!("{}/.claude.json.backup", h))];
    for pat in [format!("{}/.claude/backups/.claude.json.backup*", h),
                format!("{}/.claude-swap-backup/configs/*.json", h),
                // claude-swap names them .claude-config-<n>-<email>.json - a leading dot
                format!("{}/.claude-swap-backup/configs/.*.json", h)] {
        if let Ok(g) = glob::glob(&pat) { files.extend(g.flatten()); }
    }
    let mut from_json: std::collections::BTreeMap<String, String> = Default::default();
    for f in files {
        let Some(v) = read_json(&f.to_string_lossy()) else { continue };
        let oa = &v["oauthAccount"];
        if let (Some(u), Some(e)) = (oa["accountUuid"].as_str(), oa["emailAddress"].as_str()) {
            let name = oa["displayName"].as_str()
                .filter(|x| !x.is_empty())
                .or(oa["fullName"].as_str()).unwrap_or("");
            from_json.insert(u.to_string(), name.to_string());
            known[u] = json!({ "email": e, "name": name });
        }
    }
    // IndexedDB knows who is signed in right now, but its name is scraped out of a
    // binary and can come back truncated ("ahka" for "ahkamboh"), so a parsed
    // oauthAccount field wins over it.
    if let Some(o) = found.as_object() {
        for (k, v) in o {
            let prev = known[k.as_str()]["name"].as_str().unwrap_or("").to_string();
            let name = from_json.get(k)
                .filter(|x| !x.is_empty()).cloned()
                .or_else(|| v["name"].as_str().filter(|x| !x.is_empty()).map(String::from))
                .unwrap_or(prev);
            let email = v["email"].as_str()
                .filter(|x| !x.is_empty()).map(String::from)
                .unwrap_or_else(|| known[k.as_str()]["email"].as_str().unwrap_or("").to_string());
            known[k.as_str()] = json!({ "email": email, "name": name });
        }
    }
    if known != before {
        let _ = fs::create_dir_all(vault());
        let _ = fs::write(&cache, serde_json::to_string_pretty(&known).unwrap());
    }
    *found = known;
}

/// Every transcript that makes up one chat: current + prior sessions + their subagents.
fn transcripts_for(rec: &Value) -> Vec<Value> {
    let cwd = rec["cwd"].as_str().unwrap_or("");
    let dir = project_dir(cwd);
    let mut ids: Vec<String> = vec![];
    if let Some(s) = rec["cliSessionId"].as_str() { ids.push(s.into()); }
    // older builds: priorCliSessionIds. newer / Windows builds: bridgeSessionIds.
    for key in ["priorCliSessionIds", "bridgeSessionIds"] {
        if let Some(a) = rec[key].as_array() {
            for v in a {
                if let Some(x) = v.as_str() {
                    if !ids.iter().any(|e| e == x) { ids.push(x.into()); }
                }
            }
        }
    }
    ids.iter().map(|id| {
        let main = format!("{}/{}.jsonl", dir, id);
        let size = fs::metadata(&main).map(|m| m.len()).unwrap_or(0);
        let mut subs = 0u64; let mut sub_bytes = 0u64;
        if let Ok(rd) = glob::glob(&format!("{}/{}/subagents/*.jsonl", dir, id)) {
            for p in rd.flatten() {
                subs += 1;
                sub_bytes += fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            }
        }
        json!({ "id": id, "path": main, "exists": Path::new(&main).exists(),
                "size": size, "subagents": subs, "subBytes": sub_bytes })
    }).collect()
}

/* ---------- sessions no account claims ----------
   Claude Code writes a transcript for every session it runs, wherever it runs:
   the CLI, the VS Code extension and the desktop app all append to the same
   ~/.claude/projects tree. Only the desktop app also writes the small
   per-account record Ferry lists, so a chat started in the CLI or in VS Code is
   on disk and belongs to nobody - readable, but invisible to every account.
   These are listed as read-only sources. A chat can be imported out of one into
   an account; nothing is ever written back into them. */

/// The surface that wrote a transcript, as the file itself reports it.
fn source_of(entrypoint: &str) -> (&'static str, &'static str) {
    match entrypoint {
        "cli"            => ("cli",     "Claude Code CLI"),
        "claude-vscode"  => ("vscode",  "VS Code"),
        "claude-desktop" => ("desktop", "Desktop, no record"),
        "cursor"         => ("cursor",  "Cursor"),
        _                => ("other",   "Other sessions"),
    }
}

/// Days since 1970-01-01 for a civil date. Hinnant's algorithm, so that reading
/// a timestamp costs no dependency.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "2026-09-12T08:44:01.123Z" to epoch milliseconds. Transcripts date every line
/// this way; chat records count in milliseconds, so one has to become the other.
fn iso_ms(s: &str) -> Option<u64> {
    if s.len() < 19 { return None; }
    let num = |a: usize, z: usize| s.get(a..z).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d)  = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let ms = if s.len() >= 23 && s.as_bytes()[19] == b'.' { num(20, 23).unwrap_or(0) } else { 0 };
    let t = (days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se) * 1000 + ms;
    if t < 0 { None } else { Some(t as u64) }
}

/// The readable text of one message, whether it came as a string or as parts.
fn msg_text(c: &Value) -> String {
    if let Some(s) = c.as_str() { return s.to_string(); }
    let Some(arr) = c.as_array() else { return String::new() };
    let mut out = String::new();
    for part in arr {
        if part["type"].as_str() == Some("text") {
            if let Some(x) = part["text"].as_str() {
                if !out.is_empty() { out.push(' '); }
                out.push_str(x);
            }
        }
    }
    out
}

/// A chat's name, taken from the first thing the person typed. The desktop app
/// titles its own chats in a few words, so a whole opening prompt would tower
/// over them in the list: keep the first sentence, and cut that at a word.
/// A full stop only ends a sentence when a space follows it, or "github.com"
/// and "ocid1.tenancy.oc1" would each end one.
fn title_from(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let b = one.as_bytes();
    let mut s = one.as_str();
    for (i, c) in one.char_indices() {
        if c == '.' || c == '!' || c == '?' {
            if i + 1 >= one.len() { break; }     // one sentence: it keeps its mark
            if b[i + 1] == b' ' {
                if (12..=70).contains(&i) { s = &one[..i]; }
                break;
            }
        }
    }
    if s.chars().count() <= 60 { return s.to_string(); }
    let cut: String = s.chars().take(60).collect();
    match cut.rfind(' ') {
        Some(i) if i >= 30 => format!("{}\u{2026}", &cut[..i]),
        _ => format!("{}\u{2026}", cut.trim_end()),
    }
}

/// First non-empty value wins: a transcript states its cwd and version on every
/// line, and the earliest line is the one that describes the session.
fn fill(dst: &mut String, v: Option<&str>) {
    if dst.is_empty() {
        if let Some(x) = v { if !x.is_empty() { *dst = x.to_string(); } }
    }
}

/// Everything a transcript says about itself, in one pass: which surface wrote
/// it, where it ran, when it started and stopped, how many turns it took, and
/// what to call it. Lines over a megabyte are tool output, never metadata, so
/// they are skipped without parsing - that keeps a 90 MB transcript cheap to
/// read and bounds the memory reading it takes.
fn read_session(path: &str) -> Option<Value> {
    let md = fs::metadata(path).ok()?;
    let f = fs::File::open(path).ok()?;
    let mut rd = BufReader::with_capacity(1 << 16, f);
    let mut buf: Vec<u8> = Vec::new();
    let (mut entrypoint, mut cwd, mut version, mut branch, mut model) =
        (String::new(), String::new(), String::new(), String::new(), String::new());
    let (mut title, mut first, mut last) = (String::new(), String::new(), String::new());
    let mut turns = 0u64;

    loop {
        buf.clear();
        match rd.read_until(b'\n', &mut buf) { Ok(0) => break, Ok(_) => {}, Err(_) => break }
        if buf.len() > (1 << 20) { buf = Vec::new(); continue; }   // give the memory back
        let Ok(d) = serde_json::from_slice::<Value>(&buf) else { continue };
        let ty = d["type"].as_str().unwrap_or("");
        if ty != "user" && ty != "assistant" { continue; }

        fill(&mut entrypoint, d["entrypoint"].as_str());
        fill(&mut cwd, d["cwd"].as_str());
        fill(&mut version, d["version"].as_str());
        fill(&mut branch, d["gitBranch"].as_str());
        if ty == "assistant" { fill(&mut model, d["message"]["model"].as_str()); }
        if let Some(t) = d["timestamp"].as_str() {
            if first.is_empty() { first = t.to_string(); }
            if t > last.as_str() { last = t.to_string(); }   // ISO-8601 sorts as time
        }
        // A turn is a prompt the person typed: not a subagent's, and not the
        // harness's own <command-name> and <system-reminder> scaffolding.
        if ty == "user"
           && !d["isSidechain"].as_bool().unwrap_or(false)
           && !d["isMeta"].as_bool().unwrap_or(false) {
            let text = msg_text(&d["message"]["content"]);
            let t = text.trim();
            // "[Request interrupted…]" is written by the harness when you stop a
            // tool, not typed: it is neither a turn nor a name for the chat.
            if !t.is_empty() && !t.starts_with('<') && !t.starts_with("[Request interrupted") {
                turns += 1;
                if title.is_empty() { title = title_from(t); }
            }
        }
    }
    let mtime = md.modified().ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64).unwrap_or(0);
    if title.is_empty() { title = "(untitled)".into(); }
    Some(json!({
        "v": INDEX_V,
        "entrypoint": entrypoint, "cwd": cwd, "version": version, "branch": branch,
        "model": model, "title": title, "turns": turns, "size": md.len(),
        "created": iso_ms(&first).unwrap_or(mtime),
        "last": iso_ms(&last).unwrap_or(mtime),
    }))
}

fn index_path() -> String { format!("{}/sessions.json", vault()) }
/// Bumped whenever what is read out of a transcript changes, so an index
/// written by an older Ferry is re-read rather than believed.
const INDEX_V: u64 = 3;

/// Reading every unclaimed transcript on every scan would mean re-reading
/// hundreds of megabytes to learn nothing new, so what each one said about
/// itself is kept in the vault and re-read only when the file changes.
fn session_info(cache: &mut Value, path: &str, dirty: &mut bool) -> Option<Value> {
    let md = fs::metadata(path).ok()?;
    let mtime = md.modified().ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs()).unwrap_or(0);
    if let Some(hit) = cache.get(path) {
        if hit["v"].as_u64() == Some(INDEX_V)
           && hit["size"].as_u64() == Some(md.len()) && hit["mtime"].as_u64() == Some(mtime) {
            return Some(hit.clone());
        }
    }
    let mut info = read_session(path)?;
    info["mtime"] = json!(mtime);
    cache[path] = info.clone();
    *dirty = true;
    Some(info)
}

/// How many subagent transcripts one session left behind, and how big they are.
fn subagents_of(dir: &str, id: &str) -> (u64, u64) {
    let (mut n, mut bytes) = (0u64, 0u64);
    if let Ok(g) = glob::glob(&format!("{}/{}/subagents/*.jsonl", dir, id)) {
        for p in g.flatten() {
            n += 1;
            bytes += fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        }
    }
    (n, bytes)
}

/// Every transcript no account's record claims, grouped by the surface that
/// wrote it and shaped like an account scope so the rest of the app can list it.
fn source_scopes(claimed: &std::collections::HashSet<String>) -> Vec<Value> {
    let mut cache = read_json(&index_path()).filter(|v| v.is_object()).unwrap_or_else(|| json!({}));
    let mut dirty = false;
    let mut groups: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    let mut names: std::collections::BTreeMap<String, &'static str> = Default::default();
    let mut live: std::collections::HashSet<String> = Default::default();

    if let Ok(paths) = glob::glob(&format!("{}/*/*.jsonl", proj())) {
        for p in paths.flatten() {
            let path = p.to_string_lossy().to_string();
            let Some(id) = p.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
            live.insert(path.clone());
            if claimed.contains(&id) { continue; }
            let Some(info) = session_info(&mut cache, &path, &mut dirty) else { continue };
            // A session the Agent SDK ran is not a chat anyone had: the prompts
            // came from a program. Ferry carries conversations from one account
            // to another, so these are left where they are.
            if matches!(info["entrypoint"].as_str().unwrap_or(""), "sdk-cli" | "sdk") { continue; }
            let dir = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            let (subs, sub_bytes) = subagents_of(&dir, &id);
            let (kind, name) = source_of(info["entrypoint"].as_str().unwrap_or(""));
            names.insert(kind.to_string(), name);
            groups.entry(kind.to_string()).or_default().push(json!({
                "id": format!("local_{}", id), "sid": id,
                "title": info["title"], "cwd": info["cwd"], "model": info["model"],
                "created": info["created"], "last": info["last"], "turns": info["turns"],
                "archived": false, "forkedFrom": Value::Null,
                "files": 1, "subs": subs,
                "bytes": info["size"].as_u64().unwrap_or(0) + sub_bytes,
                "missing": 0, "absent": 0,
                "branch": info["branch"], "version": info["version"],
                "source": kind, "path": path
            }));
        }
    }
    // Forget transcripts retention has since pruned, so the index does not grow
    // forever on a machine that churns through sessions.
    if let Some(o) = cache.as_object_mut() {
        let stale: Vec<String> = o.keys().filter(|k| !live.contains(*k)).cloned().collect();
        if !stale.is_empty() { dirty = true; for k in stale { o.remove(&k); } }
    }
    if dirty {
        let _ = fs::create_dir_all(vault());
        let _ = fs::write(index_path(), serde_json::to_string(&cache).unwrap());
    }

    let mut out: Vec<Value> = groups.into_iter().map(|(kind, mut chats)| {
        chats.sort_by_key(|c| std::cmp::Reverse(c["last"].as_u64().unwrap_or(0)));
        json!({
            "acct": format!("source:{}", kind), "org": "source", "kind": "source",
            "source": kind, "sourceName": names.get(&kind).copied().unwrap_or("Sessions"),
            "chats": chats, "deleted": [], "connectors": {},
            "isCurrent": false, "label": "", "profile": Value::Null
        })
    }).collect();
    out.sort_by_key(|s| std::cmp::Reverse(
        s["chats"].as_array().unwrap().iter().map(|c| c["last"].as_u64().unwrap_or(0)).max().unwrap_or(0)));
    out
}

/* Cursor support is Windows-only. Reading its database means bundling SQLite,
   the one dependency here that costs anything, and none of this has been tried
   on a Mac - so the macOS build takes neither the dependency nor the size, and
   behaves exactly as it did before. */
#[cfg(target_os = "windows")]
mod cursor {
use super::*;

/* ---------- Cursor ----------
   Cursor is a separate application with a storage of its own: one SQLite file
   holding every conversation, not a folder of transcripts. A chat is a row in
   composerHeaders, an ordered list of bubble ids in composerData:<id>, and one
   bubbleId:<chat>:<bubble> row per message. Nothing about it resembles the
   JSONL Claude Code appends, so a chat cannot be moved between them - it has to
   be converted, and the conversion only goes one way. Writing into Cursor's
   database would mean inserting rows into a live file it holds open, where a
   mistake costs every conversation in it, so Ferry never does: every read here
   opens it read-only. */

#[cfg(target_os = "windows")]
fn cursor_dir() -> String {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| format!("{}/AppData/Roaming", home()));
    format!("{}\\Cursor\\User", appdata)
}
#[cfg(target_os = "macos")]
fn cursor_dir() -> String { format!("{}/Library/Application Support/Cursor/User", home()) }
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn cursor_dir() -> String { format!("{}/.config/Cursor/User", home()) }

fn cursor_db() -> Option<String> {
    let p = format!("{}/globalStorage/state.vscdb", cursor_dir());
    if Path::new(&p).exists() { Some(p) } else { None }
}

/// Read-only, and never anything else.
fn cursor_open() -> Option<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    rusqlite::Connection::open_with_flags(cursor_db()?,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI).ok()
}

/// A cell, whether Cursor wrote it as text or as a blob.
fn cell(row: &rusqlite::Row, i: usize) -> String {
    row.get::<_, String>(i).unwrap_or_else(|_| {
        row.get::<_, Vec<u8>>(i).map(|b| String::from_utf8_lossy(&b).to_string())
           .unwrap_or_default()
    })
}

/// Which folder each Cursor workspace is: workspace.json names it as a URI.
fn cursor_folders() -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(g) = glob::glob(&format!("{}/workspaceStorage/*/workspace.json", cursor_dir()))
        else { return out };
    for f in g.flatten() {
        let Some(v) = read_json(&f.to_string_lossy()) else { continue };
        let Some(uri) = v["folder"].as_str() else { continue };
        let Some(rest) = uri.strip_prefix("file:///") else { continue };
        // percent-decoding, for the spaces in "c:/Drive/Local LLM"
        let mut p = String::new();
        let b = rest.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(n) = u8::from_str_radix(&rest[i+1..i+3], 16) {
                    p.push(n as char); i += 3; continue;
                }
            }
            p.push(b[i] as char); i += 1;
        }
        if cfg!(target_os = "windows") { p = p.replace('/', "\\"); } else { p.insert(0, '/'); }
        if let Some(d) = f.parent().and_then(|d| d.file_name()) {
            out.insert(d.to_string_lossy().to_string(), p);
        }
    }
    out
}

/// Every Cursor conversation that has anything in it. Most headers are empty
/// shells left behind by windows that were opened and closed.
pub fn cursor_chats() -> Vec<Value> {
    let Some(c) = cursor_open() else { return vec![] };
    let ws = cursor_folders();
    let Ok(mut st) = c.prepare("select composerId, workspaceId, createdAt, lastUpdatedAt, \
                                isArchived, isSubagent, value from composerHeaders \
                                order by lastUpdatedAt desc") else { return vec![] };
    let Ok(rows) = st.query_map([], |r| Ok((
        cell(r, 0), cell(r, 1),
        r.get::<_, Option<i64>>(2).unwrap_or(None).unwrap_or(0),
        r.get::<_, Option<i64>>(3).unwrap_or(None).unwrap_or(0),
        r.get::<_, Option<i64>>(4).unwrap_or(None).unwrap_or(0),
        r.get::<_, Option<i64>>(5).unwrap_or(None).unwrap_or(0),
        cell(r, 6),
    ))) else { return vec![] };

    let mut out = vec![];
    for row in rows.flatten() {
        let (cid, wid, created, last, arch, sub, val) = row;
        if sub != 0 { continue; }                       // a subagent's side conversation
        let Some(data) = cursor_get(&c, &format!("composerData:{}", cid)) else { continue };
        let Some(d) = serde_json::from_str::<Value>(&data).ok() else { continue };
        let heads = d["fullConversationHeadersOnly"].as_array().cloned().unwrap_or_default();
        if heads.is_empty() { continue; }
        // Most bubbles are tool calls with nothing to read. The header of each
        // says whether it has text, so how much was actually said can be counted
        // without opening three thousand rows.
        let n = heads.iter().filter(|h| h["grouping"]["hasText"].as_bool().unwrap_or(false)).count();
        let n = if n > 0 { n } else { heads.len() };
        let name = serde_json::from_str::<Value>(&val).ok()
            .and_then(|v| v["name"].as_str().map(String::from))
            .filter(|s| !s.is_empty()).unwrap_or_else(|| "(unnamed)".into());
        out.push(json!({
            "id": cid, "title": name, "folder": ws.get(&wid).cloned().unwrap_or_default(),
            "created": created, "last": last, "bubbles": n, "archived": arch != 0
        }));
    }
    out
}

fn cursor_get(c: &rusqlite::Connection, key: &str) -> Option<String> {
    c.query_row("select value from cursorDiskKV where key=?1", [key], |r| Ok(cell(r, 0))).ok()
}

/// One tool call, said in a line. Cursor keeps the arguments as raw JSON; the
/// path or command in them is the part worth reading. Its own call ids say
/// nothing about what the tool did and carry newlines of their own.
fn cursor_tool_line(t: &Value) -> String {
    let name = t["name"].as_str().or(t["tool"].as_str()).unwrap_or("tool");
    let mut hint = String::new();
    if let Ok(a) = serde_json::from_str::<Value>(t["rawArgs"].as_str().unwrap_or("{}")) {
        for k in ["path", "target_file", "file", "command", "query", "pattern",
                  "globPattern", "targetDirectory", "toolName", "explanation"] {
            if let Some(v) = a[k].as_str() { if !v.is_empty() { hint = v.into(); break; } }
        }
        if hint.is_empty() {
            if let Some(o) = a.as_object() {
                let rest: serde_json::Map<String, Value> = o.iter()
                    .filter(|(k, _)| !matches!(k.as_str(),
                        "toolCallId" | "modelCallId" | "toolIndex" | "toolCallBinary"))
                    .map(|(k, v)| (k.clone(), v.clone())).collect();
                if !rest.is_empty() { hint = Value::Object(rest).to_string(); }
            }
        }
    }
    let hint: String = hint.split_whitespace().collect::<Vec<_>>().join(" ")
                           .chars().take(120).collect();
    if hint.is_empty() { format!("-> {}()", name) } else { format!("-> {}({})", name, hint) }
}

/// One Cursor conversation as plain turns, oldest first.
///
/// Most bubbles carry no prose at all - in a 4,563 bubble chat only a couple of
/// hundred do, and three thousand are tool calls. Dropping those would throw
/// away what the conversation actually did, so a run of them is folded into the
/// message before it as one line each. They are written as text, not as
/// tool_use blocks: a tool_use has to be answered by a tool_result, and there
/// is nothing on this side to answer it with.
fn cursor_messages(cid: &str) -> Vec<Value> {
    let Some(c) = cursor_open() else { return vec![] };
    let Some(data) = cursor_get(&c, &format!("composerData:{}", cid)) else { return vec![] };
    let Ok(d) = serde_json::from_str::<Value>(&data) else { return vec![] };
    let heads = d["fullConversationHeadersOnly"].as_array().cloned().unwrap_or_default();

    let mut out: Vec<Value> = vec![];
    let mut pending: Vec<String> = vec![];
    for h in &heads {
        let Some(bid) = h["bubbleId"].as_str() else { continue };
        let Some(raw) = cursor_get(&c, &format!("bubbleId:{}:{}", cid, bid)) else { continue };
        let Ok(b) = serde_json::from_str::<Value>(&raw) else { continue };
        let ts = h["createdAt"].as_str().or(b["createdAt"].as_str()).unwrap_or("").to_string();
        let text = b["text"].as_str().unwrap_or("").trim().to_string();
        let role = if b["type"].as_i64() == Some(1) { "user" } else { "assistant" };
        if !text.is_empty() {
            let mut text = text;
            if role == "user" {
                if !pending.is_empty() {
                    out.push(json!({ "role": "assistant", "text": pending.join("\n"), "t": ts }));
                    pending.clear();
                }
            } else if !pending.is_empty() {
                text = format!("{}\n\n{}", pending.join("\n"), text);
                pending.clear();
            }
            out.push(json!({ "role": role, "text": text, "t": ts }));
        } else if !b["toolFormerData"].is_null() {
            pending.push(cursor_tool_line(&b["toolFormerData"]));
        }
    }
    if !pending.is_empty() {
        let ts = out.last().map(|m| m["t"].clone()).unwrap_or(Value::Null);
        out.push(json!({ "role": "assistant", "text": pending.join("\n"), "t": ts }));
    }
    out
}

/// Message uuids that keep a uuid's shape without a uuid crate: the
/// conversation's own id with its tail replaced, so every line is distinct and
/// the chain from one to the next can be followed.
fn cursor_msg_uuid(cid: &str, i: usize) -> String {
    let mut s = cid.to_string();
    if s.len() >= 8 {
        let n = s.len();
        s.replace_range(n - 8..n, &format!("{:08x}", i as u32));
    }
    s
}

/// A Claude Code transcript, written from turns that came from somewhere else.
/// The shape is the one Claude Code appends: one JSON object a line, each
/// linked to the one before it. entrypoint says where it really came from, so
/// nothing later mistakes it for a session Claude Code ran itself.
pub fn cursor_write_transcript(chat: &Value) -> Result<String, String> {
    let cid = chat["id"].as_str().ok_or("no conversation id")?;
    let cwd = chat["folder"].as_str().unwrap_or("");
    if cwd.is_empty() { return Err("that Cursor chat has no folder on this machine".into()); }
    let msgs = cursor_messages(cid);
    if msgs.is_empty() { return Err("that Cursor chat has nothing readable in it".into()); }

    let dir = project_dir(cwd);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = format!("{}/{}.jsonl", dir, cid);
    let mut body = String::new();
    let mut prev = Value::Null;
    for (i, m) in msgs.iter().enumerate() {
        let u = cursor_msg_uuid(cid, i);
        body.push_str(&json!({
            "parentUuid": prev, "isSidechain": false, "userType": "external",
            "type": m["role"], "message": { "role": m["role"], "content": m["text"] },
            "uuid": u, "timestamp": m["t"], "cwd": cwd, "sessionId": cid,
            "gitBranch": "", "entrypoint": "cursor"
        }).to_string());
        body.push('\n');
        prev = json!(u);
    }
    fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Cursor's conversations as one more source to import from. They have no
/// transcript to point at until one is written, so they are addressed by
/// "cursor:<conversation id>" rather than by a path.
pub fn cursor_scope() -> Option<Value> {
    let chats = cursor_chats();
    if chats.is_empty() { return None; }
    let mut list: Vec<Value> = chats.iter().map(|c| json!({
        "id": format!("local_{}", c["id"].as_str().unwrap_or("")),
        "sid": c["id"], "title": c["title"], "cwd": c["folder"],
        "model": "", "created": c["created"], "last": c["last"],
        "turns": c["bubbles"], "archived": c["archived"], "forkedFrom": Value::Null,
        "files": 1, "subs": 0, "bytes": 0, "missing": 0, "absent": 0,
        "branch": "", "version": "", "source": "cursor",
        "path": format!("cursor:{}", c["id"].as_str().unwrap_or(""))
    })).collect();
    list.sort_by_key(|c| std::cmp::Reverse(c["last"].as_u64().unwrap_or(0)));
    Some(json!({
        "acct": "source:cursor", "org": "source", "kind": "source",
        "source": "cursor", "sourceName": "Cursor",
        "chats": list, "deleted": [], "connectors": {},
        "isCurrent": false, "label": "", "profile": Value::Null
    }))
}

/// A Cursor chat read straight out of Cursor, before anything is written.
pub fn cursor_detail(cid: &str) -> Result<Value, String> {
    let chat = cursor_chats().into_iter()
        .find(|c| c["id"].as_str() == Some(cid))
        .ok_or("no such Cursor chat")?;
    let msgs: Vec<Value> = cursor_messages(cid).iter().map(|m| json!({
        "role": m["role"], "t": m["t"].as_str().unwrap_or("").chars().take(16).collect::<String>(),
        "text": m["text"], "tools": []
    })).collect();
    let bytes: u64 = msgs.iter().map(|m| m["text"].as_str().unwrap_or("").len() as u64).sum();
    let rec = json!({
        "sessionId": Value::Null, "cliSessionId": cid,
        "title": chat["title"], "cwd": chat["folder"], "model": "",
        "createdAt": chat["created"], "lastActivityAt": chat["last"],
        "completedTurns": msgs.len(), "isArchived": chat["archived"],
        "source": "cursor", "sourceName": "Cursor"
    });
    Ok(json!({ "rec": rec, "files": [], "source": "cursor", "sourceName": "Cursor",
               "bytes": bytes, "subs": 0, "msgs": msgs }))
}

}   // mod cursor

#[cfg(target_os = "windows")]
use cursor::{cursor_chats, cursor_detail, cursor_scope, cursor_write_transcript};

/* Everywhere else there is no Cursor to read, and the rest of Ferry carries on
   as though it had never been asked. */
#[cfg(not(target_os = "windows"))]
fn cursor_scope() -> Option<Value> { None }
#[cfg(not(target_os = "windows"))]
fn cursor_chats() -> Vec<Value> { vec![] }
#[cfg(not(target_os = "windows"))]
fn cursor_detail(_cid: &str) -> Result<Value, String> {
    Err("Cursor chats are only read on Windows".into())
}
#[cfg(not(target_os = "windows"))]
fn cursor_write_transcript(_chat: &Value) -> Result<String, String> {
    Err("Cursor chats are only read on Windows".into())
}

/// Every <account>/<org> scope under the sessions root, found by walking the
/// directory rather than by pattern matching. Returns (account, org, dir).
fn scopes_on_disk() -> Vec<(String, String, PathBuf)> {
    let mut out = vec![];
    let root = PathBuf::from(sess());
    let Ok(l1) = fs::read_dir(&root) else { return out };
    for a in l1.flatten() {
        if !a.path().is_dir() { continue; }
        let acct = a.file_name().to_string_lossy().to_string();
        let Ok(l2) = fs::read_dir(a.path()) else { continue };
        for o in l2.flatten() {
            if !o.path().is_dir() { continue; }
            out.push((acct.clone(), o.file_name().to_string_lossy().to_string(), o.path()));
        }
    }
    out
}

/// What the scan actually looked at. Surfaced in the UI so an empty list can
/// explain itself instead of just showing nothing.
fn probe(path: &str) -> Value {
    // is_dir() collapses every failure into `false`, which hides the difference
    // between "not there" and "not allowed". Report the actual error instead.
    match fs::metadata(path) {
        Ok(m) => json!({ "ok": true, "isDir": m.is_dir() }),
        Err(e) => json!({ "ok": false, "error": format!("{:?}", e.kind()), "detail": e.to_string() }),
    }
}

fn list_dir(path: &str, take: usize) -> Value {
    match fs::read_dir(path) {
        Ok(rd) => {
            let names: Vec<String> = rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string()).collect();
            json!({ "ok": true, "count": names.len(),
                    "names": names.iter().take(take).cloned().collect::<Vec<_>>() })
        }
        Err(e) => json!({ "ok": false, "error": format!("{:?}", e.kind()), "detail": e.to_string() }),
    }
}

fn diagnostics() -> Value {
    let scopes = scopes_on_disk();
    let mut records = 0usize;
    for (_, _, dir) in &scopes {
        if let Ok(rd) = fs::read_dir(dir) {
            records += rd.flatten().filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("local_") && n.ends_with(".json")
            }).count();
        }
    }
    json!({
        "sessionsRoot": sess(),
        "sessionsProbe": probe(&sess()),
        "sessionsList": list_dir(&sess(), 8),
        "claudeDir": claude_dir(),
        "claudeDirProbe": probe(&claude_dir()),
        "appdataList": list_dir(&std::env::var("APPDATA")
                        .unwrap_or_else(|_| format!("{}/AppData/Roaming", home())), 40),
        "scopesFound": scopes.len(),
        "chatRecordsFound": records,
        "projectsRoot": proj(),
        "projectsProbe": probe(&proj()),
        "home": home(),
        "appdata": std::env::var("APPDATA").unwrap_or_else(|_| "(unset)".into()),
        "userprofile": std::env::var("USERPROFILE").unwrap_or_else(|_| "(unset)".into()),
    })
}

// Tauri runs a plain `fn` command on the main thread, so on Windows a slow
// disk scan froze the whole window. There the commands below use
// `command(async)`. Tried on macOS too and the window became unstable
// (it vanished after ~10 s), so macOS keeps the plain attribute.
#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn scan() -> Value {
    let cur = read_json(&cfg()).and_then(|c| c["lastKnownAccountUuid"].as_str().map(String::from));
    let labs = labels();
    let profs = profiles();
    let mut scopes: std::collections::BTreeMap<String, Value> = Default::default();
    // every transcript some account already answers for; the rest are sources
    let mut claimed: std::collections::HashSet<String> = Default::default();

    let touch = |scopes: &mut std::collections::BTreeMap<String, Value>, acct: &str, org: &str| {
        scopes.entry(format!("{}|{}", acct, org)).or_insert_with(|| json!({
            "acct": acct, "org": org, "chats": [], "deleted": [],
            "connectors": {}, "isCurrent": cur.as_deref() == Some(acct),
            "label": labs[acct].as_str().unwrap_or(""),
            "profile": if profs[acct].is_null() { Value::Null } else { profs[acct].clone() }
        }));
    };

    for (acct, org, dir) in scopes_on_disk() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if !(name.starts_with("local_") && name.ends_with(".json")) { continue; }
            let (acct, org) = (acct.clone(), org.clone());
            let Some(rec) = read_json(&p.to_string_lossy()) else { continue };
            touch(&mut scopes, &acct, &org);
            let key = format!("{}|{}", acct, org);
            let tr = transcripts_for(&rec);
            for t in &tr { if let Some(i) = t["id"].as_str() { claimed.insert(i.to_string()); } }
            let bytes: u64 = tr.iter().map(|t| t["size"].as_u64().unwrap_or(0) + t["subBytes"].as_u64().unwrap_or(0)).sum();
            // Only warn when the chat's OWN transcript is gone. bridgeSessionIds can
            // name sessions that never had a transcript file of their own, and those
            // must not make a perfectly readable chat look pruned.
            let missing = match tr.first() {
                Some(t) if !t["exists"].as_bool().unwrap_or(false) => 1,
                _ => 0,
            };
            let absent = tr.iter().filter(|t| !t["exists"].as_bool().unwrap_or(false)).count();
            let subs: u64 = tr.iter().map(|t| t["subagents"].as_u64().unwrap_or(0)).sum();
            let entry = json!({
                "id": rec["sessionId"], "title": rec["title"].as_str().unwrap_or("(untitled)"),
                "cwd": rec["cwd"], "model": rec["model"],
                "created": rec["createdAt"], "last": rec["lastActivityAt"],
                "turns": rec["completedTurns"], "archived": rec["isArchived"],
                "forkedFrom": rec["forkedFromSessionId"],
                "files": tr.len(), "subs": subs, "bytes": bytes,
                "missing": missing, "absent": absent,
                "path": p.to_string_lossy()
            });
            scopes.get_mut(&key).unwrap()["chats"].as_array_mut().unwrap().push(entry);
            if let Some(ms) = rec["remoteMcpServersConfig"].as_array() {
                for m in ms {
                    if let Some(nm) = m["name"].as_str() {
                        let c = &mut scopes.get_mut(&key).unwrap()["connectors"];
                        let v = c[nm].as_u64().unwrap_or(0) + 1;
                        c[nm] = json!(v);
                    }
                }
            }
        }
    }
    for (acct, org, dir) in scopes_on_disk() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let base = e.file_name().to_string_lossy().to_string();
            if !base.starts_with("deleted_") { continue; }
            let (acct, org) = (acct.clone(), org.clone());
            touch(&mut scopes, &acct, &org);
            let short = base.trim_start_matches("deleted_").to_string();
            let when: Option<u64> = fs::read_to_string(&p).ok().and_then(|s| s.trim().parse().ok());
            let key = format!("{}|{}", acct, org);
            scopes.get_mut(&key).unwrap()["deleted"].as_array_mut().unwrap()
                .push(json!({ "id": format!("local_{}", short), "when": when,
                              "path": p.to_string_lossy() }));
        }
    }
    let mut list: Vec<Value> = scopes.into_values().collect();
    for s in list.iter_mut() {
        let c = s["chats"].as_array_mut().unwrap();
        c.sort_by_key(|x| std::cmp::Reverse(x["last"].as_u64().unwrap_or(0)));
        let d = s["deleted"].as_array_mut().unwrap();
        d.sort_by_key(|x| std::cmp::Reverse(x["when"].as_u64().unwrap_or(0)));
    }
    list.sort_by_key(|s| std::cmp::Reverse(
        s["chats"].as_array().unwrap().iter().map(|c| c["last"].as_u64().unwrap_or(0)).max().unwrap_or(0)));
    // CLI and VS Code sessions come after the accounts: they are where chats are
    // imported from, not an account you can send one to.
    list.extend(source_scopes(&claimed));
    if let Some(c) = cursor_scope() { list.push(c); }
    json!({ "scopes": list, "current": cur, "appRunning": app_running(),
            "vault": vault(), "exportDir": last_export_dir(),
            "paths": { "sessions": sess(), "projects": proj(), "home": home() },
            "diag": diagnostics() })
}

/// Walk every transcript that belongs to one chat, oldest session first.
fn collect_msgs(tr: &[Value], cap: usize, per_msg: usize) -> Vec<Value> {
    let mut ordered: Vec<&Value> = tr.iter().skip(1).collect();
    ordered.reverse();
    if let Some(first) = tr.first() { ordered.push(first); }

    let mut msgs: Vec<Value> = vec![];
    for t in ordered {
        if !t["exists"].as_bool().unwrap_or(false) { continue; }
        let Ok(txt) = fs::read_to_string(t["path"].as_str().unwrap_or("")) else { continue };
        for line in txt.lines() {
            if msgs.len() >= cap { break; }
            let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
            let ty = d["type"].as_str().unwrap_or("");
            if ty != "user" && ty != "assistant" { continue; }
            let stamp = d["timestamp"].as_str().unwrap_or("").chars().take(16).collect::<String>();
            let c = &d["message"]["content"];
            let mut text = String::new();
            let mut tools: Vec<String> = vec![];
            if let Some(sx) = c.as_str() { text.push_str(sx); }
            else if let Some(arr) = c.as_array() {
                for part in arr {
                    match part["type"].as_str().unwrap_or("") {
                        "text" => { if let Some(x) = part["text"].as_str() {
                            if !text.is_empty() { text.push_str("\n\n"); } text.push_str(x); } }
                        "tool_use" => {
                            let name = part["name"].as_str().unwrap_or("tool");
                            let hint = part["input"]["command"].as_str()
                                .or_else(|| part["input"]["file_path"].as_str())
                                .or_else(|| part["input"]["pattern"].as_str())
                                .or_else(|| part["input"]["description"].as_str())
                                .unwrap_or("");
                            tools.push(if hint.is_empty() { name.to_string() }
                                       else { format!("{} — {}", name, hint.chars().take(90).collect::<String>()) });
                        }
                        _ => {}
                    }
                }
            }
            let trimmed = text.trim();
            if trimmed.is_empty() && tools.is_empty() { continue; }
            if trimmed.starts_with("<command-name>") || trimmed.starts_with("<local-command")
               || trimmed.starts_with("<system-reminder>") { continue; }
            msgs.push(json!({ "role": ty, "t": stamp,
                              "text": trimmed.chars().take(per_msg).collect::<String>(),
                              "tools": tools }));
        }
    }
    msgs
}

/// A CLI or VS Code session read straight from its transcript. There is no
/// record to read, so one is described from the file itself - the same shape
/// the reader already knows, minus an account to have written it.
fn source_parts(path: &str) -> Result<(Value, Vec<Value>), String> {
    let info = read_session(path).ok_or("transcript unreadable")?;
    let p = Path::new(path);
    let id  = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let dir = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    let (subs, sub_bytes) = subagents_of(&dir, &id);
    let tr = vec![json!({ "id": id, "path": path, "exists": true,
                          "size": info["size"], "subagents": subs, "subBytes": sub_bytes })];
    let (kind, name) = source_of(info["entrypoint"].as_str().unwrap_or(""));
    let rec = json!({
        "sessionId": Value::Null, "cliSessionId": id,
        "title": info["title"], "cwd": info["cwd"], "model": info["model"],
        "createdAt": info["created"], "lastActivityAt": info["last"],
        "completedTurns": info["turns"], "isArchived": false,
        "gitBranch": info["branch"], "cliVersion": info["version"],
        "source": kind, "sourceName": name,
    });
    Ok((rec, tr))
}

/// The record and transcripts of a chat, whether an account owns it or it is
/// still only a session on disk.
fn chat_parts(path: &str) -> Result<(Value, Vec<Value>), String> {
    if let Some(cid) = path.strip_prefix("cursor:") {
        // nothing on disk to read yet: the conversation is still in Cursor
        let d = cursor_detail(cid)?;
        return Ok((d["rec"].clone(), vec![]));
    }
    if under(&proj(), path) && path.ends_with(".jsonl") { return source_parts(path); }
    if !under(&sess(), path) {
        return Err(format!("path is outside the sessions folder\n  path: {}\n  root: {}", path, sess()));
    }
    let rec = read_json(path).ok_or("chat unreadable")?;
    let tr = transcripts_for(&rec);
    Ok((rec, tr))
}

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn chat_detail(path: String) -> Result<Value, String> {
    if let Some(cid) = path.strip_prefix("cursor:") { return cursor_detail(cid); }
    let (rec, tr) = chat_parts(&path)?;
    let msgs = collect_msgs(&tr, 800, 24000);
    let bytes: u64 = tr.iter().map(|t| t["size"].as_u64().unwrap_or(0)).sum();
    let subs: u64 = tr.iter().map(|t| t["subagents"].as_u64().unwrap_or(0)).sum();
    Ok(json!({ "rec": rec, "files": tr, "bytes": bytes, "subs": subs, "msgs": msgs }))
}

fn slug(s: &str) -> String {
    let mut out: String = s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>().to_lowercase();
    while out.contains("--") { out = out.replace("--", "-"); }
    out.trim_matches('-').chars().take(60).collect()
}

/// Save the whole conversation to ~/Downloads as Markdown, JSON, or plain text.
#[tauri::command]
async fn export_chat(app: tauri::AppHandle, path: String, fmt: String, ask: bool) -> Result<Value, String> {
    let (rec, tr) = chat_parts(&path)?;
    let msgs = collect_msgs(&tr, 100_000, 2_000_000);
    if msgs.is_empty() && fmt != "json" {
        return Err("nothing to download - this chat's transcript was pruned".into());
    }
    let title = rec["title"].as_str().unwrap_or("chat").to_string();
    // a session no account owns has no record id yet: name the file after the
    // session it does have, so a CLI chat downloads under a stable name too
    let sid   = rec["sessionId"].as_str()
                   .or_else(|| rec["cliSessionId"].as_str()).unwrap_or("");
    let short: String = sid.trim_start_matches("local_").chars().take(8).collect();

    let (body, ext) = match fmt.as_str() {
        "json" => (serde_json::to_string_pretty(&json!({
                      "title": title, "sessionId": sid, "cwd": rec["cwd"],
                      "model": rec["model"], "createdAt": rec["createdAt"],
                      "lastActivityAt": rec["lastActivityAt"],
                      "transcripts": tr, "messages": msgs })).unwrap(), "json"),
        "txt" => {
            let mut o = format!("{}\n{}\n\n", title, "=".repeat(title.len()));
            for m in &msgs {
                o.push_str(&format!("[{}] {}\n{}\n\n",
                    m["t"].as_str().unwrap_or(""),
                    if m["role"]=="user" {"You"} else {"Claude"},
                    m["text"].as_str().unwrap_or("")));
            }
            (o, "txt")
        }
        _ => {
            let mut o = format!("# {}\n\n", title);
            o.push_str(&format!("- model: `{}`\n- folder: `{}`\n- turns: {}\n- transcripts: {}\n\n---\n\n",
                rec["model"].as_str().unwrap_or("?"),
                rec["cwd"].as_str().unwrap_or("?"),
                rec["completedTurns"].as_u64().unwrap_or(0),
                tr.len()));
            for m in &msgs {
                let who = if m["role"]=="user" {"You"} else {"Claude"};
                o.push_str(&format!("### {} · {}\n\n", who, m["t"].as_str().unwrap_or("")));
                let t = m["text"].as_str().unwrap_or("");
                if !t.is_empty() { o.push_str(t); o.push_str("\n\n"); }
                if let Some(tl) = m["tools"].as_array() {
                    for x in tl { o.push_str(&format!("> `{}`\n", x.as_str().unwrap_or(""))); }
                    if !tl.is_empty() { o.push('\n'); }
                }
            }
            (o, "md")
        }
    };

    let fname = format!("{}-{}.{}", slug(&title), short, ext);
    let start = last_export_dir();

    let target: PathBuf = if ask {
        use tauri_plugin_dialog::DialogExt;
        let label = match ext { "json" => "JSON", "txt" => "Plain text", _ => "Markdown" };
        // the panel must be driven by the main thread; this command is not on it,
        // so hand the choice back over a channel instead of blocking.
        let (tx, rx) = std::sync::mpsc::channel();
        app.dialog().file()
            .set_directory(&start)
            .set_file_name(&fname)
            .add_filter(label, &[ext])
            .save_file(move |p| { let _ = tx.send(p); });
        match rx.recv().map_err(|e| e.to_string())? {
            Some(fp) => fp.into_path().map_err(|e| e.to_string())?,
            None => return Ok(json!({ "ok": false, "cancelled": true })),
        }
    } else {
        fs::create_dir_all(&start).map_err(|e| e.to_string())?;
        PathBuf::from(format!("{}/{}", start, fname))
    };

    if let Some(par) = target.parent() {
        fs::create_dir_all(par).map_err(|e| e.to_string())?;
        set_last_export_dir(&par.to_string_lossy());
    }
    fs::write(&target, body).map_err(|e| e.to_string())?;
    let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    Ok(json!({ "ok": true, "file": target.to_string_lossy(),
               "dir": target.parent().map(|p| p.to_string_lossy().to_string()),
               "name": target.file_name().map(|p| p.to_string_lossy().to_string()),
               "messages": msgs.len(), "kb": (size as f64/1024.0).round() as u64 }))
}

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn copy_chat(path: String, acct: String, org: String, mv: bool) -> Result<Value, String> {
    guard()?;
    if !under(&sess(), &path) {
        return Err(format!("path is outside the sessions folder\n  path: {}\n  root: {}", path, sess()));
    }
    let rec = read_json(&path).ok_or("source unreadable")?;
    let sid = rec["sessionId"].as_str().ok_or("no sessionId")?.to_string();
    let short = sid.trim_start_matches("local_").to_string();
    let dst_dir = format!("{}/{}/{}", sess(), acct, org);
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let dst = format!("{}/{}.json", dst_dir, sid);

    let mut out = rec.clone();
    if let Some(o) = out.as_object_mut() { for k in ACCOUNT_SCOPED { o.remove(k); } }
    snapshot(&dst, "overwrite");
    fs::write(&dst, serde_json::to_string_pretty(&out).unwrap()).map_err(|e| e.to_string())?;

    let tomb = format!("{}/deleted_{}", dst_dir, short);
    if Path::new(&tomb).exists() { snapshot(&tomb, "undelete"); let _ = fs::remove_file(&tomb); }

    if mv {
        snapshot(&path, "moved-out");
        let src_dir = Path::new(&path).parent().unwrap().to_string_lossy().to_string();
        let _ = fs::remove_file(&path);
        let _ = fs::write(format!("{}/deleted_{}", src_dir, short), now_ms().to_string());
    }
    Ok(json!({ "ok": true, "wrote": dst, "moved": mv }))
}

/// A chat started without picking a folder runs in a workspace the app makes
/// for it, and shows as having no folder at all.
fn is_scratch(cwd: &str) -> bool {
    cwd.is_empty() || cwd.to_lowercase().contains("scratch-workspaces")
}

/// Put one transcript under a second name, so it can be found from another
/// folder as well. A hard link, not a copy: one file, two names, not a byte
/// duplicated, and the folder it came from keeps working. Only a volume that
/// refuses links falls back to copying. Returns (linked, copied).
fn relink(src: &str, dst: &str) -> (u64, u64) {
    if Path::new(dst).exists() || !Path::new(src).exists() { return (0, 0); }
    if let Some(par) = Path::new(dst).parent() { let _ = fs::create_dir_all(par); }
    if fs::hard_link(src, dst).is_ok() { return (1, 0); }
    if fs::copy(src, dst).is_ok() { return (0, 1); }
    (0, 0)
}

/// Point a chat at a folder.
///
/// A chat's cwd is two things at once: the folder Claude names in its header
/// and resumes in, and where the conversation is looked up - the transcript
/// lives under ~/.claude/projects/<encoded cwd>/. So changing only the cwd
/// would show the new folder and lose the conversation with it; every
/// transcript the chat is made of has to be findable under the new name too.
#[tauri::command]
async fn set_folder(app: tauri::AppHandle, path: String, folder: Option<String>) -> Result<Value, String> {
    guard()?;
    if !under(&sess(), &path) {
        return Err(format!("path is outside the sessions folder\n  path: {}\n  root: {}", path, sess()));
    }
    let mut rec = read_json(&path).ok_or("chat unreadable")?;
    let was = rec["cwd"].as_str().unwrap_or("").to_string();

    let dir = match folder.filter(|f| !f.trim().is_empty()) {
        Some(f) => f,
        None => {
            use tauri_plugin_dialog::DialogExt;
            // open where the chat already is, or - for a chat that never had a
            // folder - the last real one its window remembers
            let start = [was.as_str(), rec["scratchPromptRecents"].as_str().unwrap_or(""), &home()]
                .into_iter().find(|d| !is_scratch(d) && Path::new(d).is_dir())
                .unwrap_or("").to_string();
            // the panel must be driven by the main thread; this command is not
            // on it, so hand the choice back over a channel instead of blocking
            let (tx, rx) = std::sync::mpsc::channel();
            app.dialog().file().set_directory(&start)
                .pick_folder(move |p| { let _ = tx.send(p); });
            match rx.recv().map_err(|e| e.to_string())? {
                Some(fp) => fp.into_path().map_err(|e| e.to_string())?.to_string_lossy().to_string(),
                None => return Ok(json!({ "ok": false, "cancelled": true })),
            }
        }
    };
    if !Path::new(&dir).is_dir() { return Err(format!("no such folder: {}", dir)); }
    if dir == was { return Ok(json!({ "ok": true, "unchanged": true, "cwd": dir })); }

    let dst_dir = project_dir(&dir);
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let (mut linked, mut copied) = (0u64, 0u64);
    for t in transcripts_for(&rec) {
        let (Some(src), Some(id)) = (t["path"].as_str(), t["id"].as_str()) else { continue };
        let (l, c) = relink(src, &format!("{}/{}.jsonl", dst_dir, id));
        linked += l; copied += c;
        let src_dir = Path::new(src).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        if let Ok(g) = glob::glob(&format!("{}/{}/subagents/*.jsonl", src_dir, id)) {
            for s in g.flatten() {
                let name = s.file_name().unwrap_or_default().to_string_lossy().to_string();
                let (l, c) = relink(&s.to_string_lossy(),
                                    &format!("{}/{}/subagents/{}", dst_dir, id, name));
                linked += l; copied += c;
            }
        }
    }
    snapshot(&path, "folder");
    rec["cwd"] = json!(dir);
    rec["originCwd"] = json!(dir);
    fs::write(&path, serde_json::to_string_pretty(&rec).unwrap()).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "cwd": dir, "was": was, "linked": linked, "copied": copied }))
}

/// Fields that describe the account and its environment rather than the chat.
/// An imported record takes them from a record the app itself wrote for that
/// account, so it never invents a value that does not resolve there.
const INHERIT: [&str; 6] = ["envScopeId", "permissionMode", "effort",
                            "chromePermissionMode", "remoteControlAutoEligible",
                            "classifierSummaryEnabled"];

/// The newest record the app wrote in this account, to copy those fields from.
fn template_record(dir: &str) -> Option<Value> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for e in fs::read_dir(dir).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if !(n.starts_with("local_") && n.ends_with(".json")) { continue; }
        let Ok(m) = e.metadata().and_then(|m| m.modified()) else { continue };
        if best.as_ref().map(|(t, _)| m > *t).unwrap_or(true) { best = Some((m, e.path())); }
    }
    read_json(&best?.1.to_string_lossy())
}

/// Give a CLI or VS Code session the per-account record it never had, so an
/// account claims it and it becomes an ordinary chat: readable in Claude,
/// and from here on copyable, movable and deletable like any other.
/// The transcript is not touched - the session stays resumable from the CLI.
#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn import_session(path: String, acct: String, org: String) -> Result<Value, String> {
    guard()?;
    // A Cursor chat has no transcript to point at - its conversation lives in
    // Cursor's database - so one is written first, and from there it is an
    // ordinary import. This is the only place Ferry writes a transcript.
    let path = match path.strip_prefix("cursor:") {
        None => path,
        Some(cid) => {
            let chat = cursor_chats().into_iter()
                .find(|c| c["id"].as_str() == Some(cid)).ok_or("no such Cursor chat")?;
            let p = cursor_write_transcript(&chat)?;
            let out = import_session_at(&p, &acct, &org)?;
            // Cursor already named the conversation; keep its name over the
            // first line of the first prompt
            if let Some(t) = chat["title"].as_str() {
                if t != "(unnamed)" {
                    if let Some(w) = out["wrote"].as_str() {
                        if let Some(mut r) = read_json(w) {
                            r["title"] = json!(t);
                            let _ = fs::write(w, serde_json::to_string_pretty(&r).unwrap());
                        }
                    }
                }
            }
            return Ok(out);
        }
    };
    import_session_at(&path, &acct, &org)
}

/// The import itself, once there is a transcript to import. Kept apart from the
/// command so a Cursor chat, whose transcript has only just been written, takes
/// the very same path as one that was always there.
fn import_session_at(path: &str, acct: &str, org: &str) -> Result<Value, String> {
    if !(under(&proj(), path) && path.ends_with(".jsonl")) {
        return Err(format!("path is outside the projects folder\n  path: {}\n  root: {}", path, proj()));
    }
    let info = read_session(path).ok_or("transcript unreadable")?;
    let cwd = info["cwd"].as_str().unwrap_or("");
    if cwd.is_empty() { return Err("this transcript does not say which folder it ran in".into()); }
    let sid = Path::new(path).file_stem().ok_or("no session id")?.to_string_lossy().to_string();
    let dst_dir = format!("{}/{}/{}", sess(), acct, org);
    if !Path::new(&dst_dir).is_dir() { return Err("that account has no folder on this machine".into()); }

    // The id the chat keeps for good, derived from the session it already has:
    // importing the same session twice updates one record instead of making a
    // second, and a later copy to another account carries the same id, exactly
    // as a copy between two accounts does.
    let id = format!("local_{}", sid);
    let mut rec = json!({
        "sessionId": id, "cliSessionId": sid,
        "cwd": cwd, "originCwd": cwd,
        "title": info["title"], "titleSource": "auto",
        "createdAt": info["created"], "lastActivityAt": info["last"],
        "lastFocusedAt": info["last"], "completedTurns": info["turns"],
        "isArchived": false,
    });
    if let Some(m) = info["model"].as_str() { if !m.is_empty() { rec["model"] = json!(m); } }
    if let Some(t) = template_record(&dst_dir) {
        for k in INHERIT { if !t[k].is_null() { rec[k] = t[k].clone(); } }
    }

    let dst = format!("{}/{}.json", dst_dir, id);
    snapshot(&dst, "import");
    fs::write(&dst, serde_json::to_string_pretty(&rec).unwrap()).map_err(|e| e.to_string())?;
    let tomb = format!("{}/deleted_{}", dst_dir, sid);
    if Path::new(&tomb).exists() { snapshot(&tomb, "undelete"); let _ = fs::remove_file(&tomb); }

    Ok(json!({ "ok": true, "wrote": dst, "id": id,
               "title": info["title"], "turns": info["turns"] }))
}

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn rename_chat(path: String, title: String) -> Result<Value, String> {
    guard()?;
    if !under(&sess(), &path) {
        return Err(format!("path is outside the sessions folder\n  path: {}\n  root: {}", path, sess()));
    }
    let mut rec = read_json(&path).ok_or("chat unreadable")?;
    snapshot(&path, "rename");
    let t = if title.trim().is_empty() { "(untitled)".to_string() } else { title.trim().to_string() };
    rec["title"] = json!(t);
    fs::write(&path, serde_json::to_string_pretty(&rec).unwrap()).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "title": t }))
}

#[tauri::command]
async fn delete_chat(app: tauri::AppHandle, path: String) -> Result<Value, String> {
    guard()?;
    if !under(&sess(), &path) {
        return Err(format!("path is outside the sessions folder\n  path: {}\n  root: {}", path, sess()));
    }
    let rec = read_json(&path).ok_or("chat unreadable")?;

    {
        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
        let title = rec["title"].as_str().unwrap_or("this chat");
        let (tx, rx) = std::sync::mpsc::channel();
        app.dialog()
            .message(format!(
                "Remove \u{201c}{}\u{201d} from this account?\n\n                 The conversation itself stays on disk and in your archive,                  so you can restore it later.", title))
            .title("Delete chat")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom("Delete".into(), "Cancel".into()))
            .show(move |ok| { let _ = tx.send(ok); });
        if !rx.recv().map_err(|e| e.to_string())? {
            return Ok(json!({ "ok": false, "cancelled": true }));
        }
    }

    let sid = rec["sessionId"].as_str().ok_or("no sessionId")?;
    let short = sid.trim_start_matches("local_");
    snapshot(&path, "delete");
    let dir = Path::new(&path).parent().unwrap().to_string_lossy().to_string();
    let _ = fs::write(format!("{}/deleted_{}", dir, short), now_ms().to_string());
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn undelete_chat(acct: String, org: String, id: String) -> Result<Value, String> {
    guard()?;
    let short = id.trim_start_matches("local_").to_string();
    let mut src: Option<PathBuf> = None;
    if let Ok(p) = glob::glob(&format!("{}/*/*/{}.json", sess(), id)) {
        for c in p.flatten() { src = Some(c); break; }
    }
    if src.is_none() {
        if let Ok(p) = glob::glob(&format!("{}/chats/*/*/{}.json", vault(), id)) {
            let mut all: Vec<PathBuf> = p.flatten().collect();
            all.sort(); src = all.pop();
        }
    }
    let src = src.ok_or("no surviving copy in any account or the vault")?;
    let mut rec = read_json(&src.to_string_lossy()).ok_or("copy unreadable")?;
    if let Some(o) = rec.as_object_mut() { for k in ACCOUNT_SCOPED { o.remove(k); } }
    let dir = format!("{}/{}/{}", sess(), acct, org);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(format!("{}/{}.json", dir, id), serde_json::to_string_pretty(&rec).unwrap())
        .map_err(|e| e.to_string())?;
    let tomb = format!("{}/deleted_{}", dir, short);
    if Path::new(&tomb).exists() { snapshot(&tomb, "undelete"); let _ = fs::remove_file(&tomb); }
    Ok(json!({ "ok": true, "from": src.to_string_lossy() }))
}

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn set_label(acct: String, name: String) -> Result<Value, String> {
    let _ = fs::create_dir_all(vault());
    let mut l = labels();
    l[acct] = json!(name.trim());
    fs::write(labels_path(), serde_json::to_string_pretty(&l).unwrap()).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn run_vault() -> Result<Value, String> {
    let base = format!("{}/chats/{}", vault(), stamp());
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let (mut recs, mut files, mut bytes) = (0u64, 0u64, 0u64);

    if let Ok(paths) = glob::glob(&format!("{}/*/*/local_*.json", sess())) {
        for p in paths.flatten() {
            let parts: Vec<String> = p.iter().map(|s| s.to_string_lossy().to_string()).collect();
            let acct = parts[parts.len()-3].clone();
            let d = format!("{}/{}", base, acct);
            let _ = fs::create_dir_all(&d);
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if fs::copy(&p, format!("{}/{}", d, name)).is_ok() { recs += 1; }
        }
    }
    // sweep the whole projects tree so orphans and subagent transcripts survive too
    for e in walkdir::WalkDir::new(proj()).into_iter().filter_map(|x| x.ok()) {
        if !e.file_type().is_file() { continue; }
        if e.path().extension().map(|x| x != "jsonl").unwrap_or(true) { continue; }
        let Ok(rel) = e.path().strip_prefix(proj()) else { continue };
        let dst = PathBuf::from(format!("{}/projects/{}", vault(), rel.to_string_lossy()));
        let src_m = e.metadata().ok().and_then(|m| m.modified().ok());
        let dst_m = fs::metadata(&dst).ok().and_then(|m| m.modified().ok());
        if let (Some(s), Some(d)) = (src_m, dst_m) { if d >= s { continue; } }
        if let Some(par) = dst.parent() { let _ = fs::create_dir_all(par); }
        if fs::copy(e.path(), &dst).is_ok() {
            files += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(json!({ "ok": true, "records": recs, "transcripts": files,
               "mb": (bytes as f64 / 1048576.0 * 10.0).round() / 10.0, "dir": base }))
}

/// Page zoom for Ctrl/Cmd + and -, Ctrl/Cmd 0 and the zoom control. The UI
/// picks and remembers the level; this applies it to the webview itself, the
/// same zoom a browser does, so layout, hit-testing and drag and drop stay right.
#[tauri::command]
fn set_zoom(webview_window: tauri::WebviewWindow, scale: f64) -> Result<(), String> {
    webview_window.set_zoom(scale.clamp(0.5, 2.0)).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            scan, chat_detail, export_chat, copy_chat, import_session, set_folder,
            rename_chat, delete_chat, undelete_chat, set_label, run_vault, set_zoom
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch Ferry");
}
