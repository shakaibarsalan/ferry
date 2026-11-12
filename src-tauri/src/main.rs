#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! ccvault - manage Claude Code chats across the local accounts on this Mac.

use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

mod share;

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
/// finds %APPDATA%\Claude empty. Newer builds have left both for
/// %LOCALAPPDATA%\Claude-Data. Look in all of them; if more than one has
/// chats, take the one written to most recently.
#[cfg(target_os = "windows")]
fn claude_dir() -> String {
    let cands = claude_dirs();
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

fn sess()  -> String { sess_in(&claude_dir()) }
fn cfg()   -> String { cfg_in(&claude_dir()) }
fn proj()  -> String { format!("{}/.claude/projects", home()) }
fn vault() -> String { format!("{}/.ferry", home()) }

/* ---------- the folders accounts are read from ----------

   Claude keeps its state in one folder per *profile*, not one per machine:
   `--user-data-dir` gives a second window its own, and that is how a second
   account is kept signed in alongside the first. Ferry used to read a single
   folder - the built-in one with the newest chats - so every account in any
   other folder was invisible, the same way the Store build's chats were
   invisible before Ferry looked inside the MSIX container.

   So a root is any folder Ferry reads accounts from: the built-in ones, plus
   the folders someone pointed Ferry at, remembered in ~/.ferry/locations.json.
   The list of extra folders starts empty, so a machine nobody has added one on
   behaves exactly as before.                                                  */

fn sess_in(root: &str) -> String { format!("{}/claude-code-sessions", root) }
fn cfg_in(root: &str)  -> String { format!("{}/config.json", root) }
fn idb_in(root: &str)  -> String { format!("{}/IndexedDB", root) }
fn locations_path()    -> String { format!("{}/locations.json", vault()) }

#[derive(Clone)]
struct Root {
    /// Short, stable, and derived from the path, so the UI can name a root
    /// without passing a whole path back and forth.
    id: String,
    path: String,
    /// What to show beside an account when more than one root has it. Empty for
    /// the folder Claude is using now, which needs no qualifying.
    label: String,
    builtin: bool,
}

/// The same folder said two ways. Windows mixes "/" and "\" depending on
/// whether a path came from a format! or from the folder panel, and ignores
/// case; a trailing separator means nothing anywhere.
fn norm_dir(p: &str) -> String {
    let s = p.trim().trim_end_matches(['/', '\\']).replace('\\', "/");
    if cfg!(windows) { s.to_ascii_lowercase() } else { s }
}
fn same_dir(a: &str, b: &str) -> bool { norm_dir(a) == norm_dir(b) }

fn root_id(path: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(norm_dir(path).as_bytes()).iter().take(4)
        .map(|b| format!("{b:02x}")).collect()
}

fn leaf(p: &str) -> String {
    Path::new(p.trim_end_matches(['/', '\\'])).file_name()
        .map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

/// Where the desktop app puts a profile without being told otherwise.
#[cfg(target_os = "windows")]
fn builtin_roots() -> Vec<String> { claude_dirs() }
#[cfg(not(target_os = "windows"))]
fn builtin_roots() -> Vec<String> { vec![claude_dir()] }

/// A built-in root that is not the one in use still has to be told apart from
/// the one that is: on Windows a direct install and the Store build can both
/// hold chats, and until now only the newer of the two was read at all.
fn builtin_label(p: &str) -> String {
    if cfg!(target_os = "windows") {
        if norm_dir(p).contains("/claude-data/") { return "Claude-Data".into(); }
        if norm_dir(p).contains("/packages/claude_") { return "Microsoft Store".into(); }
        return "AppData".into();
    }
    leaf(p)
}

fn stored_locations() -> Vec<Value> {
    read_json(&locations_path()).and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// Every folder to read accounts from, built-in ones first. Deduped: the same
/// folder listed twice would list every chat in it twice.
fn roots() -> Vec<Root> {
    merge_roots(builtin_roots(), stored_locations(), &claude_dir())
}

/// Kept apart from the machine so the merging can be tested: which folders win,
/// how they are named, and that nothing appears twice.
fn merge_roots(builtin: Vec<String>, stored: Vec<Value>, primary: &str) -> Vec<Root> {
    let mut out: Vec<Root> = vec![];
    let mut seen: Vec<String> = vec![];
    let add = |out: &mut Vec<Root>, seen: &mut Vec<String>,
               path: String, label: String, builtin: bool| {
        let n = norm_dir(&path);
        if n.is_empty() || seen.contains(&n) { return; }
        seen.push(n);
        out.push(Root { id: root_id(&path), path, label, builtin });
    };
    for p in builtin {
        let label = if same_dir(&p, primary) { String::new() } else { builtin_label(&p) };
        add(&mut out, &mut seen, p, label, true);
    }
    for l in stored {
        let Some(p) = l["path"].as_str() else { continue };
        let label = l["label"].as_str().map(str::trim).filter(|s| !s.is_empty())
            .map(String::from).unwrap_or_else(|| leaf(p));
        add(&mut out, &mut seen, p.to_string(), label, false);
    }
    out
}

fn root_by_id(id: &str) -> Option<Root> { roots().into_iter().find(|r| r.id == id) }

/// The folder Claude itself is using. Where a chat goes when nothing says
/// otherwise, and the one root that is always in the list.
fn primary_root() -> Root {
    let p = claude_dir();
    roots().into_iter().find(|r| same_dir(&r.path, &p))
        .unwrap_or(Root { id: root_id(&p), path: p, label: String::new(), builtin: true })
}

/// Which root a chat record belongs to. Every write is checked this way, so a
/// path from outside every folder we read cannot be written through.
fn root_of(path: &str) -> Result<Root, String> {
    let all = roots();
    all.iter().find(|r| under(&sess_in(&r.path), path)).cloned().ok_or_else(|| {
        let list = all.iter().map(|r| sess_in(&r.path)).collect::<Vec<_>>().join("\n         ");
        format!("path is outside every sessions folder\n  path: {}\n  roots: {}", path, list)
    })
}

#[cfg(test)]
mod roots_tests {
    use serde_json::json;

    fn paths(rs: &[super::Root]) -> Vec<String> { rs.iter().map(|r| r.path.clone()).collect() }

    #[test]
    fn the_folder_claude_is_using_needs_no_label_and_the_others_do() {
        let primary = if cfg!(windows) { "C:\\Users\\a\\AppData\\Roaming\\Claude" }
                      else { "/Users/a/Library/Application Support/Claude" };
        let rs = super::merge_roots(vec![primary.to_string()], vec![], primary);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].label, "", "the profile in use is not qualified");
        assert!(rs[0].builtin);

        // an added folder is named after itself, so two of the same account can
        // be told apart on screen
        let added = if cfg!(windows) { "C:\\ClaudeAccounts\\Account2" } else { "/tmp/Account2" };
        let rs = super::merge_roots(vec![primary.to_string()],
                                    vec![json!({ "path": added })], primary);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[1].label, "Account2", "a folder with no label of its own is named after itself");
        assert!(!rs[1].builtin);
        assert_eq!(rs[1].label, super::leaf(added));

        // and a label someone chose wins over the folder name
        let rs = super::merge_roots(vec![primary.to_string()],
                                    vec![json!({ "path": added, "label": " Work " })], primary);
        assert_eq!(rs[1].label, "Work", "a chosen label is used, trimmed");
    }

    #[test]
    fn the_same_folder_is_never_read_twice() {
        let primary = if cfg!(windows) { "C:\\Claude" } else { "/Claude" };
        // said with a trailing separator, and - on Windows - in another case
        let again = if cfg!(windows) { "c:/claude\\" } else { "/Claude/" };
        let rs = super::merge_roots(vec![primary.to_string()],
                                    vec![json!({ "path": again })], primary);
        assert_eq!(paths(&rs), vec![primary.to_string()],
                   "a stored folder that is already built in adds nothing");

        let a = if cfg!(windows) { "C:\\one" } else { "/one" };
        let rs = super::merge_roots(vec![primary.to_string(), primary.to_string()],
                                    vec![json!({ "path": a }), json!({ "path": a })], primary);
        assert_eq!(paths(&rs), vec![primary.to_string(), a.to_string()],
                   "nor does the same folder listed twice");
    }

    #[test]
    fn a_root_id_is_the_folder_said_any_way() {
        let a = if cfg!(windows) { "C:\\ClaudeAccounts\\Account2" } else { "/x/Account2" };
        let b = if cfg!(windows) { "C:/ClaudeAccounts/Account2\\" } else { "/x/Account2/" };
        assert_eq!(super::root_id(a), super::root_id(b), "{a} and {b} are one folder");
        assert_ne!(super::root_id(a), super::root_id(&format!("{a}3")));
        assert_eq!(super::root_id(a).len(), 8, "short enough to pass around");
        if cfg!(windows) {
            assert_eq!(super::root_id(a), super::root_id(&a.to_ascii_uppercase()),
                       "Windows does not care about case");
        }
    }

    #[test]
    fn a_folder_has_to_be_a_claude_profile_and_must_not_overlap_one_we_read() {
        let base = std::env::temp_dir().join(format!("ferry-loc-{}", std::process::id()));
        let p = base.to_string_lossy().to_string();
        let _ = std::fs::remove_dir_all(&base);

        assert!(super::checked_location(&p).is_err(), "a folder that is not there");
        assert!(super::checked_location("  ").is_err(), "nothing at all");
        std::fs::create_dir_all(&base).unwrap();
        let err = super::checked_location(&p).unwrap_err();
        assert!(err.contains("--user-data-dir"), "says what to point at instead: {err}");

        // either half of a profile is enough to recognise one
        std::fs::create_dir_all(base.join("claude-code-sessions")).unwrap();
        assert_eq!(super::checked_location(&format!("{p}\\")).unwrap(), p,
                   "and the answer comes back without its trailing separator");
        std::fs::remove_dir_all(base.join("claude-code-sessions")).unwrap();
        std::fs::write(base.join("config.json"), "{}").unwrap();
        assert!(super::checked_location(&p).is_ok(), "a config.json on its own also says profile");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folder_that_overlaps_one_we_read_is_refused() {
        let root = if cfg!(windows) { "C:\\ClaudeAccounts\\Account2" } else { "/x/Account2" };
        let have = vec![root.to_string()];
        assert!(super::clashing_root(&have, root).unwrap().contains("already reads"),
                "the very same folder");
        assert!(super::clashing_root(&have, &format!("{root}/claude-code-sessions"))
                    .unwrap().contains("overlaps"), "a folder inside it");
        let outside = if cfg!(windows) { "C:\\ClaudeAccounts" } else { "/x" };
        assert!(super::clashing_root(&have, outside).unwrap().contains("overlaps"),
                "and the folder that holds it");
        let sibling = if cfg!(windows) { "C:\\ClaudeAccounts\\Account3" } else { "/x/Account3" };
        assert!(super::clashing_root(&have, sibling).is_none(), "a sibling is fine");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_folder_claude_moved_to_is_built_in_and_read_first() {
        let base = std::env::temp_dir().join(format!("ferry-dirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let s = |p: &std::path::Path| p.to_string_lossy().to_string();
        let (appdata, local) = (base.join("Roaming"), base.join("Local"));
        let moved = local.join("Claude-Data").join("claude_abc");
        std::fs::create_dir_all(moved.join("claude-code-sessions")).unwrap();
        std::fs::create_dir_all(local.join("Claude-Data").join("not-a-profile")).unwrap();
        std::fs::create_dir_all(local.join("Packages").join("Claude_abc")).unwrap();

        assert_eq!(super::claude_dirs_in(&s(&appdata), &s(&local)), vec![
            s(&moved),
            format!("{}\\Claude", s(&appdata)),
            format!("{}\\LocalCache\\Roaming\\Claude", s(&local.join("Packages").join("Claude_abc"))),
        ], "Claude-Data first, and only a folder there that is a profile");
        assert_eq!(super::builtin_label(&s(&moved)), "Claude-Data");

        // a machine Claude never moved reads exactly what it always did
        std::fs::remove_dir_all(local.join("Claude-Data")).unwrap();
        assert_eq!(super::claude_dirs_in(&s(&appdata), &s(&local))[0],
                   format!("{}\\Claude", s(&appdata)));
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// A folder someone can add: it has to be a Claude profile, and it must not
/// overlap one we already read, or its chats would be listed twice.
fn checked_location(p: &str) -> Result<String, String> {
    let p = p.trim().trim_end_matches(['/', '\\']).to_string();
    if p.is_empty() { return Err("no folder given".into()); }
    if !Path::new(&p).is_dir() { return Err(format!("no such folder: {p}")); }
    let have: Vec<String> = roots().iter().map(|r| r.path.clone()).collect();
    if let Some(clash) = clashing_root(&have, &p) { return Err(clash); }
    if !is_profile(&p) {
        return Err("that folder is not a Claude profile: it has no claude-code-sessions \
                    folder and no config.json. Point Ferry at the folder Claude was \
                    started with, the one after --user-data-dir.".into());
    }
    Ok(p)
}

/// Either half of a profile is enough to recognise one.
fn is_profile(p: &str) -> bool {
    Path::new(&sess_in(p)).is_dir() || read_json(&cfg_in(p)).is_some()
}

/// A folder that is, holds, or sits inside one we already read. Any of the
/// three would list the same chats twice.
fn clashing_root(existing: &[String], p: &str) -> Option<String> {
    for r in existing {
        if same_dir(r, p) { return Some("Ferry already reads that folder".into()); }
        if under(r, p) || under(p, r) {
            return Some(format!("that folder overlaps one Ferry already reads:\n  {r}"));
        }
    }
    None
}

/// Connector uuids and tool grants belong to the account that created them.
const ACCOUNT_SCOPED: [&str; 2] = ["remoteMcpServersConfig", "enabledMcpTools"];

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}
/// Windows has no `date -r`, and spawning one would flash a console window. The
/// local wall clock comes straight from kernel32 instead: no dependency, and no
/// timezone arithmetic of our own to get wrong. kernel32 is already linked.
#[cfg(target_os = "windows")]
#[repr(C)]
struct WinSystemTime { year: u16, month: u16, dow: u16, day: u16,
                       hour: u16, min: u16, sec: u16, ms: u16 }
#[cfg(target_os = "windows")]
extern "system" { fn GetLocalTime(out: *mut WinSystemTime); }

/// yyyymmdd-hhmmss, in local time, without pulling in chrono.
///
/// The format is not cosmetic: the vault is shared with the CLI, which writes
/// `%Y%m%d-%H%M%S`, and `archived_copy_in` picks the copy to restore by
/// comparing these names as strings. An epoch count sorts below every date this
/// century, so a Windows app that stamped `1789039713` always lost to a
/// CLI-written `20260914-120000`, however much newer it was.
fn stamp() -> String {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: GetLocalTime only writes the struct it is handed, which is
        // the layout kernel32 documents and cannot fail.
        let mut t = WinSystemTime { year: 0, month: 0, dow: 0, day: 0,
                                    hour: 0, min: 0, sec: 0, ms: 0 };
        unsafe { GetLocalTime(&mut t) };
        return format!("{:04}{:02}{:02}-{:02}{:02}{:02}",
                       t.year, t.month, t.day, t.hour, t.min, t.sec);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let out = Command::new("date").args(["-r", &secs.to_string(), "+%Y%m%d-%H%M%S"]).output();
        out.ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
           .filter(|s| !s.is_empty()).unwrap_or_else(|| secs.to_string())
    }
}
/// Current Claude Code rule, read out of the shipped CLI:
/// every character that is not [a-zA-Z0-9] becomes '-', runs are NOT collapsed.
/// JavaScript strings are UTF-16, so a character outside the Basic
/// Multilingual Plane (an emoji, say) is two units and becomes "--".
fn enc_cwd(p: &str) -> String {
    p.encode_utf16().map(|u| if u < 128 && (u as u8).is_ascii_alphanumeric() { u as u8 as char } else { '-' }).collect()
}
/// Older builds only replaced the separators and left dots alone.
fn enc_cwd_legacy(p: &str) -> String {
    p.chars().map(|c| if c=='/' || c=='\\' { '-' } else { c }).collect()
}

/// Resolve a chat's working directory to its transcript folder.
/// Folders written by different Claude Code versions coexist on disk, and very
/// long paths are truncated with a "-<hash>" suffix, so try each in turn.
fn project_dir(cwd: &str) -> String { project_dir_in(&proj(), cwd) }

/// The folder name Claude Code gives a path, as its CLI writes it:
///   T=200; E(t){ e=t.replace(/[^a-zA-Z0-9]/g,"-");
///               return e.length<=T ? e : `${e.slice(0,T)}-${Math.abs(gz(t)).toString(36)}` }
///   gz(t){ e=0; for each UTF-16 unit: e=(e<<5)-e+unit|0; return e }
/// The hash is of the path itself, not of its encoding.
const CUT: usize = 200;
/// Claude Code names the folder after the NFC form of the path (its cwd is
/// stored normalized), so "é" typed as e + combining accent is the same folder.
fn nfc(path: &str) -> String { use unicode_normalization::UnicodeNormalization; path.nfc().collect() }
/// Ferry before 1.6.2 encoded per character rather than per UTF-16 unit, so an
/// emoji became one "-" rather than two. Folders it wrote that way are still
/// read; nothing new is named this way.
fn enc_cwd_chars(p: &str) -> String {
    p.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}
fn cc_folder(path: &str) -> String {
    let path = &nfc(path);
    let e = enc_cwd(path);
    if e.len() <= CUT { return e; }
    let mut h: i32 = 0;
    for u in path.encode_utf16() { h = h.wrapping_shl(5).wrapping_sub(h).wrapping_add(u as i32); }
    let mut n = (h as i64).unsigned_abs();
    let mut digits = vec![];
    loop { digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]); n /= 36; if n == 0 { break; } }
    digits.reverse();
    format!("{}-{}", &e[..CUT], String::from_utf8(digits).unwrap())
}

/// Claude Code's own name first. Then the uncut encoding and the older
/// dot-keeping one, which other versions (and Ferry before 1.6.2) wrote. For a
/// long path, a folder cut by another version shares the first 200 characters
/// with a different hash. Nothing looser: the rule used to accept any folder
/// whose name, minus its last "-word", started the path, so a chat in
/// .../GitHub/newapp was filed with .../GitHub/ferry.
fn project_dir_in(root: &str, cwd: &str) -> String {
    let named = cc_folder(cwd);
    let full = enc_cwd(&nfc(cwd));
    for cand in [&named, &full, &enc_cwd(cwd), &enc_cwd_legacy(cwd), &enc_cwd_chars(cwd)] {
        let d = format!("{}/{}", root, cand);
        if Path::new(&d).is_dir() { return d; }
    }
    // A long path cut by another Claude Code version: the same 200 characters,
    // a different hash. Two long paths can share those 200, so the folder only
    // counts if its transcripts ran in this path, as Claude Code checks.
    if full.len() > CUT {
        let mut names: Vec<String> = fs::read_dir(root).map(|rd| rd.flatten()
            .map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
        names.sort();
        for name in names {
            if let Some((prefix, hash)) = name.rsplit_once('-') {
                if prefix.len() == CUT && !hash.is_empty() && full.starts_with(prefix)
                   && ran_in(Path::new(&format!("{}/{}", root, name)), &nfc(cwd)) {
                    return format!("{}/{}", root, name);
                }
            }
        }
    }
    format!("{}/{}", root, named)
}

/// Whether a folder's transcripts ran in `cwd` (already NFC): more of them
/// name it than name any other path. The path itself is compared, not its
/// encoding, since two paths can encode alike; and one transcript an older
/// rule filed here from elsewhere doesn't decide it either way.
fn ran_in(dir: &Path, cwd: &str) -> bool {
    use std::io::Read;
    let Ok(rd) = fs::read_dir(dir) else { return false };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false) && p.is_file()).collect();
    files.sort();
    let (mut here, mut elsewhere) = (0, 0);
    for p in files.iter().take(50) {
        let Ok(f) = fs::File::open(p) else { continue };
        // a line can carry a whole image; the cwd is in the first few
        for line in BufReader::new(f.take(256 * 1024)).lines().take(20).flatten() {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(c) = v["cwd"].as_str() {
                if nfc(c) == cwd { here += 1 } else { elsewhere += 1 }
                break;
            }
        }
    }
    here > elsewhere
}

/// Where the rule before 1.6.2 filed a transcript whose own folder didn't exist
/// yet: any folder whose name, minus its last "-word", started the path. A Cursor
/// convert, an import or a set_folder wrote there, so a transcript that isn't
/// where it belongs is looked for there. Read only: set_folder is how a chat is
/// moved back into its own folder.
fn misfiled(cwd: &str, id: &str) -> Option<String> { misfiled_in(&proj(), cwd, id) }
fn misfiled_in(root: &str, cwd: &str, id: &str) -> Option<String> {
    let current = enc_cwd(cwd);
    let chars = enc_cwd_chars(cwd);
    for name in indexed_folders(root, id) {
        let Some((prefix, _)) = name.rsplit_once('-') else { continue };
        if prefix.len() >= 24 && (current.starts_with(prefix) || chars.starts_with(prefix)) {
            return Some(format!("{}/{}/{}.jsonl", root, name, id));
        }
    }
    None
}

/// The folders holding a transcript with this id, from an index of the whole
/// projects tree. The index is built on the first miss and kept for a few
/// seconds, so a scan reads the tree once however many transcripts are
/// missing: Claude Code prunes transcripts while the desktop keeps their
/// records, so misses only grow. Only the one id's folders are copied out.
fn indexed_folders(root: &str, id: &str) -> Vec<String> {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    type Idx = (Instant, String, std::collections::HashMap<String, Vec<String>>);
    static CACHE: OnceLock<Mutex<Option<Idx>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, r, idx)) = g.as_ref() {
        if r == root && at.elapsed() < Duration::from_secs(5) { return idx.get(id).cloned().unwrap_or_default(); }
    }
    let mut idx: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    if let Ok(rd) = fs::read_dir(root) {
        for d in rd.flatten() {
            let folder = d.file_name().to_string_lossy().to_string();
            let Ok(files) = fs::read_dir(d.path()) else { continue };
            for f in files.flatten() {
                let n = f.file_name().to_string_lossy().to_string();
                if let Some(id) = n.strip_suffix(".jsonl") {
                    idx.entry(id.to_string()).or_default().push(folder.clone());
                }
            }
        }
    }
    let found = idx.get(id).cloned().unwrap_or_default();
    *g = Some((Instant::now(), root.to_string(), idx));
    found
}

