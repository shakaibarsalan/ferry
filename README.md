<div align="center">

<img src="assets/logo-160.png" width="112" alt="Ferry logo">

<h1>
  Ferry
  <a href="https://github.com/ahkamboh/ferry/releases/latest" title="Download the latest release">
    <img src="assets/download.png" width="24" alt="Download">
  </a>
</h1>

**Copy or move a chat from a Claude Code account you're signed out of into the one you're using now.**

Claude picks up the old context instead of you explaining the project again — all on your machine. A 5 MB native app for macOS and Windows, plus a zero-dependency CLI.

[![Download](https://img.shields.io/badge/%E2%86%93%20Download-macOS%20%7C%20Windows-d97757?style=for-the-badge)](https://github.com/ahkamboh/ferry/releases/latest)
[![Website](https://img.shields.io/badge/Website-ahkamboh.github.io%2Fferry-1f1e1d?style=for-the-badge)](https://ahkamboh.github.io/ferry/)

[![License: MIT](https://img.shields.io/badge/License-MIT-d97757.svg)](LICENSE)
![Platform: macOS and Windows](https://img.shields.io/badge/platform-macOS%2011%2B%20%7C%20Windows%2010%2B-1f1e1d)
![Size: 5 MB](https://img.shields.io/badge/app-5%20MB-1f1e1d)
![Built with Tauri 2](https://img.shields.io/badge/built%20with-Tauri%202%20%2B%20Rust-1f1e1d)

https://github.com/user-attachments/assets/2b1b0864-71e7-43c3-b78e-b5f65c6dde21

<sub>A chat left behind in an account you're signed out of, carried into the one you're using now — then a deleted chat restored from the archive.</sub>

</div>

---

## The problem

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
| **Find CLI and VS Code chats** | sessions you ran with `claude` or in the editor that no account lists at all — read them, and add one to whichever account you like |
| **Find Cursor chats** | conversations from Cursor, converted into Claude chats in the folder they were worked in (Windows) |
| **Identify them** | email for the account you're signed into; connectors, date range and project folders for the rest. Nickname any account and it sticks |
| **Fix a chat's folder** | a chat you started without picking one shows under **No folder** in Claude — point it at the folder it really belongs to, and Claude names it there |
| **Read any chat** | full conversation with proper Markdown — tables, code blocks, lists, quotes — plus tool calls |
| **Copy or move** | drag a chat onto another account, or use the Copy / Move buttons |
| **Download** | save a whole conversation as Markdown, plain text or JSON, anywhere you choose |
| **Delete and undelete** | deletes are reversible; restore from another account or from the archive |
| **Back up** | one button archives every chat and every transcript, subagent transcripts included |
| **Zoom** | Ctrl/Cmd + and − (or Ctrl/Cmd + scroll), Ctrl/Cmd 0 to reset; the level is remembered |

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
  >_  Claude Code CLI      6 chats · no account yet
  VS  VS Code              7 chats · no account yet
  Cu  Cursor              11 chats · no account yet
```

Open one and it reads like any other chat. **Add to account** then writes the record it never had, and from that moment Claude lists it, and Ferry can copy, move, rename, archive and delete it like the rest. Nothing is written back into the transcript, so the session stays resumable from where it came.

Two details worth knowing:

- The new record's id comes from the session's own id, so importing the same chat twice updates one record instead of making a second.
- Only the account's environment fields are inherited, copied from a record the app itself wrote there. Connector settings are not, for the same reason they're stripped on copy.

## The folder a chat belongs to

Claude names a chat's folder in its header and resumes the chat there. Start one without picking a folder and it runs in a workspace the app invents for it — `…\Claude\scratch-workspaces\…` — which is why the header reads **No folder**, and why the chat is filed nowhere useful afterwards.

Ferry shows that folder next to every chat and lets you set it. Pick the folder it really belongs to and Claude names it from then on.

There's a subtlety worth knowing, because it's what makes this safe. A chat's folder is two things at once: the folder Claude shows, **and** where the conversation is looked up — the transcript lives under `~/.claude/projects/<encoded folder>/`. Change the folder alone and Claude would show the new one and lose the conversation with it. So Ferry also makes the transcript findable under the new folder, by **hard-linking** it: one file, two names, not a byte duplicated, and the old folder keeps working. Only a volume that refuses links falls back to a copy, and Ferry tells you which happened.

Every change snapshots the record first, so setting a folder is as reversible as everything else here.

## Install

### Download the app

From [Releases](https://github.com/ahkamboh/ferry/releases/latest):

| Platform | File | Notes |
|---|---|---|
| **macOS 11+** | `Ferry-<version>-macos-universal.zip` | universal — Apple Silicon and Intel |
| **Windows 10+** | `Ferry.exe` | single file, no installer, needs the WebView2 runtime |

Neither build is code-signed, so both operating systems will warn you once.

**macOS:** right-click the app → **Open** → **Open**. Or:

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
build** workflow in Actions produces both files as well.

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
python3 ferry-cli.py cursor-import 4191e56f work@       # convert one to Claude
python3 ferry-cli.py ui                       # serve the app UI at localhost:7777
python3 ferry-cli.py ui --demo                # the same UI on synthetic data
```

`list` prints the CLI and VS Code chats under the accounts, with a short id for each. `import` takes that id (or a piece of the title) and an account — its address, its nickname, or the start of its uuid — and gives the chat a record there.

`export` matches on chat title or session id. If a title matches more than one chat it lists
the candidates with their ids instead of guessing. Files go to the folder you last downloaded
to, `~/Downloads` until you pick another.

`ui` serves the **same interface as the desktop app** — the app's `dist/index.html` with a shim
that turns its `invoke()` calls into HTTP. Markdown rendering, drag-and-drop and the archive all
work there; the differences are that the browser can't open a native save panel, so downloads go
straight to your chosen folder, and zoom is left to the browser's own Ctrl/Cmd + and −.

`ui --demo` runs that same interface against a synthetic Claude tree in a temp folder: three
invented accounts, eight chats, a pruned transcript and a deleted one. Nothing is stubbed — copy,
move, delete, restore, download and the archive all execute the real code, just rooted somewhere
else, and your own chats are never read. It's how the screenshot and the demo above were recorded,
and it's the quickest way to try Ferry without a Claude install.

Run `vault` from a launchd job or cron and your history is backed up nightly without opening anything.

## Cursor

Cursor keeps its chats nothing like Claude Code does: not a folder of transcripts but a single SQLite file — one row per conversation in `composerHeaders`, an ordered list of bubble ids beside it, and one row per message, all inside `globalStorage\state.vscdb`. So a chat can't be *moved* between them. It has to be converted, and Ferry converts **one way only**.

```bash
python3 ferry-cli.py cursor                         # what Cursor has
python3 ferry-cli.py cursor-import 4191e56f work@   # convert one into a Claude chat
```

`cursor` lists the conversations that actually contain something — most headers are empty shells left by windows that were opened and closed — grouped by the folder each was worked in, because Cursor's workspaces map to real directories. The converted chat lands in the account you name, in that folder, and Claude reads it like any other.

What survives the crossing: every prompt and reply, in order, with their timestamps, and every tool call as a line naming the tool and its path or command. What doesn't: Cursor's diffs, thinking blocks and attached code chunks have no equivalent in Claude's format. A 4,563-bubble conversation converts to 241 turns in about half a second — only a couple of hundred bubbles hold prose, and three thousand are tool calls folded into the message before them.

Two deliberate limits:

- **Nothing is ever written into Cursor.** Its database is opened `mode=ro` and only read. Going the other way would mean inserting rows into a live gigabyte file Cursor holds open, where one mistake costs every conversation in it.
- **This is the one place Ferry writes a transcript** rather than only the small record beside it, because there is no transcript to point at — Cursor's conversations live in a database. The file is named after the Cursor conversation, so converting the same chat twice rewrites the one file instead of leaving a second copy.

The app shows Cursor beside the CLI and VS Code, so a conversation drags onto an account like any other chat. Reading SQLite from Rust means bundling it, which is what took Ferry from 4.3 MB to 5.3 MB — the one dependency here that costs anything. The CLI gets it free from Python's standard library.

## The archive

`Back up` copies every chat record and every transcript into `~/.ferry`.

```
~/.ferry/
  chats/<timestamp>/<account>/   chat records, one snapshot per run
  projects/                      every transcript, subagents included
  snapshots/                     an automatic copy before each change
  labels.json                    your account nicknames
  prefs.json                     last folder you downloaded to
  sessions.json                  what each CLI/VS Code transcript says about itself
```

The archive is yours, outside anything Claude Code manages. Once a chat is in it, deletion becomes cosmetic — restore it into whichever account you want.

## Safety

- **Writes are refused while the Claude app is running.** Quit Claude first; the title bar tells you when editing is off.
- **Every change is snapshotted** into `~/.ferry/snapshots/` before it happens.
- **Transcripts are never moved or edited.** Only the small metadata record moves. Importing a CLI or VS Code chat writes one; it never writes back into the transcript, and a chat that has no record yet cannot be renamed, moved or deleted. Setting a chat's folder gives its transcript a second name by hard link — the same file, still in the folder it came from, with nothing rewritten.
- **Connector settings are stripped on copy.** MCP connector IDs belong to the account that created them and don't resolve elsewhere.
- **Deleting writes a tombstone**, the same marker Claude Code uses. The conversation stays on disk.

## FAQ

**Where does Claude Code store chat history on macOS?**
Two places. Metadata per account in `~/Library/Application Support/Claude/claude-code-sessions/`, and the conversations themselves as JSONL in `~/.claude/projects/`.

**I switched Claude accounts and my chats disappeared. Are they gone?**
No. The transcripts are still on disk — only the per-account metadata changed. Ferry lists every account it finds and can copy a chat into the one you're using now.

**Can I recover a deleted Claude Code chat?**
Usually. Deletion writes a small tombstone rather than erasing the conversation, so if the transcript hasn't been pruned, Ferry can restore it from another account or from `~/.ferry`.

**The Claude app doesn't list the chats I ran in the terminal or in VS Code. Where are they?**
On disk, in `~/.claude/projects`, same as every other chat. What they don't have is the per-account record the desktop app writes, and that record is the only thing the app lists from. Ferry shows them under **not in an account**; **Add to account** writes that record, and Claude lists the chat from then on.

**Does importing a CLI chat take it away from the CLI?**
No. Only the small record is written, and the record is new — the transcript is not touched, so `claude --resume` still offers the session in the folder it ran in.

**Why does a chat show a warning dot?**
Its transcript was pruned by Claude Code's cleanup. The record survives, the conversation doesn't. Backing up prevents this.

**Does this send anything anywhere?**
No. Ferry has no network code. It reads and writes local files only.

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
empty. Ferry checks both and uses the one with the newest chats. If it still comes back empty,
the window lists every path it looked at; include that in an issue.

**Is it affiliated with Anthropic?**
No. Ferry is an independent tool that reads local files written by Claude Code.

## How it's built

Rust + [Tauri 2](https://tauri.app) with a plain HTML/CSS/JS front end — no framework, no npm, no build step for the UI. The Markdown renderer is about 60 lines, written for this app, so nothing is fetched at runtime.

The demo above was recorded with [scrolltape](https://github.com/ahkamboh/scrolltape) — its cursor
and its ffmpeg pipeline, driven along a scripted path by [`scripts/record-demo.mjs`](scripts/record-demo.mjs),
because a three-column app with no page scrolling isn't something an automatic site tour can walk.

The loading mascot is the Ferry logo come alive: one head per account, joined by
the bar, with a chat that rides across while a copy or move runs. It is pixel art in the style of
[mascot-maker](https://github.com/ahkamboh/mascot-maker), drawn as SVG in the theme's colours.

## Contributing

Issues and pull requests welcome. If you hit a layout Ferry doesn't understand — a different Claude Code version, an org setup it misreads — open an issue with the shape of your `claude-code-sessions` folder (no file contents needed).

## Licence

[MIT](LICENSE) © Ali Hamza Kamboh
