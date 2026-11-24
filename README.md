<div align="center">

<img src="assets/logo-160.png" width="112" alt="Ferry logo">

<h1>
  Ferry
  <a href="https://github.com/ahkamboh/ferry/releases/latest" title="Download the latest release">
    <img src="assets/download.png" width="24" alt="Download">
  </a>
</h1>

**Your AI coding agents' chats, in whichever account and app you use.**

Ferry finds every chat your coding agents left on this machine, across accounts and apps, and copies, moves or converts it to where you work now. The agent picks up the old context instead of you explaining the project again. Claude Code and Cursor today, Codex next. Everything stays on your machine unless you send a chat to another computer on your Wi-Fi. A native app for macOS and Windows, about a 6 MB download, plus a zero-dependency CLI.

[![Download](https://img.shields.io/badge/%E2%86%93%20Download-macOS%20%7C%20Windows-d97757?style=for-the-badge)](https://github.com/ahkamboh/ferry/releases/latest)
[![Website](https://img.shields.io/badge/Website-ahkamboh.github.io%2Fferry-1f1e1d?style=for-the-badge)](https://ahkamboh.github.io/ferry/)

[![License: MIT](https://img.shields.io/badge/License-MIT-d97757.svg)](LICENSE)
![Platform: macOS and Windows](https://img.shields.io/badge/platform-macOS%2011%2B%20%7C%20Windows%2010%2B-1f1e1d)
![Download: 6 MB](https://img.shields.io/badge/download-6%20MB-1f1e1d)
![Built with Tauri 2](https://img.shields.io/badge/built%20with-Tauri%202%20%2B%20Rust-1f1e1d)

https://github.com/user-attachments/assets/f8100da7-8bc3-46f7-a456-3894b17be269

<sub>Every feature in three minutes, with sound, on example data: a chat moved out of a signed-out account, a CLI chat added to one, Cursor chats converted both ways, a folder fixed, a download, a restore from the archive, a backup, and a chat sent to another computer on the same Wi-Fi once both screens show the same code.</sub>

</div>

---

## Works with

| Agent | What Ferry does |
|---|---|
| **Claude Code** (desktop app, CLI, VS Code) | lists every account's chats, even ones you're signed out of; copies, moves, renames, restores and backs them up; adds CLI and VS Code chats to an account |
| **Cursor** | lists its chats, converts one into a Claude Code chat, and a Claude Code chat into Cursor |
| **Codex** | coming next, not supported yet |

## The problem

Coding agents keep your history on your machine, but tie it to one account or one app. Claude Code is where it hurts most.

You sign into Claude Code with a second account — a work one, a new subscription, a client's — and your chat history is gone. Not deleted, just invisible: Claude Code keeps chat metadata **per account**, so every conversation you had is still on the disk, attached to an account you're no longer signed into.

So you start over and re-explain everything to the model.

Two things make it worse:

- **Transcripts expire.** Claude Code prunes old transcript files. Chats can outlive their own content — the title is listed, the conversation is gone.
- **Deletes can come back.** The desktop app reconciles its own state. A chat you restore by hand can be removed again later.

Ferry fixes both. It reads the files Claude Code already writes, lets you carry a chat from one account to another, and keeps an archive the app can't touch.

## What it does

| | |
|---|---|
| **See every account** | every Claude account you've signed into on this machine, with its chats — even ones you're signed out of |
| **Find CLI and VS Code chats** | sessions you ran with `claude` or in the VS Code extension that no account lists at all. Read them, then pick an account under **Add to** and press **Add to account**, or drag the chat onto it |
| **Find Cursor chats** | Cursor conversations, turned into Claude chats in the folder they were worked in (**Add to account**). A Claude chat goes the other way: pick Cursor in **Send to** and press **Convert**, or drag it onto Cursor |
| **Identify them** | each account by its nickname, its Claude display name or its email. Ferry remembers every account email Claude Code's config has named, so a signed-out account keeps its name. A green dot marks the one you're signed into. Nickname any account and it sticks |
| **Fix a chat's folder** | a chat you started without picking one shows **No folder** in Claude. Click the folder chip in the chat's header, point it at the folder it really belongs to, and Claude names it there |
| **Read any chat** | the conversation with proper Markdown — tables, code blocks, lists, quotes — plus tool calls. The app shows a Claude chat's first 800 messages (tool calls count toward that, and a message is cut at 24,000 characters); **Download** saves all of it. A huge Cursor chat opens on its newest turns, and **Load all** fetches the rest |
| **Copy or move** | pick an account in **Send to**, then **Copy** or **Move**, or drag the chat onto an account and choose on the card. A moved chat can be restored from the deleted list |
| **Rename and search** | click a chat's title to rename it; filter an account's chats by title |
| **Download** | save a whole Claude, CLI or VS Code conversation as Markdown, plain text or JSON, anywhere you choose. A Cursor chat can be downloaded after it's added to an account |
| **Delete and undelete** | deletes are reversible; restore from another account or from the archive |
| **Back up** | one button archives every Claude chat record and every transcript, subagent transcripts included (Cursor's own database isn't copied) |
| **Read another profile** | Claude keeps one folder per profile, so a second account signed in with `--user-data-dir` has chats Ferry would never see. Point Ferry at that folder under **locations** and its accounts join the list, to copy and move between like any other |
| **Nearby** | send a chat from an account, Cursor, the CLI or VS Code to someone else's Ferry on the same Wi-Fi, Mac or Windows. It arrives as a Claude chat, both screens show the same six-digit code first, and receiving works with Claude open. Off until you turn it on. Desktop app only |
| **Zoom** | Ctrl/Cmd + and − (or Ctrl/Cmd + scroll), Ctrl/Cmd 0 to reset; the level is remembered |
| **Keyboard** | F5 or Ctrl/Cmd+R re-reads from disk, Ctrl/Cmd+W closes a chat, Enter confirms a card, Esc cancels |

## Why moving a chat is instant

Claude Code stores your history in two separate layers:

| Layer | Location | Scope |
|---|---|---|
| Chat metadata — title, model, folder, timestamps | `~/Library/Application Support/Claude/claude-code-sessions/<account>/<org>/local_<id>.json` | **per account** |
| Deletion marker | same folder, `deleted_<id>` — a millisecond timestamp | per account |
| Transcripts — the actual conversation | `~/.claude/projects/<encoded-cwd>/<sessionId>.jsonl` | **shared by every account** |
| Subagent transcripts | `~/.claude/projects/<encoded-cwd>/<sessionId>/subagents/*.jsonl` | shared |

Because transcripts are account-agnostic, moving a chat only moves about **10 KB of JSON**. A 45 MB conversation transfers in the same instant as a 40 KB one, and nothing is duplicated on disk.

## Chats no account lists

The same split explains a second kind of missing chat. `claude` in a terminal and the VS Code extension write transcripts into the very same tree — but neither writes the per-account record. **So every chat you started outside the desktop app belongs to no account, and nothing lists it.** It isn't in the app, it isn't in any account, and `claude --resume` only offers it while you're standing in the folder it ran in.

Ferry finds them. Each transcript states which surface wrote it, so they arrive grouped under the accounts, titled by their first message:

```
NOT IN AN ACCOUNT
  >_         Claude Code CLI        6 chats · no account yet
  [VS Code]  VS Code                7 chats · no account yet
  []         Desktop, no record     1 chat · no account yet
  [Cursor]   <your Cursor name>    11 chats · signed in
```

A desktop chat shows up here too once its record is gone but its transcript isn't, and a transcript that doesn't say where it came from lands under **Other sessions**.

Open one and it reads like any other chat. **Add to account** then writes the record it never had, and from that moment Claude lists it, and Ferry can copy, move, rename, archive and delete it like the rest. Nothing is written back into the transcript, so the session stays resumable from where it came.

Two details worth knowing:

- The new record's id comes from the session's own id, so importing the same chat twice updates one record instead of making a second.
- Only the account's environment fields are inherited, copied from a record the app itself wrote there. Connector settings are not, for the same reason they're stripped on copy.

## The folder a chat belongs to

Claude names a chat's folder in its header and resumes the chat there. Start one without picking a folder and it runs in a workspace the app invents for it — `…\Claude\scratch-workspaces\…` — which is why the header reads **No folder**, and why the chat is filed nowhere useful afterwards.

Ferry shows that folder in the open chat's header, as **No folder** when there's none, and lets you set it by clicking it. Pick the folder it really belongs to and Claude names it from then on. A chat not yet in an account shows its folder but can't change it.

There's a subtlety worth knowing, because it's what makes this safe. A chat's folder is two things at once: the folder Claude shows, **and** where the conversation is looked up — the transcript lives under `~/.claude/projects/<encoded folder>/`. Change the folder alone and Claude would show the new one and lose the conversation with it. So Ferry also makes the transcript findable under the new folder, by **hard-linking** it: one file, two names, not a byte duplicated, and the old folder keeps working. Only a volume that refuses links falls back to a copy, and Ferry tells you which happened.

Every change snapshots the record first, so setting a folder is as reversible as everything else here. A chat an older Ferry filed under a sibling project (before 1.6.2) is still found where it was put, and **Set folder** links it into its own folder.

## Install

### Download the app

From [Releases](https://github.com/ahkamboh/ferry/releases/latest):

| Platform | File | Size | Notes |
|---|---|---|---|
| **macOS 11+** | `Ferry-<version>-macos-universal.zip` | 6.1 MB | universal — Apple Silicon and Intel |
| **Windows 10+** | `Ferry.exe` | 6.3 MB | single file, no installer, needs the WebView2 runtime |
| **Windows 10+** | `Ferry-<version>-windows-x64-setup.exe` | 2.3 MB | installer for your user only: Start menu entry, uninstaller, no admin prompt |
| **Windows 10+** | `Ferry-<version>-windows-x64.msi` | 3.1 MB | installer for everyone on the PC: asks for admin |

`Ferry.exe` and `Ferry-macos-universal.zip` (the same file as the versioned zip) keep fixed names, so `releases/latest/download/…` links always get the newest build.

Neither build is code-signed, so both operating systems will warn you once.

**macOS:** open Ferry once, then go to System Settings → Privacy & Security and click **Open Anyway**. (On macOS 14 and earlier, right-click the app → **Open** → **Open** also works.) Or:

```bash
xattr -dr com.apple.quarantine /Applications/Ferry.app
```

**Windows:** SmartScreen shows "Windows protected your PC" → **More info** → **Run anyway**.
If it says **Smart App Control blocked an app** instead, there is no Run anyway: Smart App
Control only allows signed apps. Use the [CLI](#cli)'s `ui` command, which serves the same
interface, or turn Smart App Control off under Windows Security → App & browser control.
Windows 11 already has the WebView2 runtime; on Windows 10 install the
[Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)
if the window opens blank.

### Or build it yourself

Requires [Rust](https://rustup.rs). `build.sh` is macOS; on Windows run
`cargo build --release --manifest-path src-tauri/Cargo.toml`.

```bash
git clone https://github.com/ahkamboh/ferry.git
cd ferry
./build.sh
open ~/Applications/Ferry.app
```

`build.sh` compiles the Rust binary, bundles `Ferry.app`, and installs it to `~/Applications`.

**Windows installer.** With Node.js as well, this builds both `Ferry.exe` and an installer,
`src-tauri/target/release/bundle/nsis/Ferry_<version>_x64-setup.exe`:

```bash
npx @tauri-apps/cli@2 build --bundles nsis
```

The installer needs no admin rights: it installs Ferry for the current user under
`%LOCALAPPDATA%\Programs`, adds it to the Start menu, registers an uninstaller in
Settings → Apps, and fetches the WebView2 runtime if the machine lacks it. The **Windows
build** workflow in Actions builds `Ferry.exe`, the setup `.exe` and the `.msi`, and uploads
them to the release whose tag matches the version.

## CLI

`ferry-cli.py` uses only the Python 3 standard library — no `pip install`, no virtualenv.

```bash
python3 ferry-cli.py list                     # every account and its chats
python3 ferry-cli.py vault                    # archive everything to ~/.ferry
python3 ferry-cli.py export "auth refactor"   # save a chat, Markdown by default
python3 ferry-cli.py export "auth refactor" json
python3 ferry-cli.py export 1191f0ec txt      # disambiguate by session id
python3 ferry-cli.py import 17b163e1 work@    # add a CLI chat to an account
python3 ferry-cli.py folder "auth refactor" ~/code/api   # set a chat's folder
python3 ferry-cli.py cursor                   # Cursor's own chats
python3 ferry-cli.py cursor-import 4191e56f work@       # Cursor -> Claude
python3 ferry-cli.py cursor-export "the title"          # Claude -> Cursor
python3 ferry-cli.py ui                       # serve the app UI at localhost:7777
python3 ferry-cli.py ui --demo                # the same UI on synthetic data
```

`python3 ferry-cli.py` with no command is `ui`, which opens your browser. `list` prints each account with its connectors, then the CLI, VS Code and Cursor chats, five per group, with a short id for each. `import` takes that id (or a piece of the title) and an account — its address, its nickname, or the start of its uuid — and gives the chat a record there.

`export` matches on chat title or session id. If a title matches more than one chat it lists
the candidates with their ids instead of guessing. Files go to the folder you last downloaded
to, `~/Downloads` until you pick another.

`ui` serves the **same interface as the desktop app** — the app's `dist/index.html` with a shim
that turns its `invoke()` calls into HTTP. Markdown rendering, drag-and-drop and the archive all
work there. The differences: downloads go straight to the last folder you used, Set folder asks
for a path instead of opening a folder panel, zoom is left to the browser's own Ctrl/Cmd + and −,
and Nearby is desktop-only.

`ui --demo` runs that same interface against an invented tree in a temp folder: three Claude
accounts with nine chats (one pruned, one with no folder) and a deleted one, CLI and VS Code
sessions no account lists, and a Cursor install with three conversations of its own. Copy, move, add to
account, convert, set folder, rename, download, delete, restore and the archive all run the CLI's
real code against that tree, and your own chats and your real Cursor database are never read. Two
things a browser can't have are stood in: "is the Claude app open" (off unless the recording script
switches it on), and Nearby, where the demo server plays a second computer on the network, "Sam's
PC", answering with the same states the app's Rust side reports. It's how the demo above was
recorded, and it's the quickest way to try Ferry without a Claude install.

Run `vault` from a launchd job or cron and your history is backed up nightly without opening anything.

## Cursor

Cursor keeps its chats nothing like Claude Code does: not a folder of transcripts but a single SQLite file — one row per conversation in `composerHeaders`, an ordered list of bubble ids beside it, and one row per message, all inside `globalStorage/state.vscdb`. So a chat can't be *moved* between them. It has to be converted, both ways.

```bash
python3 ferry-cli.py cursor                         # what Cursor has
python3 ferry-cli.py cursor-import 4191e56f work@   # Cursor -> Claude
python3 ferry-cli.py cursor-export "the chat title" # Claude -> Cursor
```

`cursor` lists the conversations that actually contain something — most headers are empty shells left by windows that were opened and closed — grouped by the folder each was worked in, because Cursor's workspaces map to real directories. Cursor -> Claude lands in the account you name, in that folder. Claude -> Cursor invents composer rows and keeps the Claude JSONL. Converting the same chat twice updates that one conversation.

What survives the crossing: every prompt and reply, in order, with their timestamps, and every tool call as a line naming the tool and its path or command. What doesn't: Cursor's diffs, thinking blocks and attached code chunks, and Claude's thinking blocks. Those have no equivalent on the other side.

Two deliberate limits:

- **Quit Cursor before a write.** Its database is tens of GB and the app holds it open. SQLite will still open it read-write while Cursor is running, so Ferry checks the process (and `code.lock`) instead, and refuses. Reads stay `pragma query_only` / `SQLITE_OPEN_READ_ONLY`.
- **Cursor -> Claude is one of two places Ferry writes a transcript** (the other is receiving a chat over Nearby) rather than only the small record beside it, because there is no transcript to point at. The file is named after the Cursor conversation, so converting the same chat twice rewrites the one file. Sending a Cursor chat over Nearby writes a temporary transcript in `~/.ferry/outgoing/`, deleted when the send ends; your `~/.claude` isn't touched.

The sidebar shows the signed-in Cursor account the same way it shows a Claude one (name and email from Cursor's own ItemTable, not from the CLI login). A Claude chat dropped on Cursor, or sent there with **Send to → Convert**, converts; a CLI or VS Code chat converts when dropped on Cursor. A Cursor chat goes into a Claude account with **Add to account** or by dragging it there, or to another computer from **Add to → Nearby**. Demo mode never points at the real Cursor database: it builds an invented one.

Reading SQLite from Rust means bundling it, which cost about 1 MB (1.4.0); Nearby's discovery and encryption added about half a MB more (1.6.0). The CLI gets SQLite free from Python's standard library.

## The archive

`Back up` copies every chat record and every transcript into `~/.ferry`.

```
~/.ferry/
  chats/<timestamp>/<account>/   chat records, one snapshot per run
                                 <profile>-<account>/ for an added profile's accounts
  projects/                      every transcript, subagents included
  snapshots/                     an automatic copy before each change
  labels.json                    your account nicknames
  profiles.json                  every account email Claude Code's config has named
  prefs.json                     last folder you downloaded to
  sessions.json                  what each CLI/VS Code transcript says about itself
  incoming/ outgoing/            Nearby staging, cleared when a transfer ends or at the next launch
```

The archive is yours, outside anything Claude Code manages. Once a chat is in it, deletion becomes cosmetic — restore it into whichever account you want.

Every `<timestamp>` here is local time as `yyyymmdd-hhmmss`. The app and the CLI write into the same vault, and a restore picks the newest copy by comparing those names, so both write the same shape on both platforms.

## More than one Claude profile

Claude keeps its state in one folder per *profile*, not one per machine. A second window started with `--user-data-dir` gets its own, which is how a second account stays signed in beside the first — and every chat in it is invisible to anything reading only the default folder.

Ferry finds the built-in folders by itself: on macOS `~/Library/Application Support/Claude`, on Windows `%LOCALAPPDATA%\Claude-Data\*`, `%APPDATA%\Claude` and the Microsoft Store container. An account is listed as soon as Claude has made its folder — signing in does that — so a new one can be the destination before it has a chat of its own. Any other folder has to be pointed at once, under **locations** in the sidebar — or from the CLI:

```bash
./ferry-cli.py locations                                  # the folders Ferry reads
./ferry-cli.py locations add "C:\ClaudeAccounts\Account2"  # also read this one
./ferry-cli.py locations rm c6f554c7                      # stop reading it
```

The folder is the one that comes after `--user-data-dir`: it holds `claude-code-sessions` and `config.json`. Ferry refuses a folder that is not a Claude profile, and one that overlaps a folder it already reads, which would list the same chats twice. Removing a location only forgets the path — nothing inside it is touched.

Accounts from an added folder behave like any other: copy, move, rename, delete, restore, back up, and send over Nearby. Two things follow from profiles being separate:

- **The same account can appear twice**, once per profile, with different chats in each. Ferry says which folder each one came from once more than one of them has that account.
- **A write waits only for the Claude that has that profile open.** On Windows each profile holds its own lockfile, so a chat can be renamed, deleted or restored in a profile whose window is closed while another Claude window stays open, and copying, moving or adding a chat doesn't wait at all (see [Safety](#safety)). On macOS Ferry reads which folder each running Claude was started with (`--user-data-dir`, or Claude's own folder) from the process list, copies of Claude.app under other names included, so the same per-profile rule holds there.

## Nearby

Send a chat from an account, Cursor, the CLI or VS Code to another person's Ferry on the same network. It works between a Mac and a Windows PC in either direction, and whatever it came from, it arrives as a Claude chat. Nearby is in the desktop app only.

1. Both people press **Nearby** in the top bar and choose **Turn Nearby on**. It's off until you do.
2. Open the chat, open **Send to** (**Add to** for a Cursor, CLI or VS Code chat), pick the other computer under **Nearby**, and press **Send**. If it doesn't show up (some office and café Wi-Fi blocks discovery), type the address the other person's Nearby menu shows into the box at the bottom of Send to.
3. A six-digit code appears on both screens. Check it matches theirs and press **Codes match**. Until you do, nothing about the chat has been sent, not even its title.
4. The other person sees who's sending, what, and the same code. They pick the account it goes into (**Put it in**) and press **Accept** within 150 seconds. If the chat has no folder, Accept waits until they **Choose…** one; if yours isn't on their machine, choosing one is optional and the folder chip can fix it later.

The first time, **macOS** asks whether Ferry can find devices on your local network: allow it. **Windows** asks to let Ferry through the firewall: allow it, and make sure the Wi-Fi is set to a **Private** network. A chat whose transcript was pruned can't be sent. Sending the same Cursor chat again later replaces the earlier copy on the other side; an older copy never replaces a newer one.

The key exchange is commit-then-reveal, the way Bluetooth pairing does it, so a device sitting between you can't choose keys that make the two codes agree. A device pretending to be your friend gets only your machine's name: your friend never saw its code, so you cancel. The transfer is encrypted (X25519 and ChaCha20-Poly1305), and Ferry only connects to private (10/8, 172.16/12, 192.168/16) and link-local IPv4 addresses, or this machine. Tailscale's 100.64/10, the carrier-grade range and all other IPv6 are refused; a VPN that hands out private addresses counts as local.

Receiving works with Claude and Cursor open. A new chat is one Claude has never loaded, so there's nothing for it to overwrite, and it lists the chat the next time it starts. A chat already in that account is updated where it lives, in its own folder, keeping your title and its history; the sender's folder and name don't replace yours. While Claude is open only its conversation changes, since Claude holds the record in memory. If that account deleted the chat, it arrives as a new copy with an id of its own and the deleted one stays deleted; only a chat deleted under its own conversation id (one sent from Cursor, the CLI or VS Code) is refused until you quit Claude, so Claude doesn't delete it again; then send it again. A chat already in the account that has no folder recorded takes the folder you pick, and that too is refused while Claude is open.

The receiving side decides everything before it writes anything. Ids and file names can't point outside Claude's folders, sizes are capped, files wait in a staging folder until all of them arrive, a chat can't take over another chat's record, and a different conversation under the same id is refused with nothing changed. A longer copy of the same conversation replaces the shorter one, after a snapshot.

Only accept chats from people you trust. A chat you continue in Claude becomes context Claude acts on.

## Safety

- **Copying, moving and adding a chat go through while Claude is open, on Windows and macOS.** The record a transfer writes is new to that account, so Claude has nothing in memory for it to clash with, and Ferry writes it to a sibling file and swaps it in with one rename, retried if Claude is mid-read, so Claude never reads half of it. Claude reads an account's chats only when that account starts, so an open window can't see a file that appears later; when the destination account is the one signed in to an open Claude window, Ferry hands the chat to that window instead, through Claude's own `claude://resume` link (the one Claude Code's `/desktop` uses). Claude adopts the chat, writes its record and opens it there at once: no restart, and the sessions running in that window carry on. Otherwise Claude lists the chat the next time it opens that account. A chat the account has in its deleted list arrives as a new chat with an id of its own, and the deleted one stays deleted, the way Nearby does it: an open Claude holds the old id as deleted and would delete it again. Editing a chat Claude already holds (renaming, deleting, restoring or setting its folder) still waits for that profile's Claude to quit, since Claude keeps those records in memory and writes its own copy back. Adding a Cursor, CLI or VS Code chat works the same way while Claude is open; converting a chat *into* Cursor still needs Cursor closed. On macOS every copy of Claude.app shares one bundle id and the `claude://` scheme, so Ferry hands the link to the exact copy that has that profile open, with `open -a <that copy>`: a second account run from a copy of Claude.app shows the chat at once too.
- **A chat's forks move with it.** A fork names its parent by the parent's id, and a chat Claude adopts gets a new id (`local_<conversation id>`), so Ferry moves a chat's forks along with it, then points the chats it brought in at their parent's id there, matching by conversation. Nothing else in the account is re-linked, and a chat detached to top level in Claude ("Detach to top level", `lineageDetached`) is not a fork to Ferry: it stays on its own, isn't carried with the chat it came from, and stays detached through a hand-off. Forks are handed over like any chat and open straight away; Claude's adopt keeps no link to a parent, so Ferry writes the link back onto the record Claude made, and the fork shows under its parent the next time that window starts.
- **A chat in the middle of a reply can't be moved.** If its newest turn hasn't ended and the window running it is open, that window would write its record straight back and two windows would share one conversation, so Ferry waits for the reply to finish. A chat that has finished moves at once, any chat moves while its account's Claude is closed, and a copy works at any time.
- **After a move, leave the old entry alone.** A window that had the chat's old account open keeps listing it until it next starts: Claude has no way to be told to drop a chat while it runs. Don't open it there (Claude may write its copy back), and don't delete it there: before Claude deletes a chat's conversation file it checks that no other record claims it, but it only looks in its own profile and Claude's standard ones, not in a folder a window was started with through `--user-data-dir`. So deleting the old entry can delete the conversation the moved chat still uses. If the window had the chat open, it saves the record back on its own. Ferry lists such a copy under a folded **Moved out** section, tagged with where the chat lives now (**→ Xaibridge**, or **duplicate** when a move back left two records in one account), and finishes the move by itself the next time that window is closed: it takes the copy out (snapshotted first) and points its forks at the record that stays. A chat moved into an account that already has its conversation doesn't get a second record: the one there is reused.
- **Archived chats are listed apart.** A chat archived in Claude doesn't count among an account's chats in Ferry; it sits in a folded **Archived** section under the list, and a search looks there too.
- **Every change is snapshotted** into `~/.ferry/snapshots/` before it happens.
- **Transcripts are never moved or edited**, with two exceptions: converting a Cursor chat into Claude writes its transcript, and receiving a chat over Nearby writes its transcripts, where a longer copy of a conversation replaces the shorter one after a snapshot. Otherwise only the small metadata record moves. Importing a CLI or VS Code chat writes one; it never writes back into the transcript, and a chat that has no record yet cannot be renamed, moved or deleted. Setting a chat's folder gives its transcript a second name by hard link — the same file, still in the folder it came from, with nothing rewritten.
- **Connector settings are stripped on copy.** MCP connector IDs belong to the account that created them and don't resolve elsewhere.
- **Deleting writes a tombstone**, the same marker Claude Code uses. The conversation stays on disk.

## FAQ

**Where does Claude Code store chat history on macOS?**
Two places. Metadata per account in `~/Library/Application Support/Claude/claude-code-sessions/`, and the conversations themselves as JSONL in `~/.claude/projects/`.

**I switched Claude accounts and my chats disappeared. Are they gone?**
No. The transcripts are still on disk — only the per-account metadata changed. Ferry lists every account it finds and can copy a chat into the one you're using now.

**Can I recover a deleted Claude Code chat?**
Usually. Deletion writes a small tombstone rather than erasing the conversation. **Restore** needs a copy of the chat's record: in another account, in a Ferry backup, or (from 1.6.3) in the snapshot Ferry takes when it deletes a chat. The conversation comes back with it if its transcript is still on disk. A chat deleted in Claude whose transcript survives also shows under **Desktop, no record**, where **Add to account** gives it a new record.

**The Claude app doesn't list the chats I ran in the terminal or in VS Code. Where are they?**
On disk, in `~/.claude/projects`, same as every other chat. What they don't have is the per-account record the desktop app writes, and that record is the only thing the app lists from. Ferry shows them under **not in an account**; **Add to account** writes that record, and Claude lists the chat from then on.

**Does importing a CLI chat take it away from the CLI?**
No. Only the small record is written, and the record is new — the transcript is not touched, so `claude --resume` still offers the session in the folder it ran in.

**Why does a chat show a warning dot?**
Its transcript was pruned by Claude Code's cleanup. The record survives, the conversation doesn't. A backup taken before the cleanup keeps a copy in `~/.ferry/projects/`. Ferry doesn't read from there, so copy the file back into `~/.claude/projects/` to read the chat again.

**Does this send anything anywhere?**
Not over the internet. Ferry reads and writes local files. The one network feature, Nearby, only talks to other Ferry apps on private and link-local addresses, is off until you turn it on, and sends a chat only after both people confirm the same code. The transfer is encrypted.

**The other computer doesn't show up in Nearby.**
Check that Nearby is turned on at both ends, that you allowed Ferry on the local network (macOS) or through the firewall (Windows), and that Windows has the Wi-Fi set to Private. Some office and café networks block discovery; then type the address the other person's Nearby menu shows into the box at the bottom of **Send to**.

**Does it work on Windows or Linux?**
macOS and Windows are both built and tested. On Windows the layout is
`%APPDATA%\Claude\claude-code-sessions` with transcripts in `%USERPROFILE%\.claude\projects`,
and a working directory like `D:\Claude` maps to the folder `D--Claude`. Linux has no Claude
Code desktop app, so there are no per-account chat records to move — only the archive half
would apply.

**Ferry shows no accounts on Windows, but Claude has chats.**
The Microsoft Store build of Claude runs in a container that redirects `%APPDATA%\Claude` to
`%LOCALAPPDATA%\Packages\Claude_<id>\LocalCache\Roaming\Claude`. Only Claude sees the redirect,
so anything outside it — Ferry, or a terminal you opened yourself — finds `%APPDATA%\Claude`
empty. Newer builds of Claude move their data out of both, to `%LOCALAPPDATA%\Claude-Data\<install>`,
and leave only a `.user-data-left.<install>` file where it was. Ferry checks all of them and uses
the one with the newest chats. If it still comes back empty, the window lists every path it
looked at; include that in an issue.

**I keep a second account signed in with `--user-data-dir`. Ferry doesn't list it.**
That profile has a folder of its own, and Ferry only reads the built-in ones until it is told otherwise. Add the folder under **locations** in the sidebar, or `./ferry-cli.py locations add <folder>`, and its accounts appear with the rest.

**Is it affiliated with Anthropic?**
No. Ferry is an independent tool that reads local files written by Claude Code.

## How it's built

Rust + [Tauri 2](https://tauri.app) with a plain HTML/CSS/JS front end — no framework, no npm, no build step for the UI. The Markdown renderer is about 50 lines, written for this app, so nothing is fetched at runtime. The crates that matter: rusqlite with bundled SQLite (Cursor), mdns-sd and if-addrs (Nearby discovery), x25519-dalek, chacha20poly1305 and sha2 (Nearby pairing and encryption), and unicode-normalization (Claude Code's folder names).

The demo above was recorded with [scrolltape](https://github.com/ahkamboh/scrolltape) — its cursor
and its ffmpeg pipeline, driven along a scripted path by [`scripts/record-demo.mjs`](scripts/record-demo.mjs)
against `ui --demo`, because a three-column app with no page scrolling isn't something an automatic
site tour can walk. The voice-over is [Kokoro](https://github.com/thewh1teagle/kokoro-onnx), run
offline by [`scripts/narrate.py`](scripts/narrate.py); its lines double as the subtitles in the strip
under the app. The ring that points at things, the key badges and the dragged row are drawn by the
recording script, not by Ferry.

The loading mascot is the Ferry logo come alive: one head per account, joined by
the bar, with a chat that rides across while a copy or move runs. It is pixel art in the style of
[mascot-maker](https://github.com/ahkamboh/mascot-maker), drawn as SVG in the theme's colours.

## Contributing

Issues and pull requests welcome. If you hit a layout Ferry doesn't understand — a different Claude Code version, an org setup it misreads — open an issue with the shape of your `claude-code-sessions` folder (no file contents needed).

## Licence

[MIT](LICENSE) © Ali Hamza Kamboh