#[cfg(test)]
mod archive_tests {
    #[test]
    fn restore_takes_the_newest_kept_copy_backup_or_delete_snapshot() {
        let v = std::env::temp_dir().join(format!("ferry-arch-{}", std::process::id()));
        let vs = v.to_string_lossy().to_string();
        let id = "local_1111aaaa-0000-4000-8000-000000000001";
        let put = |rel: &str, title: &str| {
            let p = v.join(rel); std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, format!("{{\"title\":{:?}}}", title)).unwrap();
        };
        // glob answers in the platform's own separators, so the path is read
        // back one way to be said about one way
        let kept = |vs: &str| super::archived_copy_in(vs, id).unwrap()
            .to_string_lossy().replace('\\', "/");
        assert!(super::archived_copy_in(&vs, id).is_none(), "nothing kept yet");
        put(&format!("chats/20260901-100000/acct-a/{id}.json"), "from a backup");
        assert!(kept(&vs).contains("chats/20260901"), "{}", kept(&vs));
        // deleted in Ferry after that backup: the delete snapshot is the newer copy
        put(&format!("snapshots/20260905-120000-delete-{id}.json"), "at delete");
        assert!(kept(&vs).contains("-delete-"), "{}", kept(&vs));
        // and a later backup wins again
        put(&format!("chats/20260910-080000/acct-a/{id}.json"), "later backup");
        assert!(kept(&vs).contains("chats/20260910"), "{}", kept(&vs));
        // other snapshots of the same record (a rename, a move) are not restore sources
        put(&format!("snapshots/20260911-090000-rename-{id}.json"), "renamed");
        assert!(kept(&vs).contains("chats/20260910"), "{}", kept(&vs));
        let _ = std::fs::remove_dir_all(&v);
    }
}

