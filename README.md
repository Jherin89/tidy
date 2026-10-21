<h1 align="center">tidy</h1>

<p align="center">
  <b>Clean up a messy Downloads folder in one command, without risking your files.</b><br>
  A safe, rules-driven file organizer for macOS (Linux and Windows too), written in Rust.<br>
  Dry-run by default · Trash, never delete · Never overwrites · One-command undo
</p>

<p align="center">
  <a href="https://github.com/Jherin89/tidy/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Jherin89/tidy/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/Jherin89/tidy/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/Jherin89/tidy?sort=semver"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Rust 1.85+" src="https://img.shields.io/badge/rust-1.85%2B-orange.svg">
  <img alt="macOS Apple Silicon" src="https://img.shields.io/badge/macOS-Apple%20Silicon-black.svg">
</p>

<p align="center">
  <img src="docs/demo.gif" alt="tidy demo: preview the plan, apply it, then undo it" width="100%">
</p>

```bash
tidy            # preview what would happen to ~/Downloads (changes nothing)
tidy --apply    # do it
tidy undo       # changed your mind? put everything back
```

---

## Contents

- [Why tidy?](#why-tidy)
- [Features](#features)
- [Safety](#safety)
- [Setup guide (step by step)](#setup-guide-step-by-step)
- [Usage guide (step by step)](#usage-guide-step-by-step)
- [Writing rules](#writing-rules)
- [Built-in rules](#built-in-rules)
- [Where tidy keeps its files](#where-tidy-keeps-its-files)
- [Command reference](#command-reference)
- [FAQ and troubleshooting](#faq-and-troubleshooting)
- [Contributing](#contributing)
- [License](#license)

## Why tidy?

Downloads folders fill up with invoices, screenshots, HEIC photos from your phone, installers you ran once, and zip files you already unpacked. Cleaning by hand is tedious, and most "auto-organizer" apps move things without telling you first or delete files you wanted to keep.

tidy works the other way around:

1. **You see the plan first.** A plain `tidy` prints a table of every file, the rule it matched, and where it would go. Nothing moves until you say `--apply`.
2. **Your rules, in plain text.** "PDFs go to `~/Documents/PDFs`", "installers older than 30 days go to the Trash". Rules live in one small TOML file you can read and edit.
3. **Mistakes are cheap.** Every run is logged, and `tidy undo` reverses it.

## Features

- **Preview table by default.** Shows file, matched rule, action, destination, and size, plus totals per rule and per action.
- **Rules that match on extension, name pattern, age, and size**, combined however you like. The first matching rule wins.
- **Two actions:** move to any folder (created when needed, with `~` expansion and optional `YYYY-MM` subfolders), or send to the system **Trash**.
- **Sensible built-in rules**, so it works with no setup. Run `tidy init` to get an editable copy.
- **Undo log** for every applied run, and `tidy undo` to reverse the latest one.
- **Watch mode:** `tidy watch` sorts new downloads as soon as they finish.
- **Skips files that aren't finished or aren't yours to move:** partial downloads (`.crdownload`, `.part`, `.download`, `.tmp`, `.partial`), hidden files like `.DS_Store`, folders, and symlinks.
- **`--json` output** for scripts and automations.
- **A single small binary.** No runtime and no background service unless you set one up.

<p align="center">
  <img src="docs/dry-run.png" alt="tidy dry-run preview table" width="100%">
  <br><sub>A real dry run of the built-in rules on a sample Downloads folder.</sub>
</p>

## Safety

tidy is built so that the worst outcome of a bad rule is a few files in the wrong folder, and `tidy undo` fixes that.

| Guarantee | What it means |
| --- | --- |
| **Dry run is the default** | Without `--apply`, nothing is moved or trashed, and no folders are created. The preview ends with `Dry run. Nothing was changed.` |
| **Trash, not delete** | The only removal action is `trash`, which uses the operating system's Trash through the [`trash`](https://crates.io/crates/trash) crate. tidy has no permanent-delete action, and a rule that asks for `delete` is rejected. |
| **Never overwrites** | If `invoice.pdf` already exists at the destination, the new one becomes `invoice (1).pdf`, then `invoice (2).pdf`, and so on. Existing files are never replaced. |
| **Undo** | Every `--apply` (and every watch batch) writes a JSON log. `tidy undo` moves files back to where they were and removes the now-empty folders the run created. Folders that held something before the run, or that you've put something in since, are left alone. |
| **Careful cross-disk moves** | When source and destination are on different disks, tidy copies, checks the size, and only then removes the original. |
| **Leaves partial files alone** | In-progress downloads, hidden files, folders, and symlinks are never touched. |

> [!IMPORTANT]
> **macOS Trash caveat.** On macOS, apps can put files *into* the Trash but there is no supported way to take them back *out* programmatically. So on a Mac, `tidy undo` moves every *moved* file back automatically, but for *trashed* files it prints each file name and its original location so you can drag them back from the Trash (or select them in Finder and choose **File → Put Back**). On Linux and Windows, trashed files are restored automatically.

## Setup guide (step by step)

You don't need to know Rust to use tidy. Pick **Option A** (download the app, about 2 minutes) or **Option B** (build it yourself).

### Option A: download the prebuilt Mac binary (Apple Silicon: M1/M2/M3/M4)

1. **Check your Mac's chip.** Click the Apple menu → **About This Mac**. If it says *Apple M1/M2/M3/M4*, continue. (Intel Mac? Use Option B.)
2. **Download** `tidy-0.1.0-aarch64-apple-darwin.tar.gz` from the [latest release](https://github.com/Jherin89/tidy/releases/latest). Your browser saves it to `~/Downloads`.
3. **Open Terminal** (press <kbd>⌘</kbd> <kbd>Space</kbd>, type *Terminal*, press <kbd>Return</kbd>).
4. **Optional: check the download.** Compare this output with the `.sha256` file on the release page:
   ```bash
   cd ~/Downloads
   shasum -a 256 tidy-0.1.0-aarch64-apple-darwin.tar.gz
   ```
5. **Unpack it:**
   ```bash
   tar -xzf tidy-0.1.0-aarch64-apple-darwin.tar.gz
   ```
6. **Remove the download quarantine flag.** The binary isn't notarized by Apple, so without this step macOS says it "cannot be opened because the developer cannot be verified":
   ```bash
   xattr -d com.apple.quarantine tidy-0.1.0-aarch64-apple-darwin/tidy
   ```
   If it says `No such xattr`, the flag wasn't set and you can move on.
7. **Put `tidy` on your PATH.** Choose one:
   - **For your user only (no password needed):**
     ```bash
     mkdir -p ~/.local/bin
     mv tidy-0.1.0-aarch64-apple-darwin/tidy ~/.local/bin/
     echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
     source ~/.zshrc
     ```
   - **System-wide (asks for your password):**
     ```bash
     sudo mkdir -p /usr/local/bin
     sudo mv tidy-0.1.0-aarch64-apple-darwin/tidy /usr/local/bin/
     ```
8. **Verify it works:**
   ```bash
   tidy --version     # → tidy 0.1.0
   tidy               # safe: only shows a preview of ~/Downloads
   ```
9. **Clean up the installer files** (optional): `rm -r ~/Downloads/tidy-0.1.0-aarch64-apple-darwin*`

### Option B: build from source with Rust (any Mac, Linux, or Windows)

1. **Install Rust** with the official installer, [rustup](https://rustup.rs), and accept the defaults:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
   Then open a **new** Terminal window, or run `source "$HOME/.cargo/env"`.
2. **macOS only:** if you're asked to install the *Command Line Developer Tools*, click **Install** (or run `xcode-select --install`).
3. **Build and install tidy:**
   ```bash
   cargo install --git https://github.com/Jherin89/tidy
   ```
   This takes a minute or two and puts `tidy` in `~/.cargo/bin`, which rustup already added to your PATH.
4. **Verify:** `tidy --version`

> **Note on the name:** the command is `tidy`, but the Cargo *package* is named **`tidy-dl`**, because `tidy` on crates.io belongs to an unrelated project. Install from this repository as shown above. `cargo install tidy` would install that other project.

**Updating:** download the newer release (Option A), or run `cargo install --git https://github.com/Jherin89/tidy --force` (Option B).
**Uninstalling:** delete the `tidy` binary (`which tidy` shows where it is). You can also remove the config and log folders listed in [Where tidy keeps its files](#where-tidy-keeps-its-files).

## Usage guide (step by step)

### 1. Preview your first clean-up

```bash
tidy
```

With no folder argument, tidy looks at `~/Downloads`. It **only prints a plan**. Nothing changes yet.

### 2. Read the table

An excerpt of the run in the screenshot above:

```text
file                                     rule            action  destination                                                             size
───────────────────────────────────────  ──────────────  ──────  ──────────────────────────────────────────────────────────────────────  ──────
IMG_4821.HEIC                            Images          move    ~/Pictures/Downloads/IMG_4821.HEIC                                      2.4 MB
invoice-2026-03.pdf                      PDFs            move    ~/Documents/PDFs/invoice-2026-03.pdf                                    188 KB
Q3-pipeline-forecast.xlsx                Spreadsheets    move    ~/Documents/Spreadsheets/Q3-pipeline-forecast.xlsx                      349 KB
Screenshot 2026-10-01 at 9.14.22 AM.png  Screenshots     move    ~/Pictures/Screenshots/2026-10/Screenshot 2026-10-01 at 9.14.22 AM.png  500 KB
zoom-installer.pkg                       Old installers  trash   Trash                                                                   49 MB

Summary
  matched      18   386 MB
  unmatched     3   left in place
  skipped       3
...
Unmatched  (left in place)
  Docker.dmg                627 MB
...
Skipped
  .DS_Store                   hidden
  old-projects/               directory
  ubuntu-24.04.iso.crdownload partial download

Dry run. Nothing was changed.
Re-run with --apply to perform these actions.
```

- **file**: a file in the folder (only the top level; subfolders are never entered).
- **rule**: the first rule that matched it.
- **action**: `move` (green) or `trash` (yellow).
- **destination**: where it will go. `~` means your home folder. If a file with that name is already there, you'll see a ` (1)` suffix.
- **Unmatched** files match no rule and stay where they are. In the example, `Docker.dmg` is an installer but it's only 5 days old, so the 30-day "Old installers" rule doesn't apply yet.
- **Skipped** files are never touched: hidden files, folders, symlinks, and downloads still in progress.

### 3. Apply it

```bash
tidy --apply
```

```text
Applied 18 actions (386 MB).
Undo log  ~/Library/Application Support/tidy/logs/run-20261021T155316096822Z.json
Run `tidy undo` to reverse this run.
```

`--apply` carries out exactly what the preview showed. Folders like `~/Documents/PDFs` are created as needed.

### 4. Undo it

```bash
tidy undo
```

<p align="center"><img src="docs/undo.png" alt="tidy undo output" width="100%"></p>

`tidy undo` reverses the most recent run that hasn't been undone yet. Run it again to step back through earlier runs. On a Mac, see the [Trash caveat](#safety) for trashed files.

### 5. Make the rules your own

```bash
tidy init      # writes the built-in rules to a commented rules.toml
tidy rules     # shows the active rules and which file they came from
```

`tidy init` prints the path it wrote (on a Mac: `~/.config/tidy/rules.toml`). Open it in any editor:

```bash
open -e ~/.config/tidy/rules.toml     # TextEdit
# or: nano ~/.config/tidy/rules.toml
```

Edit, save, and run `tidy` again to preview the effect. See [Writing rules](#writing-rules) for examples. If you make a typo, tidy refuses to run and shows the line with the problem.

### 6. Tidy a different folder

```bash
tidy ~/Desktop
tidy ~/Desktop --apply
tidy "/Volumes/USB Stick/Inbox"
```

Relative destinations (like `destination = "Archives"`) are created inside whichever folder you are tidying.

### 7. Only touch older files

```bash
tidy --older-than 7d          # ignore anything modified in the last 7 days
tidy --older-than 2w --apply
```

Units are `s`, `m` (minutes), `h`, `d`, `w`.

### 8. Use it from scripts: `--json`

```bash
tidy --json | jq '.summary'
tidy --json | jq -r '.entries[] | "\(.rule)\t\(.file)"'
tidy --apply --json > last-run.json
```

The output is one JSON document with `mode` (`dry-run` or `applied`), `entries` (file, path, size, rule, action, destination, status), `unmatched`, `skipped`, `summary`, `log_path`, and `errors`. `tidy rules --json` and `tidy undo --json` work too. Errors go to stderr with a `tidy:` prefix and a nonzero exit code.

### 9. Sort new downloads automatically: `tidy watch`

```bash
tidy watch               # watches ~/Downloads; press Ctrl-C to stop
tidy watch ~/Desktop
```

Watch mode waits until a new file's size has stopped changing for about 2 seconds, then applies your rules to it. It ignores `.crdownload`/`.part` files and picks up the finished file once the browser renames it. Watch mode **applies immediately** (that's its job), but every batch is still logged, so `tidy undo` works. It only reacts to new or changed files. To clean up what's already there, run `tidy --apply` once.

#### Keep it running in the background on macOS (launchd)

1. Find the full path of the binary: `which tidy` (for example `/Users/you/.local/bin/tidy`).
2. Save this as `~/Library/LaunchAgents/com.github.jherin89.tidy.plist`, replacing both `/Users/you` paths:
   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <key>Label</key>             <string>com.github.jherin89.tidy</string>
     <key>ProgramArguments</key>
     <array>
       <string>/Users/you/.local/bin/tidy</string>
       <string>watch</string>
     </array>
     <key>RunAtLoad</key>         <true/>
     <key>KeepAlive</key>         <true/>
     <key>StandardOutPath</key>   <string>/Users/you/Library/Logs/tidy.log</string>
     <key>StandardErrorPath</key> <string>/Users/you/Library/Logs/tidy.log</string>
   </dict>
   </plist>
   ```
3. Load it (it also starts at every login):
   ```bash
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.github.jherin89.tidy.plist
   ```
4. Check on it with `tail -f ~/Library/Logs/tidy.log`. Stop it with:
   ```bash
   launchctl bootout gui/$(id -u)/com.github.jherin89.tidy
   ```

If the log shows `Operation not permitted`, macOS is blocking background access to Downloads. Open **System Settings → Privacy & Security → Full Disk Access**, click **+**, press <kbd>⌘</kbd><kbd>⇧</kbd><kbd>G</kbd>, paste the path from step 1, and add it.

**Linux:** run `tidy watch` from a systemd user service with `Restart=on-failure` instead.

## Writing rules

Rules live in `rules.toml` as a list of `[[rules]]` blocks. tidy checks each file against the rules **from top to bottom, and the first match wins.** A file that matches nothing stays where it is.

```toml
# 1. Match by extension (case-insensitive), move by month
[[rules]]
name = "Screenshots"
glob = ["Screenshot*", "Screen Shot*"]      # file-name pattern(s); any one may match
extensions = ["png", "jpg"]                   # AND it must be one of these types
action = "move"
destination = "~/Pictures/Screenshots"
by_date = true                                # → ~/Pictures/Screenshots/2026-10/

# 2. Match by age: installers you ran more than 30 days ago go to the Trash
[[rules]]
name = "Old installers"
extensions = ["dmg", "pkg"]
older_than = "30d"                            # s, m (minutes), h, d, w
action = "trash"

# 3. Match by name pattern: anything that looks like an invoice or receipt
[[rules]]
name = "Invoices"
glob = ["invoice*", "*receipt*"]
extensions = ["pdf"]
action = "move"
destination = "~/Documents/Finance/Invoices"

# 4. Match by size: big videos go to an external drive
[[rules]]
name = "Huge videos"
extensions = ["mp4", "mov", "mkv"]
larger_than = "1GB"                           # KB/MB/GB (1000s) or KiB/MiB/GiB (1024s)
action = "move"
destination = "/Volumes/Archive/Videos"

# 5. Combine everything: small, old zip files → Trash
[[rules]]
name = "Stale small zips"
extensions = ["zip"]
older_than = "60d"
smaller_than = "5MB"
action = "trash"

# 6. Relative destination = a subfolder of the folder being tidied
[[rules]]
name = "Archives"
extensions = ["zip", "tar.gz", "7z"]
action = "move"
destination = "Archives"                      # → ~/Downloads/Archives/
```

Put specific rules (like *Invoices*) **above** general ones (like *PDFs*), or the general rule will match first.

| Condition | Meaning | Example |
| --- | --- | --- |
| `extensions` | File type, case-insensitive. A list or a single string. Multi-part types like `tar.gz` work. | `["pdf"]`, `"heic"` |
| `glob` | File-name pattern, case-insensitive. `*` matches anything, `?` matches one character. | `["Screenshot*", "*receipt*"]` |
| `older_than` | Last modified at least this long ago. | `"30d"`, `"12h"`, `"2w"` |
| `larger_than` | At least this big. | `"100MB"`, `"1GiB"` |
| `smaller_than` | At most this big. | `"500KB"` |

Within one condition, any value may match (OR). Different conditions must all match (AND). Every rule needs at least one condition.

| Action | Fields | Effect |
| --- | --- | --- |
| `move` | `destination` (required), `by_date` (optional) | Moves the file. `~` is your home folder. Missing folders are created. Existing names get ` (1)`, ` (2)`… |
| `trash` | none | Sends the file to the system Trash. |

## Built-in rules

These rules apply until you create a rules file. `tidy init` writes them out so you can edit them.

| # | Rule | Matches | Action |
| --- | --- | --- | --- |
| 1 | Screenshots | `Screenshot*` / `Screen Shot*` (png, jpg, jpeg, webp) | move → `~/Pictures/Screenshots/YYYY-MM/` |
| 2 | Old installers | dmg, pkg, exe, msi, deb, rpm, AppImage, iso, **older than 30 days** | **Trash** |
| 3 | PDFs | pdf | move → `~/Documents/PDFs` |
| 4 | Images | jpg, jpeg, png, gif, webp, heic, heif, tif, tiff, bmp, svg | move → `~/Pictures/Downloads` |
| 5 | Videos | mp4, mov, mkv, avi, webm, m4v, wmv | move → `~/Movies/Downloads` |
| 6 | Audio | mp3, wav, flac, aac, m4a, ogg, wma, aiff | move → `~/Music/Downloads` |
| 7 | Spreadsheets | xls, xlsx, csv, ods, numbers, tsv | move → `~/Documents/Spreadsheets` |
| 8 | Documents | doc, docx, txt, rtf, odt, md, pages, ppt, pptx, key, odp | move → `~/Documents/Downloads` |
| 9 | Old archives | zip, tar, gz, tgz, bz2, xz, 7z, rar, tar.gz, tar.bz2, tar.xz, **older than 14 days** | move → `~/Downloads/Archives` |
| 10 | Ebooks | epub, mobi, azw, azw3 | move → `~/Documents/Books` |

Recent installers and archives are left alone on purpose, because you may still need them.

## Where tidy keeps its files

| What | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Rules file | `~/.config/tidy/rules.toml` (also reads `~/Library/Application Support/tidy/rules.toml`) | `~/.config/tidy/rules.toml` (`$XDG_CONFIG_HOME`) | `%APPDATA%\tidy\rules.toml` |
| Undo logs | `~/Library/Application Support/tidy/logs/` | `~/.local/share/tidy/logs/` | `%APPDATA%\tidy\logs\` |

On macOS, tidy prefers `~/.config/tidy/rules.toml` if it exists, otherwise `~/Library/Application Support/tidy/rules.toml`. `--rules FILE` overrides both. `tidy rules` always shows which file is in use.

Undo logs are plain JSON, one file per run (`run-<timestamp>.json`), listing each original path, the action, and the final destination.

<details>
<summary><b>Environment variables</b> (mostly for testing and demos)</summary>

| Variable | Effect |
| --- | --- |
| `TIDY_HOME` | Home directory used for `~` and the default `~/Downloads`. |
| `TIDY_CONFIG_DIR` | Replaces the config directory. Rules are read from `$TIDY_CONFIG_DIR/tidy/rules.toml`. |
| `TIDY_DATA_DIR` | Replaces the data directory. Logs go to `$TIDY_DATA_DIR/tidy/logs/`. |
| `TIDY_FAKE_TRASH` | Moves "trashed" files into this folder instead of the real Trash. |
| `TIDY_WATCH_SETTLE_MS` | How long a watched file's size must stay unchanged (default `2000`). |
| `NO_COLOR` / `CLICOLOR_FORCE=1` | Turn color off / force it on even when piped. |

The demo GIF and screenshots were recorded with these variables pointing at a throwaway folder ([`docs/demo/`](docs/demo)), so no real files were touched.
</details>

## Command reference

| Command | What it does |
| --- | --- |
| `tidy [PATH]` | Preview `PATH` (default `~/Downloads`). Changes nothing. |
| `tidy --apply [PATH]` | Carry out the plan. |
| `tidy --json [PATH]` | Print the plan (or, with `--apply`, the result) as JSON. |
| `tidy --older-than 30d [PATH]` | Only consider files at least this old. |
| `tidy --rules FILE [PATH]` | Use a specific rules file. |
| `tidy init [--force]` | Write the built-in rules to your rules file. Won't overwrite without `--force`. |
| `tidy config --init` / `--show` | Same as `tidy init` / `tidy rules`. |
| `tidy rules [--json]` | Show the active rules and where they came from. |
| `tidy undo [--json]` | Reverse the most recent run that hasn't been undone. |
| `tidy watch [PATH]` | Watch a folder and sort new files as they finish. Ctrl-C to stop. |
| `tidy --help`, `tidy --version` | Help and version. |

Exit codes: `0` on success; `1` on errors (missing or invalid folder, bad rules file, a failed action, or an undo that couldn't put every file back).

## FAQ and troubleshooting

<details><summary><b>"tidy" cannot be opened because the developer cannot be verified</b></summary>

The binary isn't notarized. Run `xattr -d com.apple.quarantine "$(which tidy)"`, or go to **System Settings → Privacy & Security** and click **Open Anyway**.
</details>

<details><summary><b><code>zsh: command not found: tidy</code></b></summary>

The folder you put it in isn't on your PATH. For `~/.local/bin`: `echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc`. For a Cargo install, open a new Terminal window (or run `source "$HOME/.cargo/env"`).
</details>

<details><summary><b><code>bad CPU type in executable</code></b></summary>

You downloaded the Apple Silicon build on an Intel Mac. Use [Option B](#option-b-build-from-source-with-rust-any-mac-linux-or-windows).
</details>

<details><summary><b><code>Operation not permitted</code> when reading ~/Downloads</b></summary>

macOS privacy protection. Allow your terminal app (or the `tidy` binary, if it runs from launchd) under **System Settings → Privacy & Security → Files and Folders** or **Full Disk Access**.
</details>

<details><summary><b>Why wasn't a file moved?</b></summary>

Look at the preview. It's either **Unmatched** (no rule fits, often because of an `older_than` condition), or **Skipped** (hidden, a folder, a symlink, or a partial download). Also check rule order, since the first match wins.
</details>

<details><summary><b>Can tidy delete my files?</b></summary>

No. The only removal action is `trash`, and things in the Trash stay there until *you* empty it. Moves never overwrite.
</details>

<details><summary><b>I emptied the Trash. Can undo bring files back?</b></summary>

No. Once the Trash is emptied, the files are gone. That's why `trash` rules in the defaults only target old installers.
</details>

<details><summary><b>Does it look inside subfolders?</b></summary>

No, only files directly in the folder. Subfolders (including ones tidy created, like `Archives`) are never entered.
</details>

<details><summary><b>Why is the crate called <code>tidy-dl</code>?</b></summary>

The name `tidy` on crates.io is taken by an unrelated project. The package is `tidy-dl` and the command is still `tidy`. tidy isn't published to crates.io; install it from GitHub.
</details>

## Contributing

Issues and pull requests are welcome, especially new built-in rules, platform fixes, and docs.

```bash
git clone https://github.com/Jherin89/tidy && cd tidy
cargo build --release
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- The toolchain is pinned to Rust **1.85.1** in `rust-toolchain.toml`, and rustup picks it up automatically.
- Tests build a throwaway home, config, data, and Trash folder under a temp directory, so they never touch your real Downloads or Trash. Please keep it that way. A test also checks that no new code can delete files.
- CI runs fmt, clippy, and tests on every push and PR. Pushing a `v*` tag builds the macOS Apple Silicon binary and attaches it to a GitHub Release.
- To re-record the demo: `cargo build --release`, then `vhs docs/demo/demo.tape` (needs [vhs](https://github.com/charmbracelet/vhs)).

## License

[MIT](LICENSE) © 2026 Jordan Herin