#[cfg(test)]
mod project_dir_tests {
    #[test]
    fn a_sibling_project_is_not_a_truncated_name() {
        let root = std::env::temp_dir().join(format!("ferry-pdir-{}", std::process::id()));
        let r = root.to_string_lossy().to_string();
        std::fs::create_dir_all(root.join("-Users-bob-Documents-GitHub-ferry")).unwrap();
        assert_eq!(super::project_dir_in(&r, "/Users/bob/Documents/GitHub/newapp"),
                   format!("{r}/-Users-bob-Documents-GitHub-newapp"), "not the ferry project's folder");
        // a long path's new folder gets Claude Code's own cut-and-hash name
        let long = format!("/Users/bob/{}", "deep/".repeat(60));
        let named = super::project_dir_in(&r, &long);
        assert_eq!(named, format!("{r}/{}", super::cc_folder(&long)));
        // a folder another version cut, with a different hash, is found when its
        // transcripts ran in this path, and not when they ran in another long
        // path that shares the same first 200 characters
        let enc = super::enc_cwd(&long);
        let other = format!("{long}service-b");
        let cut_other = format!("{}-zzzzzz", &enc[..super::CUT]);
        std::fs::create_dir_all(root.join(&cut_other)).unwrap();
        std::fs::write(root.join(&cut_other).join("x.jsonl"),
                       format!("{{\"type\":\"user\",\"cwd\":{:?}}}\n", other)).unwrap();
        assert_eq!(super::project_dir_in(&r, &long), format!("{r}/{}", super::cc_folder(&long)),
                   "another project's cut folder is not this path's");
        let cut = format!("{}-1a2b3c", &enc[..super::CUT]);
        std::fs::create_dir_all(root.join(&cut)).unwrap();
        std::fs::write(root.join(&cut).join("y.jsonl"),
                       format!("{{\"type\":\"user\",\"cwd\":{:?}}}\n", long)).unwrap();
        assert_eq!(super::project_dir_in(&r, &long), format!("{r}/{cut}"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two long paths can encode alike (a dot and a dash both become "-"), and a
    /// cut folder can hold a transcript an older rule filed there from elsewhere.
    #[test]
    fn a_cut_folder_is_matched_on_the_path_itself() {
        let root = std::env::temp_dir().join(format!("ferry-pdir-alike-{}", std::process::id()));
        let r = root.to_string_lossy().to_string();
        let base = format!("/Users/bob/{}", "deep/".repeat(45));
        let (dotted, dashed) = (format!("{base}app.v2"), format!("{base}app-v2"));
        assert_eq!(super::enc_cwd(&dotted), super::enc_cwd(&dashed));
        let line = |c: &str| format!("{{\"type\":\"user\",\"cwd\":{:?}}}\n", c);
        let cut = format!("{}-qqqqqq", &super::enc_cwd(&dotted)[..super::CUT]);
        std::fs::create_dir_all(root.join(&cut)).unwrap();
        std::fs::write(root.join(&cut).join("a.jsonl"), line(&dotted)).unwrap();
        std::fs::write(root.join(&cut).join("b.jsonl"), line(&dotted)).unwrap();
        assert_eq!(super::project_dir_in(&r, &dotted), format!("{r}/{cut}"));
        assert_eq!(super::project_dir_in(&r, &dashed), format!("{r}/{}", super::cc_folder(&dashed)),
                   "a path that only encodes the same gets its own folder");
        // one transcript filed here from the other path doesn't flip either answer
        std::fs::write(root.join(&cut).join("0.jsonl"), line(&dashed)).unwrap();
        assert_eq!(super::project_dir_in(&r, &dotted), format!("{r}/{cut}"));
        assert_ne!(super::project_dir_in(&r, &dashed), format!("{r}/{cut}"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Claude Code's own folder names, computed by its JavaScript (T=200, gz) in
    /// Node and pasted here: short, accented, emoji, Windows, and cut paths.
    #[test]
    fn folder_names_match_claude_code() {
        for (path, want) in [
            ("/Users/ali/code/app", "-Users-ali-code-app"),
            ("/Users/ali/Projét", "-Users-ali-Proj-t"),
            ("/Users/ali/😀 notes", "-Users-ali----notes"),
            ("C:\\Users\\shakaib\\code\\api", "C--Users-shakaib-code-api"),
            ("/Users/bob/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep", "-Users-bob-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-44fwdd"),
            ("/Users/ali/😀/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/deep/Projét", "-Users-ali----deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-deep-d-v8pkh9")
        ] {
            assert_eq!(super::cc_folder(path), want, "{path}");
        }
    }

    /// This machine's own chats, read only: how many find their transcript in
    /// their own folder, how many only where the old rule misfiled it, and how
    /// many have none. Prints counts, nothing else. cargo test -- --ignored
    #[test]
    #[ignore]
    fn real_chats_still_find_their_transcripts() {
        let (mut direct, mut refound, mut gone) = (0, 0, 0);
        for f in glob::glob(&format!("{}/*/*/local_*.json", super::sess())).unwrap().flatten() {
            let Some(rec) = super::read_json(&f.to_string_lossy()) else { continue };
            let Some(t) = super::transcripts_for(&rec).into_iter().next() else { continue };
            let own = format!("{}/{}.jsonl", super::project_dir(rec["cwd"].as_str().unwrap_or("")), t["id"].as_str().unwrap_or(""));
            if !t["exists"].as_bool().unwrap_or(false) { gone += 1; }
            else if t["path"].as_str() == Some(own.as_str()) { direct += 1; }
            else { refound += 1; }
        }
        eprintln!("direct {direct}, found where the old rule misfiled them {refound}, no transcript {gone}");
    }

    #[test]
    fn a_transcript_the_old_rule_misfiled_is_still_found() {
        let root = std::env::temp_dir().join(format!("ferry-misfiled-{}", std::process::id()));
        let r = root.to_string_lossy().to_string();
        let sib = root.join("-Users-bob-Documents-GitHub-ferry");
        std::fs::create_dir_all(&sib).unwrap();
        std::fs::write(sib.join("abc-123.jsonl"), "{}\n").unwrap();
        let found = super::misfiled_in(&r, "/Users/bob/Documents/GitHub/newapp", "abc-123").unwrap();
        assert!(found.ends_with("-Users-bob-Documents-GitHub-ferry/abc-123.jsonl"), "{found}");
        assert!(super::misfiled_in(&r, "/Users/bob/Documents/GitHub/newapp", "other-id").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// The newest copy of a chat's record that Ferry kept: a backup
/// (chats/<stamp>/<account>/<id>.json) or the snapshot Delete takes
/// (snapshots/<stamp>-delete-<id>.json), whichever is later. Without the second,
/// a chat deleted in Ferry before any backup could never be restored.
fn archived_copy(id: &str) -> Option<PathBuf> { archived_copy_in(&vault(), id) }
fn archived_copy_in(vault: &str, id: &str) -> Option<PathBuf> {
    let (v, i) = (glob::Pattern::escape(vault), glob::Pattern::escape(id));
    let backups = glob::glob(&format!("{v}/chats/*/*/{i}.json")).ok()?.flatten()
        .filter_map(|p| Some((p.parent()?.parent()?.file_name()?.to_string_lossy().to_string(), p)));
    let deletes = glob::glob(&format!("{v}/snapshots/*-delete-{i}.json")).ok()?.flatten()
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().chars().take(15).collect::<String>(), p)));
    backups.chain(deletes).max_by(|a, b| a.0.cmp(&b.0)).map(|(_, p)| p)
}

/// The title of a chat's newest kept copy.
fn archived_title(id: &str) -> Option<String> {
    read_json(&archived_copy(id)?.to_string_lossy())?["title"].as_str()
        .filter(|t| !t.is_empty()).map(str::to_string)
}

fn read_json(p: &str) -> Option<Value> {
    fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
}
#[cfg(target_os = "macos")]
fn app_running() -> bool {
    // -a is not optional. pgrep hides itself and every one of its ancestors
    // unless it is passed, so a Ferry launched from the target app's own
    // terminal would be told the app is not running.
    Command::new("pgrep").args(["-a", "-f", "Claude.app/Contents/MacOS/Claude"])
        .output().map(|o| !o.stdout.is_empty()).unwrap_or(false)
}
/// Every folder a Windows Claude can keep its state in: a direct install's
/// %APPDATA%\Claude, the Microsoft Store build's
/// %LOCALAPPDATA%\Packages\Claude_<publisher>\LocalCache\Roaming\Claude, and
/// %LOCALAPPDATA%\Claude-Data\<install>, where newer builds move both of them
/// - leaving only a `.user-data-left.<install>` file behind. Nothing else
/// pointed Ferry there, so after that move every account was gone from it.
#[cfg(target_os = "windows")]
fn claude_dirs() -> Vec<String> {
    let appdata = std::env::var("APPDATA")
        .unwrap_or_else(|_| format!("{}/AppData/Roaming", home()));
    let local = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| format!("{}/AppData/Local", home()));
    claude_dirs_in(&appdata, &local)
}

/// Kept apart from the environment so it can be tested on a made-up tree.
/// Claude-Data comes first: on a machine with no chats anywhere yet, that is
/// the folder Claude writes to next. Only a folder that is a profile counts,
/// so nothing else Claude keeps there turns into a location.
#[cfg(target_os = "windows")]
fn claude_dirs_in(appdata: &str, local: &str) -> Vec<String> {
    let mut cands: Vec<String> = fs::read_dir(format!("{}\\Claude-Data", local))
        .map(|rd| rd.flatten().map(|e| e.path().display().to_string())
             .filter(|p| is_profile(p)).collect::<Vec<String>>())
        .unwrap_or_default();
    cands.sort();
    cands.push(format!("{}\\Claude", appdata));
    if let Ok(rd) = fs::read_dir(format!("{}\\Packages", local)) {
        let mut pkgs: Vec<String> = rd.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
            .map(|e| format!("{}\\LocalCache\\Roaming\\Claude", e.path().display()))
            .collect();
        pkgs.sort();
        cands.extend(pkgs);
    }
    cands
}

/// The desktop app is Chromium: while it runs it holds <user data>\lockfile open,
/// and Windows deletes the file when that handle closes, crash included. So a
/// held lockfile means running. This replaced a PowerShell process query that
/// took ~2 s, flashed a console window, and only matched the Store install.
///
/// The handle is not exclusive, though, and that is the whole trick. It has
/// delete access - that is how delete-on-close works - and lets others read. A
/// second open collides with it only if that open refuses to share delete.
/// std's File::open shares read, write and delete, so it was let straight in
/// and reported Claude as quit while it was plainly open: measured against a
/// running Store install, where the CLI's plain open() caught it every time.
/// lock_held shares read and write only - a sharing violation while the app
/// runs, and a clean open, or no file at all, once it has gone.
///
/// A process list is no substitute. The Claude Code CLI is claude.exe too, both
/// the standalone one and the one the desktop app's Code tab runs, so counting
/// claude.exe would refuse forever while any terminal session was open.
#[cfg(target_os = "windows")]
fn app_running() -> bool {
    // a Store install and a direct install can sit side by side, and the one
    // that is running need not be the one with the newest chats
    claude_dirs().iter().any(|d| lock_held(&format!("{}\\lockfile", d)))
}

#[cfg(target_os = "windows")]
fn lock_held(path: &str) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    match fs::OpenOptions::new().read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)   // not FILE_SHARE_DELETE, see above
        .open(path)
    {
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
        Ok(_) => false,
    }
}

/// Chromium's lockfile, rebuilt with nothing but std: delete access, delete on
/// close, others allowed to read, write and delete. Windows-only because the
/// mechanism is; there is no such handle on a Mac to model.
#[cfg(all(test, target_os = "windows"))]
mod lockfile_tests {
    use std::os::windows::fs::OpenOptionsExt;

    #[test]
    fn a_held_chromium_lockfile_reads_as_running() {
        const GENERIC_READ: u32 = 0x8000_0000;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const DELETE: u32 = 0x0001_0000;
        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
        let path = std::env::temp_dir().join("ferry-lockfile-test");
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);
        assert!(!super::lock_held(&p), "no lockfile is not a running app");

        let holder = std::fs::OpenOptions::new().read(true).write(true).create(true)
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
            .share_mode(0x7)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&path).unwrap();
        assert!(super::lock_held(&p), "a held lockfile must read as running");
        // the open the guard used to make is let in, which was the bug
        assert!(std::fs::File::open(&path).is_ok());

        drop(holder);
        assert!(!path.exists(), "delete-on-close takes the file with the handle");
        assert!(!super::lock_held(&p), "once the app has gone it is not running");
    }
}

/// A record has to be writable while Claude is open, which is the whole point of
/// the per-file swap. Windows-only because the swap and the retry are; on a Mac
/// the guard still blocks the write and a plain `fs::write` is all there is.
#[cfg(all(test, target_os = "windows"))]
mod write_record_tests {
    use std::fs;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ferry-write-record-{}-{}", tag, super::now_ms()));
        let _ = fs::create_dir_all(&d);
        d
    }

    #[test]
    fn writes_a_new_record_and_leaves_no_temp_behind() {
        let d = tmp_dir("new");
        let dst = d.join("local_x.json");
        let p = dst.to_string_lossy().to_string();
        super::write_record(&p, "{\"a\":1}").unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "{\"a\":1}");
        assert!(!d.join("local_x.json.ferrytmp").exists(), "the temp file is swapped away, not left");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn overwrites_an_existing_record_in_place() {
        let d = tmp_dir("over");
        let dst = d.join("local_y.json");
        let p = dst.to_string_lossy().to_string();
        fs::write(&dst, "old").unwrap();
        super::write_record(&p, "new").unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new", "the swap replaces the old bytes");
        assert!(!d.join("local_y.json.ferrytmp").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_write_leaves_neither_a_record_nor_a_temp() {
        // a folder that is not there: the temp write itself fails, and nothing
        // half-made is left for Claude's local_*.json scan to find
        let base = tmp_dir("gone");
        let d = base.join("no-such-account");
        let p = d.join("local_z.json").to_string_lossy().to_string();
        assert!(super::write_record(&p, "x").is_err());
        assert!(!d.join("local_z.json").exists() && !d.join("local_z.json.ferrytmp").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_transfer_goes_through_while_claude_is_open_unless_claude_would_undo_it() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tmp_dir("open");
        let root = super::Root { id: "t".into(), path: d.to_string_lossy().to_string(),
                                 label: String::new(), builtin: false };
        // Claude's lockfile, held the way Chromium holds it (see lockfile_tests)
        let holder = fs::OpenOptions::new().read(true).write(true).create(true)
            .access_mode(0x8000_0000 | 0x4000_0000 | 0x0001_0000)   // read, write, delete
            .share_mode(0x7)
            .custom_flags(0x0400_0000)                               // delete on close
            .open(d.join("lockfile")).unwrap();
        assert!(super::guard_in(&root).is_err(), "an edit of a record Claude holds still waits");
        assert!(super::guard_transfer(&root).is_ok(), "a transfer does not");

        let rec = d.join("local_q.json").to_string_lossy().to_string();
        let tomb = d.join("deleted_q");
        let tomb_s = tomb.to_string_lossy().to_string();
        assert!(super::refuse_resurrect(&root, &rec, &tomb_s).is_ok(), "a chat new to the account is fine");
        fs::write(&tomb, "1").unwrap();
        assert!(super::refuse_resurrect(&root, &rec, &tomb_s).is_err(),
                "one in its deleted list is what an open Claude would delete again");

        drop(holder);
        assert!(super::refuse_resurrect(&root, &rec, &tomb_s).is_ok(), "once Claude has quit it can come back");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn remove_record_takes_the_file_and_reports_a_missing_one() {
        let d = tmp_dir("rm");
        let dst = d.join("local_r.json");
        let p = dst.to_string_lossy().to_string();
        fs::write(&dst, "x").unwrap();
        super::remove_record(&p).unwrap();
        assert!(!dst.exists(), "the record is gone");
        assert!(super::remove_record(&p).is_err(), "a second remove reports it is already gone");
        let _ = fs::remove_dir_all(&d);
    }
}

/// The vault is shared with the CLI, so a Windows stamp has to be the same shape
/// the CLI writes and sort against it correctly. Windows-only because on a Mac
/// `date` has always answered in this shape.
#[cfg(all(test, target_os = "windows"))]
mod stamp_tests {
    #[test]
    fn a_windows_stamp_is_a_date_the_cli_can_be_compared_with() {
        let s = super::stamp();
        assert_eq!(s.len(), 15, "yyyymmdd-hhmmss is 15 characters: {s}");
        assert_eq!(&s[8..9], "-", "the date and the time are separated: {s}");
        assert!(s[..8].chars().chain(s[9..].chars()).all(|c| c.is_ascii_digit()),
                "everything either side of the dash is a digit: {s}");

        let (y, mo, d) = (&s[..4], &s[4..6], &s[6..8]);
        assert!(("2020".."2100").contains(&y), "a plausible year: {y}");
        assert!(("01"..="12").contains(&mo), "a real month: {mo}");
        assert!(("01"..="31").contains(&d), "a real day: {d}");

        // the regression this replaced: the app stamped an epoch count, and
        // every epoch written so far begins with a 1, so it sorted below every
        // date in this format - a restore comparing names picked the CLI's
        // copy however old it was
        assert!(s.as_str() > "1789039713", "a date must outrank the epochs the app wrote: {s}");
        assert!(s.as_str() > "20200101-000000" && s.as_str() < "21000101-000000",
                "and must sort between two dates either side of it: {s}");
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn app_running() -> bool { false }
fn guard() -> Result<(), String> {
    if app_running() { Err("Claude desktop is running. Quit it first, then retry.".into()) }
    else { Ok(()) }
}

/// A write into one profile only has to wait for the Claude that has *that*
/// profile open. On Windows every profile holds its own lockfile, so the
/// question can be asked per folder. Elsewhere there is one process list and
/// nothing in it says which folder the app was started with, so the answer
/// stays the cautious one: any Claude blocks every write, as it always did.
#[cfg(target_os = "windows")]
fn app_running_in(root: &str) -> bool { lock_held(&format!("{}\\lockfile", root)) }
#[cfg(not(target_os = "windows"))]
fn app_running_in(_root: &str) -> bool { app_running() }

fn guard_in(root: &Root) -> Result<(), String> {
    if !app_running_in(&root.path) { return Ok(()); }
    Err(if root.label.is_empty() {
        "Claude desktop is running. Quit it first, then retry.".to_string()
    } else {
        format!("Claude is open with the {} profile. Quit that window first, then retry.", root.label)
    })
}

/// The guard for a transfer: a chat copied, moved or added into an account.
/// On Windows that no longer waits for Claude. The record it writes is new to
/// that account - Claude has never loaded it, so there is nothing in Claude's
/// memory for it to clash with - and it is swapped in whole (`write_record`),
/// so an open Claude never reads half of it. Claude lists it on its next reload
/// or restart. Edits to a record Claude may already hold - rename, folder,
/// delete, restore - keep `guard_in`: Claude keeps those in memory and writes
/// its own copy back over them. Elsewhere a transfer waits for Claude to quit,
/// as every write always has.
#[cfg(target_os = "windows")]
fn guard_transfer(_root: &Root) -> Result<(), String> { Ok(()) }
#[cfg(not(target_os = "windows"))]
fn guard_transfer(root: &Root) -> Result<(), String> { guard_in(root) }

/// The one transfer an open Claude would undo: a chat in this account's deleted
/// list is one Claude holds as deleted, so writing it back while that Claude
/// runs is exactly the write it reverses - the rule Nearby's receive follows
/// too. Only Windows gets here with Claude open; elsewhere `guard_transfer` has
/// already refused, so there is nothing to check.
#[cfg(target_os = "windows")]
fn refuse_resurrect(root: &Root, rec: &str, tomb: &str) -> Result<(), String> {
    if Path::new(tomb).exists() && !Path::new(rec).exists() && app_running_in(&root.path) {
        return Err(if root.label.is_empty() {
            "That chat is in this account's deleted list, and Claude is open, so Claude could delete it again. Quit Claude, then retry.".to_string()
        } else {
            format!("That chat is in this account's deleted list, and Claude is open with the {} profile, so Claude could delete it again. Quit that window, then retry.", root.label)
        });
    }
    Ok(())
}
#[cfg(not(target_os = "windows"))]
fn refuse_resurrect(_root: &Root, _rec: &str, _tomb: &str) -> Result<(), String> { Ok(()) }

/// A file another process holds for a moment: a sharing violation, or access
/// denied, which is what replacing or deleting a file someone has open without
/// delete sharing gets. Worth a short retry; any other error is not.
#[cfg(target_os = "windows")]
fn held_briefly(e: &std::io::Error) -> bool { matches!(e.raw_os_error(), Some(32) | Some(5)) }

/// Write a record so an open Claude never sees half of it. The bytes go to a
/// sibling first - named so Claude's `local_*.json` scan skips it - and are
/// swapped in with one same-directory rename (replace-existing), retried
/// briefly while Claude happens to be reading the old file. Elsewhere a write
/// waits for Claude to quit, so a plain write has always been safe and that is
/// all this is.
#[cfg(target_os = "windows")]
fn write_record(dst: &str, bytes: &str) -> Result<(), String> {
    let tmp = format!("{}.ferrytmp", dst);
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let mut i = 0u64;
    loop {
        match fs::rename(&tmp, dst) {
            Ok(()) => return Ok(()),
            Err(e) if held_briefly(&e) && i < 10 => {
                i += 1;
                std::thread::sleep(std::time::Duration::from_millis(40 * i));
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(format!("could not write {}: {}", dst, e));
            }
        }
    }
}
#[cfg(not(target_os = "windows"))]
fn write_record(dst: &str, bytes: &str) -> Result<(), String> {
    fs::write(dst, bytes).map_err(|e| e.to_string())
}

/// Take a moved chat's record out of the account it left, retried the same way
/// while Claude is reading it. The caller took a snapshot first, so a removal
/// that still fails leaves a copy behind, never a loss.
#[cfg(target_os = "windows")]
fn remove_record(path: &str) -> std::io::Result<()> {
    let mut i = 0u64;
    loop {
        match fs::remove_file(path) {
            Err(e) if held_briefly(&e) && i < 10 => {
                i += 1;
                std::thread::sleep(std::time::Duration::from_millis(40 * i));
            }
            r => return r,
        }
    }
}
#[cfg(not(target_os = "windows"))]
fn remove_record(path: &str) -> std::io::Result<()> { fs::remove_file(path) }
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
    let dbs: Vec<String> = roots().iter().map(|r| idb_in(&r.path)).collect();
    for e in dbs.iter().flat_map(|d| walkdir::WalkDir::new(d).into_iter().filter_map(|e| e.ok())) {
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
        let mut here = dir.clone();
        let mut main = format!("{}/{}.jsonl", here, id);
        if !Path::new(&main).exists() {
            if let Some(p) = misfiled(cwd, id) {
                here = Path::new(&p).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or(here);
                main = p;
            }
        }
        let size = fs::metadata(&main).map(|m| m.len()).unwrap_or(0);
        let mut subs = 0u64; let mut sub_bytes = 0u64;
        if let Ok(rd) = glob::glob(&format!("{}/{}/subagents/*.jsonl", here, id)) {
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
        // A leftover JSONL from converting out of Cursor, not the Cursor app.
        // That dest is cursor_scope(), one row. Never emit source "cursor" here.
        "cursor"         => ("other",   "Other sessions"),
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
            let (mut kind, mut name) = source_of(info["entrypoint"].as_str().unwrap_or(""));
            // leftover JSONL must not become the Cursor dest
            if kind == "cursor" { kind = "other"; name = "Other sessions"; }
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
            // a source belongs to no profile: these chats are on the machine,
            // not in anybody's account, which is the whole point of listing them
            "root": "", "rootLabel": "",
            "source": kind, "sourceName": names.get(&kind).copied().unwrap_or("Sessions"),
            "chats": chats, "deleted": [], "connectors": {},
            "isCurrent": false, "label": "", "profile": Value::Null
        })
    }).collect();
    out.sort_by_key(|s| std::cmp::Reverse(
        s["chats"].as_array().unwrap().iter().map(|c| c["last"].as_u64().unwrap_or(0)).max().unwrap_or(0)));
    out
}

mod cursor {
use super::*;

/* ---------- Cursor ----------
   Cursor is a separate application with a storage of its own: one SQLite file
   holding every conversation, not a folder of transcripts. A chat is a row in
   composerHeaders, an ordered list of bubble ids in composerData:<id>, and one
   bubbleId:<chat>:<bubble> row per message. Nothing about it resembles the
   JSONL Claude Code appends, so a chat cannot be moved between them - it has to
   be converted. Reads are always read-only. Writes (Claude -> Cursor) are
   refused while Cursor is running: the file is tens of GB and Cursor holds
   it open. Do not trust SQLite's lock for that - the file opens read-write
   while the app is up. */

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

fn cursor_app_dir() -> String {
    Path::new(&cursor_dir()).parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(cursor_dir)
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    Command::new("kill").args(["-0", &pid.to_string()])
        .stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
        Err(_) => false,
    }
}

#[cfg(not(any(unix, windows)))]
fn pid_alive(_pid: u32) -> bool { false }

/// True if the Cursor app is up. code.lock holds the main pid; pgrep /
/// tasklist is the fallback when the lock file was left behind. SQLite
/// opening read-write is not a signal - that succeeds while Cursor is running.
pub fn cursor_running() -> bool {
    let lock = format!("{}/code.lock", cursor_app_dir());
    if let Ok(s) = fs::read_to_string(&lock) {
        if let Ok(pid) = s.trim().parse::<u32>() {
            if pid_alive(pid) { return true; }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // -a is not optional here. By default pgrep excludes itself and all of
        // its ancestors, and Cursor's main process is an ancestor of any Ferry
        // started from a Cursor terminal - so the plain form answered "not
        // running" for a Cursor that was plainly running.
        return Command::new("pgrep").args(["-a", "-f", "Cursor.app/Contents/MacOS/Cursor"])
            .output().map(|o| !o.stdout.is_empty()).unwrap_or(false);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let out = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq Cursor.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        return out.map(|o| String::from_utf8_lossy(&o.stdout).contains("Cursor.exe")).unwrap_or(false);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { false }
}

fn cursor_guard() -> Result<(), String> {
    if cursor_running() { Err("Cursor is running - quit it first, then retry".into()) }
    else { Ok(()) }
}

/// Signed-in Cursor account: email and display name from ItemTable.
/// The CLI login in ~/.cursor/cli-config.json is a different account; ignore it.
pub fn cursor_profile() -> Option<Value> {
    let c = cursor_open()?;
    let email: String = c.query_row(
        "select value from ItemTable where key=?1",
        ["cursorAuth/cachedEmail"],
        |r| Ok(cell(r, 0).trim().trim_matches('"').to_string()),
    ).unwrap_or_default();
    let raw: String = c.query_row(
        "select value from ItemTable where key=?1",
        ["cursorAuth/cachedScopedProfile"],
        |r| Ok(cell(r, 0)),
    ).unwrap_or_default();
    let name = serde_json::from_str::<Value>(&raw).ok()
        .and_then(|v| v["displayName"].as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    if email.is_empty() && name.is_empty() { return None; }
    Some(json!({ "email": email, "name": name }))
}

/// Read-only, and never anything else.
fn cursor_open() -> Option<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    // A plain path, not a URI. Cursor on macOS lives under Application Support,
    // and a space inside a URI filename is a different file. READ_ONLY is the
    // flag that matters; the CLI's file:?mode=ro is the same guarantee.
    rusqlite::Connection::open_with_flags(cursor_db()?,
        OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
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
        // percent-decoding, for the spaces in "c:/Drive/Local LLM". The bytes
        // are UTF-8: collected as bytes, "Proj%C3%A9t" is Projét, not ProjÃ©t.
        let mut raw: Vec<u8> = vec![];
        let b = rest.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(n) = u8::from_str_radix(&rest[i+1..i+3], 16) {
                    raw.push(n); i += 3; continue;
                }
            }
            raw.push(b[i]); i += 1;
        }
        let mut p = String::from_utf8_lossy(&raw).to_string();
        if cfg!(target_os = "windows") { p = p.replace('/', "\\"); } else { p.insert(0, '/'); }
        if let Some(d) = f.parent().and_then(|d| d.file_name()) {
            out.insert(d.to_string_lossy().to_string(), p);
        }
    }
    out
}

/// The one query behind both the conversation list and a single conversation,
/// so a chat opened on its own is the chat that was in the list. narrow is the
/// extra predicate that picks one row.
///
/// The counting happens in SQLite. composerData keeps the only copy of the
/// bubble list and runs to 181 MB across these conversations, so carrying each
/// blob over here to count it froze the window for a second on every load;
/// json_each walks the same array in C and gives back two integers. The two
/// joins are the emptiness test as well, because a conversation with no bubbles
/// contributes no rows to group and falls out on its own.
///
/// The subagent filter is in SQL for the same reason: 3,509 of the 3,825 header
/// rows are a subagent's own side conversation, and reading their 5.9 MB of
/// value text only to drop it here was 5.9 MB read for nothing.
fn chat_sql(narrow: &str) -> String {
    // Most bubbles are tool calls with nothing to read. The header of each says
    // whether it has text, so how much was actually said is counted without
    // opening three thousand rows.
    format!("select h.composerId, h.workspaceId, h.createdAt, h.lastUpdatedAt, \
                    h.isArchived, json_extract(h.value,'$.name'), count(*), \
                    sum(json_extract(e.value,'$.grouping.hasText')=1) \
               from composerHeaders h \
               join cursorDiskKV d on d.key='composerData:'||h.composerId \
               join json_each(d.value,'$.fullConversationHeadersOnly') e \
              where coalesce(h.isSubagent,0)=0 {} \
              group by h.composerId order by h.lastUpdatedAt desc", narrow)
}

/// One conversation row, however it was found.
fn chat_row(ws: &std::collections::BTreeMap<String, String>, r: &rusqlite::Row)
    -> rusqlite::Result<Value>
{
    let wid = cell(r, 1);
    let bubbles = r.get::<_, Option<i64>>(6).unwrap_or(None).unwrap_or(0);
    let said = r.get::<_, Option<i64>>(7).unwrap_or(None).unwrap_or(0);
    Ok(json!({
        "id": cell(r, 0),
        "title": r.get::<_, Option<String>>(5).unwrap_or(None)
            .filter(|s| !s.is_empty()).unwrap_or_else(|| "(unnamed)".into()),
        "folder": ws.get(&wid).cloned().unwrap_or_default(),
        "created": r.get::<_, Option<i64>>(2).unwrap_or(None).unwrap_or(0),
        "last": r.get::<_, Option<i64>>(3).unwrap_or(None).unwrap_or(0),
        "bubbles": if said > 0 { said } else { bubbles },
        "archived": r.get::<_, Option<i64>>(4).unwrap_or(None).unwrap_or(0) != 0
    }))
}

/// Every Cursor conversation that has anything in it. Most headers are empty
/// shells left behind by windows that were opened and closed.
pub fn cursor_chats() -> Vec<Value> {
    let Some(c) = cursor_open() else { return vec![] };
    chats_from(&c, &cursor_folders())
}

/// Grouped first, one conversation at a time only if that died.
///
/// json_each cannot skip a row it cannot read. One malformed composerData blob
/// fails the whole grouped statement, so the list would come back empty instead
/// of one chat short. The slow path is the only one that can drop just the bad
/// chat. Asking json_valid first would cost a second pass over all 181 MB of
/// blob on every load, so the cost is paid on the failure rather than on every
/// load that has nothing wrong with it.
fn chats_from(c: &rusqlite::Connection,
              ws: &std::collections::BTreeMap<String, String>) -> Vec<Value> {
    match chats_grouped(c, ws) {
        Ok(v) => v,
        Err(_) => chats_slow(c, ws),
    }
}

/// The one statement, with the error kept rather than dropped: a blob json_each
/// choked on halfway through must not read as the end of the list.
fn chats_grouped(c: &rusqlite::Connection,
                 ws: &std::collections::BTreeMap<String, String>)
    -> rusqlite::Result<Vec<Value>>
{
    let mut st = c.prepare(&chat_sql(""))?;
    let rows = st.query_map([], |r| chat_row(ws, r))?;
    rows.collect()
}

/// One conversation at a time, with the blob parsed here rather than in SQLite,
/// so a conversation whose blob will not parse is the only thing lost.
fn chats_slow(c: &rusqlite::Connection,
              ws: &std::collections::BTreeMap<String, String>) -> Vec<Value> {
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
        let Some(data) = cursor_get(c, &format!("composerData:{}", cid)) else { continue };
        let Some(d) = serde_json::from_str::<Value>(&data).ok() else { continue };
        let heads = d["fullConversationHeadersOnly"].as_array().cloned().unwrap_or_default();
        if heads.is_empty() { continue; }
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

/// One conversation by id, without building the whole list to find it. The
/// primary key answers this in well under a millisecond.
///
/// No fallback wanted here: the predicate leaves json_each one blob to walk, so
/// the only blob that can fail this is the one being asked for, and the old code
/// skipped that chat too.
pub fn cursor_chat(cid: &str) -> Option<Value> {
    let c = cursor_open()?;
    let ws = cursor_folders();
    c.query_row(&chat_sql("and h.composerId=?1"), [cid], |r| chat_row(&ws, r)).ok()
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

/// One Cursor conversation as plain turns, oldest first, and how many turns it
/// has in all.
///
/// Most bubbles carry no prose at all - in a 4,563 bubble chat only a couple of
/// hundred do, and three thousand are tool calls. Dropping those would throw
/// away what the conversation actually did, so a run of them is folded into the
/// message before it as one line each. They are written as text, not as
/// tool_use blocks: a tool_use has to be answered by a tool_result, and there
/// is nothing on this side to answer it with.
///
/// tail is how many turns with prose are wanted, counted from the newest, or 0
/// for every one of them. The headers are in order, so a tail reads them
/// backwards and stops as soon as it has enough: the worst conversation on this
/// machine is 6,721 bubbles and 441 MB of blob for 2,581 turns, and the newest
/// 300 of those sit in its last 787 bubbles and 95 MB. Only the display path
/// passes a tail. Converting a conversation reads all of it, see cursor_messages.
fn cursor_turns(cid: &str, tail: usize) -> (Vec<Value>, usize) {
    let Some(c) = cursor_open() else { return (vec![], 0) };
    let Some(data) = cursor_get(&c, &format!("composerData:{}", cid)) else { return (vec![], 0) };
    let Ok(d) = serde_json::from_str::<Value>(&data) else { return (vec![], 0) };
    let heads = d["fullConversationHeadersOnly"].as_array().cloned().unwrap_or_default();
    // the count the chat list already shows, taken from the headers alone, so
    // it is still the whole conversation when the read below stops early
    let said = heads.iter().filter(|h| h["grouping"]["hasText"].as_bool().unwrap_or(false)).count();
    let total = if said > 0 { said } else { heads.len() };

    // One statement for thousands of lookups. Connection::query_row prepares a
    // fresh one on every call, and this runs once a bubble.
    let Ok(mut q) = c.prepare("select value from cursorDiskKV where key=?1") else { return (vec![], total) };

    // stamp, Cursor's own bubble type, prose, tool call line
    let mut read: Vec<(String, i64, String, String)> = vec![];
    let mut got = 0usize;
    for h in heads.iter().rev() {
        let Some(bid) = h["bubbleId"].as_str() else { continue };
        let Ok(raw) = q.query_row([format!("bubbleId:{}:{}", cid, bid)], |r| Ok(cell(r, 0))) else { continue };
        let Ok(b) = serde_json::from_str::<Value>(&raw) else { continue };
        let text = b["text"].as_str().unwrap_or("").trim().to_string();
        let tool = if text.is_empty() && !b["toolFormerData"].is_null() {
            cursor_tool_line(&b["toolFormerData"])
        } else { String::new() };
        if text.is_empty() && tool.is_empty() { continue; }
        let prose = !text.is_empty();
        read.push((h["createdAt"].as_str().or(b["createdAt"].as_str()).unwrap_or("").to_string(),
                   b["type"].as_i64().unwrap_or(0), text, tool));
        if prose {
            got += 1;
            if tail > 0 && got >= tail { break; }
        }
    }
    read.reverse();                          // a tool run belongs to the turn after it

    let mut out: Vec<Value> = vec![];
    let mut pending: Vec<String> = vec![];
    for (ts, typ, text, tool) in read {
        if text.is_empty() { pending.push(tool); continue; }
        let role = if typ == 1 { "user" } else { "assistant" };
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
    }
    if !pending.is_empty() {
        let ts = out.last().map(|m| m["t"].clone()).unwrap_or(Value::Null);
        out.push(json!({ "role": "assistant", "text": pending.join("\n"), "t": ts }));
    }
    (out, total)
}

/// Every turn in a conversation, for the paths that convert one rather than
/// show it. No cap reaches here on purpose: a converted chat that quietly lost
/// messages cannot be told from a complete one, and the copy is what the user
/// keeps.
fn cursor_messages(cid: &str) -> Vec<Value> { cursor_turns(cid, 0).0 }

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

/// Whether this transcript is one Ferry wrote from a Cursor conversation, which
/// is the only kind a conversion may overwrite. It says so on every line; only
/// the first is read, because the file being asked about can be 25 MB.
fn ferry_wrote(path: &str) -> bool {
    let Ok(f) = fs::File::open(path) else { return false };
    let mut line = String::new();
    if BufReader::new(f).read_line(&mut line).is_err() { return false; }
    serde_json::from_str::<Value>(&line)
        .map(|v| v["entrypoint"].as_str() == Some("cursor"))
        .unwrap_or(false)
}

#[cfg(test)]
mod transcript_tests {
    /// A Claude Code session must never read as Ferry's own, or converting a
    /// round-tripped chat back would overwrite the conversation it came from.
    #[test]
    fn only_ferrys_own_transcript_counts_as_ours() {
        let dir = std::env::temp_dir();
        let ours = dir.join("ferry-wrote-ours.jsonl");
        let theirs = dir.join("ferry-wrote-theirs.jsonl");
        std::fs::write(&ours, "{\"type\":\"user\",\"entrypoint\":\"cursor\"}\n").unwrap();
        std::fs::write(&theirs, "{\"type\":\"user\",\"entrypoint\":\"claude-desktop\"}\n").unwrap();
        assert!(super::ferry_wrote(&ours.to_string_lossy()));
        assert!(!super::ferry_wrote(&theirs.to_string_lossy()), "a Claude session is not ours");
        assert!(!super::ferry_wrote(&dir.join("ferry-wrote-absent.jsonl").to_string_lossy()));
        let _ = std::fs::remove_file(&ours);
        let _ = std::fs::remove_file(&theirs);
    }
}

/// Claude Code transcript lines for turns that came from somewhere else. The
/// shape is the one Claude Code appends: one JSON object a line, each linked to
/// the one before it. entrypoint says where it really came from, so nothing
/// later mistakes it for a session Claude Code ran itself.
pub fn transcript_lines(cid: &str, cwd: &str, msgs: &[Value]) -> String {
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
    body
}

/// A Cursor conversation as transcript lines, written nowhere: Nearby sends
/// these without leaving a transcript behind on this machine.
pub fn cursor_transcript(chat: &Value) -> Result<String, String> {
    let cid = chat["id"].as_str().ok_or("no conversation id")?;
    let msgs = cursor_messages(cid);
    if msgs.is_empty() { return Err("that Cursor chat has nothing readable in it".into()); }
    Ok(transcript_lines(cid, chat["folder"].as_str().unwrap_or(""), &msgs))
}

/// A Cursor conversation written into ~/.claude/projects as a Claude Code
/// transcript, so an account can claim it.
pub fn cursor_write_transcript(chat: &Value) -> Result<String, String> {
    let cid = chat["id"].as_str().ok_or("no conversation id")?;
    let cwd = chat["folder"].as_str().unwrap_or("");
    if cwd.is_empty() { return Err("that Cursor chat has no folder on this machine".into()); }
    let msgs = cursor_messages(cid);
    if msgs.is_empty() { return Err("that Cursor chat has nothing readable in it".into()); }

    let dir = project_dir(cwd);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = format!("{}/{}.jsonl", dir, cid);

    // A Claude chat converted into Cursor keeps its session id as the composer
    // id, so converting it back names the transcript after that same session -
    // and that is the file the original chat already has, whenever the chat's
    // folder is the Cursor workspace itself rather than something inside it. On
    // the machine this was found on that was 8 of 14 chats, the largest 25 MB
    // and 9,567 lines. The write below is a plain overwrite, so the chat would
    // have come back as a few hundred flattened turns with the rest gone, and
    // nothing would have said so.
    if Path::new(&path).exists() && !ferry_wrote(&path) {
        return Err(format!(
            "that would overwrite a transcript Claude Code wrote, at {path}. \
             Nothing was changed."));
    }
    snapshot(&path, "transcript");          // does nothing when there is no file yet
    fs::write(&path, transcript_lines(cid, cwd, &msgs)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Cursor's conversations as one more source, and as a drop target for a
/// Claude chat going the other way. They have no transcript to point at until
/// one is written, so they are addressed by "cursor:<conversation id>".
/// Shown whenever the DB file exists, even with 0 chats: the send-to list
/// and the sidebar drop target need the row, not a conversation count.
pub fn cursor_scope() -> Option<Value> {
    if cursor_db().is_none() { return None; }
    let chats = cursor_chats();
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
    let prof = cursor_profile().unwrap_or(Value::Null);
    Some(json!({
        "acct": "source:cursor", "org": "source", "kind": "source",
        "root": "", "rootLabel": "",
        "source": "cursor", "sourceName": "Cursor",
        "chats": list, "deleted": [], "connectors": {},
        "isCurrent": !prof.is_null(), "label": "", "profile": prof
    }))
}

/// A Cursor chat read straight out of Cursor, before anything is written.
/// Only the newest turns come back: the whole of the worst conversation here is
/// 2.59 MB and 46,296 nodes, which locked the window on every click. The window
/// asks for the rest with cursor_full.
pub fn cursor_detail(cid: &str) -> Result<Value, String> { cursor_view(cid, 300) }

/// The same chat with nothing held back, for the window's "load all".
pub fn cursor_full(cid: &str) -> Result<Value, String> { cursor_view(cid, 0) }

fn cursor_view(cid: &str, tail: usize) -> Result<Value, String> {
    let chat = cursor_chat(cid).ok_or("no such Cursor chat")?;
    let (turns, total) = cursor_turns(cid, tail);
    let msgs: Vec<Value> = turns.iter().map(|m| json!({
        "role": m["role"], "t": m["t"].as_str().unwrap_or("").chars().take(16).collect::<String>(),
        "text": m["text"].as_str().unwrap_or("").chars().take(24000).collect::<String>(),
        "tools": []
    })).collect();
    let bytes: u64 = msgs.iter().map(|m| m["text"].as_str().unwrap_or("").len() as u64).sum();
    let rec = json!({
        "sessionId": Value::Null, "cliSessionId": cid,
        "title": chat["title"], "cwd": chat["folder"], "model": "",
        "createdAt": chat["created"], "lastActivityAt": chat["last"],
        "completedTurns": total, "isArchived": chat["archived"],
        "source": "cursor", "sourceName": "Cursor"
    });
    Ok(json!({ "rec": rec, "files": [], "source": "cursor", "sourceName": "Cursor",
               "bytes": bytes, "subs": 0, "msgs": msgs,
               "total": total, "full": tail == 0 }))
}

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

/// Same id the Python CLI mints, so either side rewrites one Cursor chat.
fn ferry_uuid(seed: &str) -> String {
    let h = fnv1a(&format!("ferry-claude:{seed}"));
    let h2 = fnv1a(&format!("ferry-claude2:{seed}"));
    format!("{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        (h >> 32) as u32, ((h >> 16) & 0xffff) as u16, (h & 0xfff) as u16,
        ((h2 >> 48) & 0xfff) as u16, h2 & 0xffffffffffff)
}

fn looks_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b[8] == b'-' && b[13] == b'-' && b[18] == b'-' && b[23] == b'-'
}

fn composer_id(rec: &Value) -> String {
    let sid = rec["cliSessionId"].as_str()
        .map(String::from)
        .or_else(|| rec["sessionId"].as_str().map(|s| s.strip_prefix("local_").unwrap_or(s).to_string()))
        .unwrap_or_default();
    if looks_uuid(&sid) { sid } else { ferry_uuid(if sid.is_empty() {
        rec["title"].as_str().unwrap_or("chat")
    } else { &sid }) }
}

fn leap(y: i64) -> bool { y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) }

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let mut days = 0i64;
    for yy in 1970..y { days += if leap(yy) { 366 } else { 365 }; }
    const MD: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for mm in 1..m {
        days += MD[(mm - 1) as usize];
        if mm == 2 && leap(y) { days += 1; }
    }
    days + d - 1
}

fn civil_from_unix_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 { y += 1; }
    (y as i32, m as u32, d as u32)
}

fn iso_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, d) = civil_from_unix_days(secs.div_euclid(86400));
    let tod = secs.rem_euclid(86400);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        y, m, d, tod / 3600, (tod % 3600) / 60, tod % 60)
}

fn parse_iso_ms(raw: &str) -> Option<i64> {
    let raw = raw.trim().trim_end_matches('Z').split('+').next().unwrap_or(raw);
    if raw.len() < 16 { return None; }
    let y: i64 = raw.get(0..4)?.parse().ok()?;
    let mo: i64 = raw.get(5..7)?.parse().ok()?;
    let d: i64 = raw.get(8..10)?.parse().ok()?;
    let h: i64 = raw.get(11..13)?.parse().ok()?;
    let mi: i64 = raw.get(14..16)?.parse().ok()?;
    let s: i64 = raw.get(17..19).and_then(|x| x.parse().ok()).unwrap_or(0);
    Some((days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s) * 1000)
}

fn ms_from(v: &Value, fallback: i64) -> i64 {
    if let Some(n) = v.as_i64() {
        if n > 10_i64.pow(11) { return n; }
        if n > 10_i64.pow(9) { return n * 1000; }
    }
    if let Some(n) = v.as_u64() {
        if n > 10u64.pow(11) { return n as i64; }
        if n > 10u64.pow(9) { return (n as i64) * 1000; }
    }
    if let Some(s) = v.as_str() {
        if let Ok(n) = s.parse::<i64>() {
            if n > 10_i64.pow(11) { return n; }
        }
        if let Some(ms) = parse_iso_ms(s) { return ms; }
    }
    fallback
}

fn now_ms_i() -> i64 { now_ms() as i64 }

fn cursor_richtext(text: &str) -> String {
    let children: Vec<Value> = text.split('\n').map(|para| json!({
        "children": if para.is_empty() { vec![] } else { vec![json!({
            "detail": 0, "format": 0, "mode": "normal", "style": "",
            "text": para, "type": "text", "version": 1
        })] },
        "direction": "ltr", "format": "", "indent": 0,
        "type": "paragraph", "version": 1
    })).collect();
    json!({ "root": {
        "children": if children.is_empty() {
            vec![json!({"children":[],"direction":"ltr","format":"","indent":0,"type":"paragraph","version":1})]
        } else { children },
        "direction": "ltr", "format": "", "indent": 0, "type": "root", "version": 1
    }}).to_string()
}

fn path_key(p: &str) -> String {
    let s = Path::new(p).to_string_lossy().to_string();
    if cfg!(windows) { s.to_ascii_lowercase() } else { s }
}

fn cursor_workspace_id(cwd: &str) -> String {
    if cwd.is_empty() { return String::new(); }
    let cwd_n = path_key(cwd);
    let sep = if cfg!(windows) { "\\" } else { "/" };
    let mut best = String::new();
    let mut best_len = -1isize;
    for (wid, p) in cursor_folders() {
        let n = path_key(&p);
        if cwd_n == n || cwd_n.starts_with(&(n.clone() + sep)) {
            if n.len() as isize > best_len { best_len = n.len() as isize; best = wid; }
        }
    }
    best
}

fn bubble_skel(bid: &str, typ: i64, text: &str, created: &str) -> Value {
    let mut v = json!({
        "_v": 3, "type": typ, "bubbleId": bid, "text": text,
        "richText": cursor_richtext(text), "createdAt": created,
        "conversationState": "~", "unifiedMode": 2, "isAgentic": false,
        "tokenCount": { "inputTokens": 0, "outputTokens": 0 }
    });
    let o = v.as_object_mut().unwrap();
    for k in [
        "approximateLintErrors", "lints", "codebaseContextChunks", "commits",
        "pullRequests", "attachedCodeChunks", "assistantSuggestedDiffs", "gitDiffs",
        "interpreterResults", "images", "attachedFolders", "attachedFoldersNew",
        "userResponsesToSuggestedCodeBlocks", "suggestedCodeBlocks",
        "diffsForCompressingFiles", "relevantFiles", "toolResults", "notepads",
        "capabilities", "multiFileLinterErrors", "diffHistories",
        "recentLocationsHistory", "recentlyViewedFiles", "fileDiffTrajectories",
        "docsReferences", "webReferences", "aiWebSearchResults",
        "attachedFoldersListDirResults", "humanChanges", "summarizedComposers",
        "cursorRules", "cursorCommands", "contextPieces", "editTrailContexts",
        "allThinkingBlocks", "diffsSinceLastApply", "deletedFiles",
        "supportedTools", "attachedFileCodeChunksMetadataOnly", "consoleLogs",
        "uiElementPicked", "knowledgeItems", "documentationSelections",
        "externalLinks", "projectLayouts", "capabilityContexts", "todos",
        "mcpDescriptors", "workspaceUris", "pastChats",
    ] { o.insert(k.into(), json!([])); }
    for k in [
        "existedSubsequentTerminalCommand", "existedPreviousTerminalCommand",
        "attachedHumanChanges", "cursorCommandsExplicitlySet",
        "pastChatsExplicitlySet", "isRefunded",
    ] { o.insert(k.into(), json!(false)); }
    v
}

fn flatten_turns(msgs: &[Value]) -> Vec<(String, String, String)> {
    let mut out = vec![];
    for m in msgs {
        let mut text = m["text"].as_str().unwrap_or("").to_string();
        if let Some(tools) = m["tools"].as_array() {
            let extra: Vec<String> = tools.iter()
                .filter_map(|t| t.as_str().map(|s| format!("-> {s}")))
                .collect();
            if !extra.is_empty() {
                let block = extra.join("\n");
                text = if text.is_empty() { block } else { format!("{text}\n\n{block}") };
            }
        }
        let text = text.trim().to_string();
        if text.is_empty() { continue; }
        out.push((
            m["role"].as_str().unwrap_or("assistant").to_string(),
            text,
            m["t"].as_str().unwrap_or("").to_string(),
        ));
    }
    out
}

/// How long Ferry is willing to wait for the write lock. Cursor's own
/// connection sets no busy_timeout at all, so it gets SQLITE_BUSY the instant
/// someone else holds the lock and does not retry. Ferry is therefore the one
/// that has to be patient, and the one that has to be quick.
const CURSOR_BUSY_MS: i64 = 5000;

/// The key range holding one conversation's bubbles. A prefix range, not LIKE:
/// it is what Cursor itself uses to sweep a prefix, and on 1.8M rows it is an
/// index seek rather than a scan.
fn bubble_range(cid: &str) -> (String, String) {
    (format!("bubbleId:{cid}:"), format!("bubbleId:{cid};"))
}

/// busy_timeout is a wait, not a queue: a Cursor that is committing without
/// pause can hold the lock every time Ferry looks. The transaction rolls back
/// on the way out, so nothing was written and the honest answer is to say so.
fn busy_or(e: rusqlite::Error) -> String {
    let s = e.to_string();
    if s.contains("locked") || s.contains("busy") {
        return "Cursor is writing too steadily to get a turn - nothing was \
                changed. Try again in a moment, or quit Cursor and convert.".into();
    }
    s
}

/// Insert or replace one converted conversation. Same Claude session always
/// lands on the same composerId, so a second export updates that one chat.
pub fn cursor_write_claude(rec: &Value, msgs: &[Value]) -> Result<Value, String> {
    cursor_guard()?;
    let db = cursor_db().ok_or("no Cursor storage here")?;
    let turns = flatten_turns(msgs);
    if turns.is_empty() {
        return Err("nothing to convert - this chat's transcript was pruned".into());
    }
    let cid = composer_id(rec);
    let title = rec["title"].as_str().unwrap_or("(untitled)").to_string();
    let created = ms_from(&rec["createdAt"], ms_from(&json!(turns[0].2.clone()), now_ms_i()));
    let last = ms_from(&rec["lastActivityAt"], ms_from(&json!(turns.last().unwrap().2.clone()), created));
    let wid = cursor_workspace_id(rec["cwd"].as_str().unwrap_or(""));
    let mut heads = vec![];
    let mut bubbles = vec![];
    for (i, (role, text, t)) in turns.iter().enumerate() {
        let bid = ferry_uuid(&format!("bubble:{cid}:{i}"));
        let typ = if role == "user" { 1 } else { 2 };
        let created_iso = parse_iso_ms(t).map(iso_from_ms)
            .unwrap_or_else(|| iso_from_ms(created + i as i64));
        heads.push(json!({
            "bubbleId": bid, "type": typ, "createdAt": created_iso,
            "grouping": { "isRenderable": true, "hasText": true,
                          "isShortPlainText": text.len() < 200,
                          "toolDisplayComputed": true }
        }));
        bubbles.push((bid.clone(), bubble_skel(&bid, typ, text, &created_iso)));
    }
    let header = json!({
        "type": "head", "composerId": cid, "name": title, "subtitle": "",
        "createdAt": created, "lastUpdatedAt": last, "unifiedMode": 2,
        "forceMode": "edit", "hasUnreadMessages": false, "isDraft": false,
        "isArchived": false, "isSpec": false, "isProject": false,
        "workspaceIdentifier": { "id": wid }
    });
    let data = json!({
        "_v": 17, "composerId": cid, "name": title, "status": "completed",
        "createdAt": created, "lastUpdatedAt": last, "unifiedMode": 2,
        "hasLoaded": true, "fullConversationHeadersOnly": heads,
        "text": turns[0].1, "richText": cursor_richtext(&turns[0].1)
    });
    let snap_dir = format!("{}/snapshots", vault());
    let _ = fs::create_dir_all(&snap_dir);
    let stem = format!("{}/{}-cursor-{}", snap_dir, stamp(), &cid[..8.min(cid.len())]);
    let _ = fs::write(format!("{stem}.json"),
        serde_json::to_string_pretty(&json!({
            "composerId": cid, "title": title, "workspaceId": wid,
            "header": header, "composerData": data,
            "bubbles": bubbles.iter().map(|(_, b)| b).collect::<Vec<_>>()
        })).unwrap_or_default());

    let mut c = rusqlite::Connection::open(&db).map_err(|e| e.to_string())?;
    c.busy_timeout(std::time::Duration::from_millis(CURSOR_BUSY_MS as u64))
        .map_err(|e| e.to_string())?;
    c.execute("create table if not exists composerHeaders (\
        composerId text primary key, workspaceId text, createdAt integer,\
        lastUpdatedAt integer, isArchived integer, isSubagent integer,\
        recency integer, checkpointAt integer, value text, subagentTypeName text)", [])
        .map_err(|e| e.to_string())?;
    c.execute("create table if not exists cursorDiskKV (\
        key text unique on conflict replace, value blob)", [])
        .map_err(|e| e.to_string())?;
    let (lo, hi) = bubble_range(&cid);

    // One transaction, taken up front and let go quickly. Everything between
    // BEGIN IMMEDIATE and COMMIT is time Cursor's own writes would fail in, so
    // there is nothing in here that is not the write itself. Without it, a
    // 4,500 message chat was 4,500 separate commits with the old rows already
    // deleted - anything that stopped halfway left half a conversation.
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| busy_or(e))?;
    {
        let was_header: Option<Vec<Value>> = tx.query_row(
            "select composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, \
                    isSubagent, recency, checkpointAt, value, subagentTypeName \
             from composerHeaders where composerId=?1",
            [&cid],
            |r| Ok((0..10).map(|i| json!(cell(r, i))).collect()),
        ).ok();
        let mut was_kv: Vec<Value> = vec![];
        {
            let mut q = tx.prepare("select key, value from cursorDiskKV \
                                    where key=?1 or (key>=?2 and key<?3)")
                .map_err(|e| e.to_string())?;
            let rows = q.query_map(rusqlite::params![format!("composerData:{cid}"), &lo, &hi],
                    |r| Ok(json!([cell(r, 0), cell(r, 1)])))
                .map_err(|e| e.to_string())?;
            for row in rows { if let Ok(v) = row { was_kv.push(v); } }
        }
        // a second convert of the same Claude chat: keep what it replaces
        if was_header.is_some() || !was_kv.is_empty() {
            let _ = fs::write(format!("{stem}-replaced.json"),
                serde_json::to_string_pretty(&json!({
                    "composerId": cid, "header": was_header, "cursorDiskKV": was_kv
                })).unwrap_or_default());
        }
        tx.execute("delete from cursorDiskKV where key>=?1 and key<?2",
                   rusqlite::params![&lo, &hi]).map_err(|e| e.to_string())?;
        tx.execute("insert or replace into composerHeaders \
                   (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, \
                    isSubagent, recency, checkpointAt, value, subagentTypeName) \
                   values (?1,?2,?3,?4,0,0,?5,?6,?7,'')",
            rusqlite::params![cid, wid, created, last, last, last, header.to_string()])
            .map_err(|e| e.to_string())?;
        tx.execute("insert or replace into cursorDiskKV (key, value) values (?1,?2)",
            rusqlite::params![format!("composerData:{cid}"), data.to_string()])
            .map_err(|e| e.to_string())?;
        for (bid, bub) in &bubbles {
            tx.execute("insert or replace into cursorDiskKV (key, value) values (?1,?2)",
                rusqlite::params![format!("bubbleId:{cid}:{bid}"), bub.to_string()])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| busy_or(e))?;
    drop(c);

    // Read it back on a connection of its own. A commit that reported success
    // and a row that is really there are not the same claim.
    let v = cursor_open().ok_or("cannot reopen Cursor storage to verify")?;
    let hdr: i64 = v.query_row("select count(*) from composerHeaders where composerId=?1",
        [&cid], |r| r.get(0)).map_err(|e| e.to_string())?;
    let body: i64 = v.query_row("select count(*) from cursorDiskKV where key=?1",
        [format!("composerData:{cid}")], |r| r.get(0)).map_err(|e| e.to_string())?;
    let bubs: i64 = v.query_row("select count(*) from cursorDiskKV where key>=?1 and key<?2",
        rusqlite::params![&lo, &hi], |r| r.get(0)).map_err(|e| e.to_string())?;
    if hdr != 1 || body != 1 || bubs != bubbles.len() as i64 {
        return Err(format!(
            "the write did not survive: {hdr} header, {body} body, {bubs} of {} messages",
            bubbles.len()));
    }

    Ok(json!({ "ok": true, "id": cid, "title": title, "messages": turns.len(),
               "workspaceId": wid, "folder": rec["cwd"].as_str().unwrap_or(""),
               // Cursor's per-window chat list keeps only the headers whose
               // workspaceIdentifier matches that window, so a chat whose
               // folder matched no Cursor workspace is on disk and in no list.
               // Reloading will not change that, and saying it would be a lie.
               "noWorkspace": wid.is_empty() }))
}

/// The real state.vscdb is tens of gigabytes and opened read-only, so the one
/// thing worth checking here cannot be checked against it: a blob that will not
/// parse. A scratch file with the two table shapes the query needs stands in.
#[cfg(test)]
mod tests {
    fn blob(n: usize) -> String {
        let heads: Vec<String> = (0..n)
            .map(|i| format!("{{\"bubbleId\":\"b{i}\",\"grouping\":{{\"hasText\":true}}}}"))
            .collect();
        format!("{{\"fullConversationHeadersOnly\":[{}]}}", heads.join(","))
    }

    /// Three conversations, the middle one's blob unreadable. Losing that one is
    /// the whole point; losing the other two is the bug this guards.
    #[test]
    fn malformed_blob_costs_one_chat_not_the_list() {
        let path = std::env::temp_dir().join("ferry-fallback-test.vscdb");
        let _ = std::fs::remove_file(&path);
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "create table composerHeaders(composerId text primary key, workspaceId text, \
                createdAt integer, lastUpdatedAt integer, isArchived integer, \
                isSubagent integer, value text); \
             create table cursorDiskKV(key text primary key, value text);").unwrap();
        for (cid, last, sub, data) in [
            ("aaa11111", 3000, 0, blob(4)),
            ("bbb22222", 2000, 0, "{not json at all".to_string()),
            ("ccc33333", 1000, 0, blob(6)),
            ("ddd44444",  500, 1, blob(2)),      // a subagent's side conversation
        ] {
            c.execute("insert into composerHeaders values(?1,'ws1',1000,?2,0,?3,'{\"name\":\"n\"}')",
                      rusqlite::params![cid, last, sub]).unwrap();
            c.execute("insert into cursorDiskKV values('composerData:'||?1,?2)",
                      rusqlite::params![cid, data]).unwrap();
        }
        let ws = std::collections::BTreeMap::new();

        assert!(super::chats_grouped(&c, &ws).is_err(),
                "json_each should fail the grouped statement on the bad blob");
        let got = super::chats_from(&c, &ws);
        assert_eq!(got.len(), 2, "expected the two readable chats, got {got:?}");
        assert_eq!(got[0]["id"], "aaa11111");    // still newest first
        assert_eq!(got[1]["id"], "ccc33333");
        let _ = std::fs::remove_file(&path);
    }
}

}   // mod cursor

use cursor::{cursor_chat, cursor_detail, cursor_full, cursor_running, cursor_scope, cursor_transcript,
             cursor_write_claude, cursor_write_transcript};
#[cfg(test)]
use cursor::transcript_lines;

/// Every <account>/<org> scope under the sessions root, found by walking the
/// directory rather than by pattern matching. Returns (account, org, dir).
/// Every account folder in every root: the root it was found in, the account,
/// the org, and the folder itself.
fn scopes_on_disk() -> Vec<(Root, String, String, PathBuf)> {
    let mut out = vec![];
    for root in roots() {
        let Ok(l1) = fs::read_dir(PathBuf::from(sess_in(&root.path))) else { continue };
        for a in l1.flatten() {
            if !a.path().is_dir() { continue; }
            let acct = a.file_name().to_string_lossy().to_string();
            let Ok(l2) = fs::read_dir(a.path()) else { continue };
            for o in l2.flatten() {
                if !o.path().is_dir() { continue; }
                out.push((root.clone(), acct.clone(),
                          o.file_name().to_string_lossy().to_string(), o.path()));
            }
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
    for (_, _, _, dir) in &scopes {
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
        "roots": location_list(),
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
    // Each profile is signed into its own account, so "the one you are signed
    // into" is a question per root, not per machine.
    let cur_in: std::collections::HashMap<String, String> = roots().iter().filter_map(|r| {
        let who = read_json(&cfg_in(&r.path))?["lastKnownAccountUuid"].as_str()?.to_string();
        Some((r.id.clone(), who))
    }).collect();
    let labs = labels();
    let profs = profiles();
    let mut scopes: std::collections::BTreeMap<String, Value> = Default::default();
    // every transcript some account already answers for; the rest are sources
    let mut claimed: std::collections::HashSet<String> = Default::default();

    let touch = |scopes: &mut std::collections::BTreeMap<String, Value>,
                 root: &Root, acct: &str, org: &str| {
        scopes.entry(format!("{}|{}|{}", root.id, acct, org)).or_insert_with(|| json!({
            "acct": acct, "org": org, "chats": [], "deleted": [],
            "root": root.id, "rootPath": root.path, "rootLabel": root.label,
            "rootBuiltin": root.builtin,
            "connectors": {},
            "isCurrent": cur_in.get(&root.id).map(String::as_str) == Some(acct),
            "label": labs[acct].as_str().unwrap_or(""),
            "profile": if profs[acct].is_null() { Value::Null } else { profs[acct].clone() }
        }));
    };

    for (root, acct, org, dir) in scopes_on_disk() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if !(name.starts_with("local_") && name.ends_with(".json")) { continue; }
            let (acct, org) = (acct.clone(), org.clone());
            let Some(rec) = read_json(&p.to_string_lossy()) else { continue };
            touch(&mut scopes, &root, &acct, &org);
            let key = format!("{}|{}|{}", root.id, acct, org);
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
    for (root, acct, org, dir) in scopes_on_disk() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let base = e.file_name().to_string_lossy().to_string();
            if !base.starts_with("deleted_") { continue; }
            let (acct, org) = (acct.clone(), org.clone());
            touch(&mut scopes, &root, &acct, &org);
            let short = base.trim_start_matches("deleted_").to_string();
            let when: Option<u64> = fs::read_to_string(&p).ok().and_then(|s| s.trim().parse().ok());
            let key = format!("{}|{}|{}", root.id, acct, org);
            scopes.get_mut(&key).unwrap()["deleted"].as_array_mut().unwrap()
                .push(json!({ "id": format!("local_{}", short), "when": when,
                              "path": p.to_string_lossy() }));
        }
    }
    // A deleted chat is named by its title where one survives: in the account a
    // move took it to, else in the newest archived copy, which Restore would use.
    let titles: std::collections::HashMap<String, String> = scopes.values()
        .flat_map(|s| s["chats"].as_array().cloned().unwrap_or_default())
        .filter_map(|c| Some((c["id"].as_str()?.to_string(), c["title"].as_str()?.to_string())))
        .collect();
    for s in scopes.values_mut() {
        for d in s["deleted"].as_array_mut().unwrap() {
            let id = d["id"].as_str().unwrap_or("").to_string();
            if let Some(t) = titles.get(&id).cloned().or_else(|| archived_title(&id)) {
                d["title"] = json!(t);
            }
        }
    }
    // An account is listed before it has a chat. Signing in is what makes its
    // folder, and an account with nothing in it yet is just where a chat gets
    // copied to - left out, it could not be picked as the destination at all.
    // Once per profile, so an org folder that stays empty beside one with
    // chats adds no second row.
    for (root, acct, org, _) in scopes_on_disk() {
        let mine = format!("{}|{}|", root.id, acct);
        if scopes.keys().any(|k| k.starts_with(&mine)) { continue; }
        touch(&mut scopes, &root, &acct, &org);
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
            "cursorRunning": cursor_running(),
            "locations": location_list(),
            "vault": vault(), "exportDir": last_export_dir(),
            "version": env!("CARGO_PKG_VERSION"),
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
    root_of(path)?;
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

/// The rest of a conversation chat_detail held back, asked for by the window.
#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn chat_full(path: String) -> Result<Value, String> {
    match path.strip_prefix("cursor:") {
        Some(cid) => cursor_full(cid),
        // a Claude chat arrives whole already, so there is nothing else to fetch
        None => chat_detail(path),
    }
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
fn copy_chat(path: String, acct: String, org: String, mv: bool, root: Option<String>)
    -> Result<Value, String>
{
    let src = root_of(&path)?;
    // No root named means the account is in the same profile as the chat, which
    // is every copy on a machine nobody has added a folder on.
    let dst = match root.as_deref().filter(|r| !r.is_empty()) {
        None => src.clone(),
        Some(id) => root_by_id(id).ok_or("Ferry is no longer reading that folder")?,
    };
    // A move takes the record out of the source as well, so wherever a transfer
    // waits for Claude, both ends have to be free of a Claude that could put it
    // back. On Windows it goes ahead: if the Claude that has the source open
    // writes its copy back before it reloads, the chat ends up in both accounts
    // - a copy, never a loss - which is why the card says to reload.
    if mv { guard_transfer(&src)?; }
    guard_transfer(&dst)?;
    let rec = read_json(&path).ok_or("source unreadable")?;
    let sid = rec["sessionId"].as_str().ok_or("no sessionId")?.to_string();
    let short = sid.trim_start_matches("local_").to_string();
    let dst_dir = format!("{}/{}/{}", sess_in(&dst.path), acct, org);
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let rec_at = format!("{}/{}.json", dst_dir, sid);
    refuse_resurrect(&dst, &rec_at, &format!("{}/deleted_{}", dst_dir, short))?;
    let dst = rec_at;

    let mut out = rec.clone();
    if let Some(o) = out.as_object_mut() { for k in ACCOUNT_SCOPED { o.remove(k); } }
    snapshot(&dst, "overwrite");
    write_record(&dst, &serde_json::to_string_pretty(&out).unwrap())?;

    let tomb = format!("{}/deleted_{}", dst_dir, short);
    if Path::new(&tomb).exists() { snapshot(&tomb, "undelete"); let _ = fs::remove_file(&tomb); }

    if mv {
        snapshot(&path, "moved-out");
        let src_dir = Path::new(&path).parent().unwrap().to_string_lossy().to_string();
        let _ = remove_record(&path);
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
    guard_in(&root_of(&path)?)?;
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
    // The same folder is still work to do when a transcript was found somewhere
    // else, as the rule before 1.6.2 filed some: this moves it home.
    let home_dir = project_dir(&dir);
    let astray = transcripts_for(&rec).iter().any(|t| {
        t["exists"].as_bool().unwrap_or(false)
            && t["path"].as_str().and_then(|p| Path::new(p).parent())
                .map(|p| p != Path::new(&home_dir)).unwrap_or(false)
    });
    if dir == was && !astray { return Ok(json!({ "ok": true, "unchanged": true, "cwd": dir })); }

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
fn import_session(path: String, acct: String, org: String, root: Option<String>)
    -> Result<Value, String>
{
    let dst = match root.as_deref().filter(|r| !r.is_empty()) {
        None => primary_root(),
        Some(id) => root_by_id(id).ok_or("Ferry is no longer reading that folder")?,
    };
    guard_transfer(&dst)?;
    // A Cursor chat has no transcript to point at - its conversation lives in
    // Cursor's database - so one is written first, and from there it is an
    // ordinary import. Nearby's receive (share.rs) is the other place Ferry
    // writes a transcript.
    let path = match path.strip_prefix("cursor:") {
        None => path,
        Some(cid) => {
            let chat = cursor_chat(cid).ok_or("no such Cursor chat")?;
            let p = cursor_write_transcript(&chat)?;
            let out = import_session_at(&p, &acct, &org, &dst)?;
            // Cursor already named the conversation; keep its name over the
            // first line of the first prompt
            if let Some(t) = chat["title"].as_str() {
                if t != "(unnamed)" {
                    if let Some(w) = out["wrote"].as_str() {
                        if let Some(mut r) = read_json(w) {
                            r["title"] = json!(t);
                            let _ = write_record(w, &serde_json::to_string_pretty(&r).unwrap());
                        }
                    }
                }
            }
            return Ok(out);
        }
    };
    import_session_at(&path, &acct, &org, &dst)
}

/// Convert a Claude chat into a Cursor conversation. The JSONL stays; Cursor
/// gets invented composer rows. Refused while Cursor is running.
#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn export_to_cursor(path: String) -> Result<Value, String> {
    if path.starts_with("cursor:") {
        return Err("that chat is already in Cursor".into());
    }
    let (rec, tr) = chat_parts(&path)?;
    let msgs = collect_msgs(&tr, 100_000, 2_000_000);
    cursor_write_claude(&rec, &msgs)
}

/// The import itself, once there is a transcript to import. Kept apart from the
/// command so a Cursor chat, whose transcript has only just been written, takes
/// the very same path as one that was always there.
fn import_session_at(path: &str, acct: &str, org: &str, dst: &Root) -> Result<Value, String> {
    if !(under(&proj(), path) && path.ends_with(".jsonl")) {
        return Err(format!("path is outside the projects folder\n  path: {}\n  root: {}", path, proj()));
    }
    let info = read_session(path).ok_or("transcript unreadable")?;
    let cwd = info["cwd"].as_str().unwrap_or("");
    if cwd.is_empty() { return Err("this transcript does not say which folder it ran in".into()); }
    let sid = Path::new(path).file_stem().ok_or("no session id")?.to_string_lossy().to_string();
    let dst_dir = format!("{}/{}/{}", sess_in(&dst.path), acct, org);
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

    refuse_resurrect(dst, &format!("{}/{}.json", dst_dir, id), &format!("{}/deleted_{}", dst_dir, sid))?;
    let dst = format!("{}/{}.json", dst_dir, id);
    snapshot(&dst, "import");
    write_record(&dst, &serde_json::to_string_pretty(&rec).unwrap())?;
    let tomb = format!("{}/deleted_{}", dst_dir, sid);
    if Path::new(&tomb).exists() { snapshot(&tomb, "undelete"); let _ = fs::remove_file(&tomb); }

    Ok(json!({ "ok": true, "wrote": dst, "id": id,
               "title": info["title"], "turns": info["turns"] }))
}

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn rename_chat(path: String, title: String) -> Result<Value, String> {
    guard_in(&root_of(&path)?)?;
    let mut rec = read_json(&path).ok_or("chat unreadable")?;
    snapshot(&path, "rename");
    let t = if title.trim().is_empty() { "(untitled)".to_string() } else { title.trim().to_string() };
    rec["title"] = json!(t);
    fs::write(&path, serde_json::to_string_pretty(&rec).unwrap()).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "title": t }))
}

#[tauri::command]
async fn delete_chat(app: tauri::AppHandle, path: String) -> Result<Value, String> {
    guard_in(&root_of(&path)?)?;
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
fn undelete_chat(acct: String, org: String, id: String, root: Option<String>)
    -> Result<Value, String>
{
    let dst = match root.as_deref().filter(|r| !r.is_empty()) {
        None => primary_root(),
        Some(r) => root_by_id(r).ok_or("Ferry is no longer reading that folder")?,
    };
    guard_in(&dst)?;
    let short = id.trim_start_matches("local_").to_string();
    let mut src: Option<PathBuf> = None;
    for r in roots() {
        let g = format!("{}/*/*/{}.json", glob::Pattern::escape(&sess_in(&r.path)),
                        glob::Pattern::escape(&id));
        if let Ok(p) = glob::glob(&g) {
            for c in p.flatten() { src = Some(c); break; }
        }
        if src.is_some() { break; }
    }
    if src.is_none() { src = archived_copy(&id); }
    let src = src.ok_or("no surviving copy in any account or the vault")?;
    let mut rec = read_json(&src.to_string_lossy()).ok_or("copy unreadable")?;
    if let Some(o) = rec.as_object_mut() { for k in ACCOUNT_SCOPED { o.remove(k); } }
    let dir = format!("{}/{}/{}", sess_in(&dst.path), acct, org);
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

    for root in roots() {
        let Ok(paths) = glob::glob(&format!("{}/*/*/local_*.json", sess_in(&root.path))) else { continue };
        for p in paths.flatten() {
            let parts: Vec<String> = p.iter().map(|s| s.to_string_lossy().to_string()).collect();
            let acct = parts[parts.len()-3].clone();
            // The same account can be signed in in two profiles with different
            // chats in each, so the profile goes in the folder name. One level,
            // because a restore looks for chats/<stamp>/<account>/<id>.json.
            let d = if root.label.is_empty() { format!("{}/{}", base, acct) }
                    else { format!("{}/{}-{}", base, slug(&root.label), acct) };
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

/* ---------- Nearby: another Ferry on the same network ---------- */

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn nearby(on: bool) -> Result<Value, String> {
    if on { share::start() } else { Ok(share::stop()) }
}

#[tauri::command]
fn nearby_state() -> Value { share::state() }

#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn nearby_send(path: String, to: String) -> Result<Value, String> { share::send(path, to) }

#[tauri::command]
fn nearby_answer(accept: bool, acct: String, org: String, folder: Option<String>,
                 root: Option<String>) -> Result<Value, String> {
    // the account picked on the card can be in any profile, so the chat has to
    // land in that profile's folder and not simply in the one Claude is using
    let sess = match root.as_deref().filter(|r| !r.is_empty()) {
        None => None,
        Some(id) => Some(sess_in(&root_by_id(id).ok_or("Ferry is no longer reading that folder")?.path)),
    };
    share::answer(accept, acct, org, folder, sess)
}

#[tauri::command]
fn nearby_confirm(yes: bool) -> Result<Value, String> { share::confirm_send(yes) }

#[tauri::command]
fn nearby_cancel() -> Value { share::cancel() }

#[tauri::command]
fn nearby_dismiss() -> Value { share::dismiss() }

/* ---------- locations ---------- */

fn write_locations(all: &[Value]) -> Result<(), String> {
    fs::create_dir_all(vault()).map_err(|e| e.to_string())?;
    fs::write(locations_path(), serde_json::to_string_pretty(&json!(all)).unwrap())
        .map_err(|e| e.to_string())
}

/// What each folder Ferry reads is, and how much is in it - so a folder that
/// turns out to hold nothing says so, instead of looking as though it worked.
fn location_list() -> Value {
    let found = scopes_on_disk();
    json!(roots().into_iter().map(|r| {
        let mine: Vec<_> = found.iter().filter(|(x, ..)| x.id == r.id).collect();
        let accts: std::collections::HashSet<&String> = mine.iter().map(|(_, a, ..)| a).collect();
        let chats: usize = mine.iter().map(|(.., dir)| {
            fs::read_dir(dir).map(|rd| rd.flatten().filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("local_") && n.ends_with(".json")
            }).count()).unwrap_or(0)
        }).sum();
        json!({ "id": r.id, "path": r.path, "label": r.label, "builtin": r.builtin,
                "exists": Path::new(&r.path).is_dir(),
                "accounts": accts.len(), "chats": chats })
    }).collect::<Vec<_>>())
}

/// Reads every root to count what is in it, so on Windows it goes off the
/// thread that draws, like the other commands that touch the disk.
#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn locations() -> Value { location_list() }

/// Read a Claude profile Ferry would not have found by itself - the folder a
/// second account's window was started with, after --user-data-dir.
#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn add_location(path: String) -> Result<Value, String> {
    let path = checked_location(&path)?;
    let mut all = stored_locations();
    all.push(json!({ "path": path, "label": leaf(&path), "added": now_ms() as u64 }));
    write_locations(&all)?;
    Ok(json!({ "ok": true, "id": root_id(&path), "locations": location_list() }))
}

/// Forget a folder. Nothing in it is touched: the chats stay where Claude put
/// them, and adding the folder again brings them back into the list.
#[cfg_attr(target_os = "windows", tauri::command(async))]
#[cfg_attr(not(target_os = "windows"), tauri::command)]
fn remove_location(id: String) -> Result<Value, String> {
    if root_by_id(&id).map(|r| r.builtin).unwrap_or(false) {
        return Err("that is one of Claude's own folders; Ferry always reads it".into());
    }
    let before = stored_locations();
    let after: Vec<Value> = before.iter()
        .filter(|l| l["path"].as_str().map(|p| root_id(p) != id).unwrap_or(true))
        .cloned().collect();
    if after.len() == before.len() { return Err("Ferry is not reading that folder".into()); }
    write_locations(&after)?;
    Ok(json!({ "ok": true, "locations": location_list() }))
}

/// A folder on this machine, for a chat that arrives from one where its folder
/// has a path that means nothing here.
#[tauri::command]
async fn pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().set_directory(home()).pick_folder(move |p| { let _ = tx.send(p); });
    match rx.recv().map_err(|e| e.to_string())? {
        Some(fp) => Ok(Some(fp.into_path().map_err(|e| e.to_string())?.to_string_lossy().to_string())),
        None => Ok(None),
    }
}

fn main() {
    share::sweep();                     // staging a quit mid-transfer left behind
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            scan, chat_detail, chat_full, export_chat, copy_chat, import_session, export_to_cursor,
            set_folder, rename_chat, delete_chat, undelete_chat, set_label, run_vault, set_zoom,
            nearby, nearby_state, nearby_send, nearby_answer, nearby_confirm, nearby_cancel,
            nearby_dismiss, pick_folder, locations, add_location, remove_location
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch Ferry");
}
