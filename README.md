# tidy

**tidy** is a free, open-source command-line tool that clears a messy Downloads folder using rules you can read and edit.

It is dry-run by default. A plain `tidy` prints a preview and changes nothing. Pass `--apply` when the plan looks right. Moves never overwrite a file that is already there, and anything sent away goes to the system Trash rather than being deleted.

macOS is the primary target (Jordan Herin, GitHub [Jherin89](https://github.com/Jherin89)). Linux and Windows work too. Trash restore is the one place they differ — see [Safety](#safety).

![tidy dry-run demo](docs/demo.gif)

*Placeholder. Replace `docs/demo.gif` with a recording of a real dry run.*

## Install

From source, with a recent stable Rust toolchain:

```bash
cargo install --git https://github.com/Jherin89/tidy
```

That builds the `tidy` binary. The Cargo package is named `tidy-dl` because the crates.io name `tidy` is already taken by a different project. Install from this repository. Do not `cargo install tidy` from crates.io.

Prebuilt binaries for macOS Apple Silicon are attached to [GitHub Releases](https://github.com/Jherin89/tidy/releases). Each tag `v*` publishes `tidy-<version>-aarch64-apple-darwin.tar.gz` and a `.sha256` checksum.

```bash
tar -xzf tidy-*-aarch64-apple-darwin.tar.gz
sudo install tidy-*/tidy /usr/local/bin/tidy
tidy --version
```

## Quick start

```bash
tidy                  # preview ~/Downloads
tidy --apply          # perform the preview
tidy undo             # reverse the most recent run
tidy init             # write an editable rules file
tidy rules            # show the rules that are active
tidy watch            # apply rules to new files as they settle
tidy ~/some/folder    # tidy a different folder
```

Only the top level of the folder is considered. Hidden files, directories, symlinks, and partial downloads (`.crdownload`, `.part`, `.download`, `.tmp`, `.partial`) are skipped.

## Example

This is real output from the built-in rules (color omitted):

```text
Folder  ~/Downloads
Rules   built-in defaults

file                       rule            action  destination                                               size
─────────────────────────  ──────────────  ──────  ────────────────────────────────────────────────────────  ────
budget.csv                 Spreadsheets    move    ~/Documents/Spreadsheets/budget.csv                       3 B
notes.txt                  Documents       move    ~/Documents/Downloads/notes.txt                           5 B
old-photos.zip             Old archives    move    ~/Downloads/Archives/old-photos.zip                       3 B
photo.png                  Images          move    ~/Pictures/Downloads/photo.png                            3 B
quarterly-report.pdf       PDFs            move    ~/Documents/PDFs/quarterly-report.pdf                     3 B
Screenshot 2026-10-01.png  Screenshots     move    ~/Pictures/Screenshots/2026-10/Screenshot 2026-10-01.png  4 B
Setup.dmg                  Old installers  trash   Trash                                                     3 B
song.mp3                   Audio           move    ~/Music/Downloads/song.mp3                                3 B

Summary
  matched       8   27 B
  unmatched     1   left in place
  skipped       3

By action
  move        7   24 B
  trash       1   3 B

By rule
  Audio              1   3 B
  Documents          1   5 B
  Images             1   3 B
  Old archives       1   3 B
  Old installers     1   3 B
  PDFs               1   3 B
  Screenshots        1   4 B
  Spreadsheets       1   3 B

Unmatched  (left in place)
  mystery.bin              3 B

Skipped
  .secret.pdf              hidden
  Album/                   directory
  movie.mp4.part           partial download

Dry run. Nothing was changed.
Re-run with --apply to perform these actions.
```

A recent installer and a recent archive do not match the “old” rules, so they stay put. `mystery.bin` matches nothing, so it is reported and left alone.

## Commands

| Command | What it does |
| --- | --- |
| `tidy [PATH]` | Preview `PATH`, or `~/Downloads` when `PATH` is omitted. |
| `tidy --apply [PATH]` | Perform the preview. |
| `tidy --json [PATH]` | Print the plan, or the applied result, as JSON. |
| `tidy --rules FILE` | Use `FILE` instead of the configured rules. |
| `tidy --older-than 30d` | Ignore files newer than that. Applied before the rules. |
| `tidy init [--force]` | Write the built-in rules. Refuses to overwrite unless `--force`. |
| `tidy config --init` | Same as `tidy init`. `--show` prints the active rules. |
| `tidy rules` | Print the active rules and the file they came from. |
| `tidy undo` | Reverse the newest run that has not already been undone. |
| `tidy watch [PATH]` | Watch the folder and apply rules to new files. Implies `--apply`. |

`--json` and `--rules` work with the subcommands too (`tidy rules --json`, `tidy undo --json`).

## Rules

Rules live in a TOML file. `tidy init` writes a commented copy of the defaults. The first rule whose conditions all match wins. A file that matches nothing is left where it is.

Every condition you set must match. Omit a condition to ignore it. A rule needs at least one condition, so an empty rule cannot accidentally trash the whole folder.

Within `extensions` or `glob`, any one value is enough (OR). Different conditions combine with AND. Size and age comparisons are inclusive.

```toml
[[rules]]
name = "Screenshots"
glob = ["Screenshot*", "Screen Shot*"]
extensions = ["png", "jpg", "jpeg", "webp"]
action = "move"
destination = "~/Pictures/Screenshots"
by_date = true

[[rules]]
name = "Old installers"
extensions = ["dmg", "pkg", "exe", "msi", "deb", "rpm", "AppImage", "iso"]
older_than = "30d"
action = "trash"

[[rules]]
name = "Big PDFs"
extensions = ["pdf"]
larger_than = "20MB"
smaller_than = "500MB"
action = "move"
destination = "~/Documents/PDFs"
```

### Conditions

| Key | Meaning |
| --- | --- |
| `extensions` | Case-insensitive list, or a single string. The extension is the text after the last dot, so `Report.PDF` matches `pdf`. A value that contains a dot, such as `tar.gz`, matches when the file name ends with it. |
| `glob` | File-name glob (globset syntax), case-insensitive. A list matches if any pattern matches. `Screenshot*` does not look at the directory. |
| `older_than` | Modified time is at least this old. Units: `s`, `m`, `h`, `d`, `w` (seconds, minutes, hours, days, weeks). `m` is minutes, not months. Examples: `30d`, `12h`, `2w`. |
| `larger_than` | Size is at least this big. Decimal units (`KB`, `MB`, `GB`, `TB`, and `K`/`M`/`G`/`T`) are powers of 1000. Binary units (`KiB`, `MiB`, `GiB`, `TiB`) are powers of 1024. Examples: `100MB`, `500KB`, `1GiB`. |
| `smaller_than` | Size is at most this big. Same units. |

The preview table prints sizes in decimal units (1000-based). A rule written as `1GiB` still compares against 1024³ bytes.

### Actions

`action = "move"` requires `destination`.

- `~/Documents/PDFs` expands `~` to the home directory (`TIDY_HOME` when that is set).
- A relative destination such as `Archives` is created inside the folder being tidied.
- `by_date = true` appends a `YYYY-MM` folder from the file’s modified time, in the local timezone.
- Missing directories are created only when you `--apply`. A dry run does not create them.

`action = "trash"` sends the file to the system Trash. `delete` is rejected on purpose: tidy does not permanently delete files.

If the destination name is already taken, tidy inserts ` (1)`, ` (2)`, … before the extension. `archive.tar.gz` becomes `archive (1).tar.gz`. The same suffix is used for `.tar.bz2`, `.tar.xz`, and `.tar.zst`.

### Built-in rules

Used when no rules file exists, in this order:

1. **Screenshots** — `Screenshot*` / `Screen Shot*` images → `~/Pictures/Screenshots/YYYY-MM`
2. **Old installers** — `dmg`, `pkg`, `exe`, `msi`, `deb`, `rpm`, `AppImage`, `iso`, older than 30 days → Trash
3. **PDFs** → `~/Documents/PDFs`
4. **Images** → `~/Pictures/Downloads`
5. **Videos** → `~/Movies/Downloads` (the macOS folder name; it is just a directory elsewhere)
6. **Audio** → `~/Music/Downloads`
7. **Spreadsheets** — `xls`, `xlsx`, `csv`, `ods`, `numbers`, `tsv` → `~/Documents/Spreadsheets`
8. **Documents** — office files, text, and slides → `~/Documents/Downloads`
9. **Old archives** — `zip`, `tar`, `gz`, `7z`, `rar`, and the other common archive extensions, older than 14 days → `~/Downloads/Archives`
10. **Ebooks** — `epub`, `mobi`, `azw`, `azw3` → `~/Documents/Books`

Recent installers and recent archives stay in the folder. Run `tidy init` to save this set and edit it.

### Where the rules file is

`--rules FILE` always wins. The file must exist.

Otherwise tidy looks here:

1. When `TIDY_CONFIG_DIR` is set: `$TIDY_CONFIG_DIR/tidy/rules.toml`. This skips the macOS preference below, because the variable is explicit.
2. On macOS, if `~/.config/tidy/rules.toml` exists, that file.
3. `<config dir>/tidy/rules.toml`, where `<config dir>` is `dirs::config_dir()`:
   - macOS: `~/Library/Application Support`
   - Linux: `$XDG_CONFIG_HOME` or `~/.config`
   - Windows: `%APPDATA%`

If that file does not exist, tidy uses the built-in rules and says so. `tidy rules` prints the source path, or the path `tidy init` would create.

`tidy init` on macOS writes `~/.config/tidy/rules.toml`, unless the Application Support file already exists and the XDG file does not. In that case it updates the file already being read. Elsewhere it writes `<config dir>/tidy/rules.toml`.

## Safety

tidy is safe by default.

- **Dry run.** Without `--apply`, nothing is moved, nothing is trashed, and destination folders are not created. The last lines of the preview tell you to re-run with `--apply`.
- **Trash, not delete.** `action = "trash"` calls the operating-system Trash through the [`trash`](https://crates.io/crates/trash) crate. There is no permanent-delete action. The only time tidy removes a file you already had is the source side of a cross-device move, and only after the copy’s length has been checked against the original.
- **Never overwrite.** If the destination name exists, or another file in the same run already claimed it, tidy picks `name (1).ext`, then `name (2).ext`, and so on. The file that was already there is left untouched. Undo uses the same rule when the original spot has been filled.
- **Undo.** Every applied run writes a JSON log. `tidy undo` reverses the newest log that has not been fully undone: moves are moved back, and trashed files are restored where the platform allows it. See below for macOS.
- **Watch.** `tidy watch` is the one mode that applies without a separate `--apply`, because that is the point of it. It still writes an undo log for each batch. Ctrl-C stops the watcher.

Tests and demos can set `TIDY_FAKE_TRASH` to a directory. Trashed files are moved there instead of the real Trash. The test suite also sets `TIDY_HOME`, `TIDY_CONFIG_DIR`, and `TIDY_DATA_DIR` so it never touches a real Downloads folder.

### Undo and the macOS Trash

On Linux and Windows, undo restores trashed files with `trash::os_limited::restore_all`. If the original name is now taken, the file stays in the Trash. tidy will not overwrite whatever is there, and it tells you to restore that file yourself.

On macOS the `trash` crate cannot restore programmatically. `tidy undo` still moves ordinary files back. For anything it trashed, it prints each file name and the original location and tells you to put it back from the Trash by hand. The log is marked undone so the same reminder is not repeated on the next `tidy undo`. The command exits with status 1 when any file was not actually put back.

`tidy undo` exits 0 when every file was moved back or restored. If a move cannot be reversed (the destination file is already gone, for example), that entry stays pending and a later `tidy undo` retries it.

### Moves across devices

tidy tries `rename` first. If the source and destination are on different devices, it copies the bytes, checks that the destination length matches the source, and only then removes the source. If the check fails, the source stays where it was.

## Watch mode

```bash
tidy watch
tidy watch ~/Downloads
TIDY_WATCH_SETTLE_MS=500 tidy watch /tmp/inbox
```

`notify-debouncer-mini` debounces filesystem events. A new top-level file is tidied only after its size has stayed the same for about 2 seconds (`TIDY_WATCH_SETTLE_MS` overrides that, in milliseconds). Partial-download extensions are ignored, so a `.crdownload` that is still growing is left alone. When the browser renames it to the final name, that new file is what gets matched.

Each batch is applied immediately and written to the undo log. Ctrl-C exits cleanly.

## Environment variables

| Variable | Effect |
| --- | --- |
| `TIDY_HOME` | Home directory used for `~/` and for the default `~/Downloads`. |
| `TIDY_CONFIG_DIR` | Replaces `dirs::config_dir()`. Rules are `$TIDY_CONFIG_DIR/tidy/rules.toml`. |
| `TIDY_DATA_DIR` | Replaces `dirs::data_dir()`. Undo logs are `$TIDY_DATA_DIR/tidy/logs/run-<timestamp>.json`. |
| `TIDY_FAKE_TRASH` | Move trashed files into this directory instead of the system Trash. For tests and demos. |
| `TIDY_WATCH_SETTLE_MS` | How long a watched file’s size must stay unchanged, in milliseconds. Default 2000. |
| `NO_COLOR` | Disable ANSI color. Color is also off when stdout is not a terminal. |

## JSON

`--json` prints a single JSON document to stdout. Errors still go to stderr with a `tidy:` prefix.

```json
{
  "version": 1,
  "mode": "dry-run",
  "folder": "/Users/jordan/Downloads",
  "rules_source": "built-in defaults",
  "rules_path": null,
  "entries": [
    {
      "file": "report.pdf",
      "path": "/Users/jordan/Downloads/report.pdf",
      "size": 1234,
      "rule": "PDFs",
      "action": "move",
      "destination": "/Users/jordan/Documents/PDFs/report.pdf",
      "status": "planned"
    }
  ],
  "unmatched": [],
  "skipped": [],
  "summary": {
    "matched": 1,
    "unmatched": 0,
    "skipped": 0,
    "total_size": 1234,
    "by_rule": { "PDFs": 1 },
    "by_action": { "move": 1 }
  },
  "log_path": null,
  "errors": []
}
```

After `--apply`, `mode` is `"applied"` and `status` is `"applied"` or `"error"`. `log_path` is the undo log when at least one action succeeded.

## Development

Rust 1.85.1, pinned in `rust-toolchain.toml`.

```bash
cargo build --release
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests build their own home, config, data, and trash directories under `tempfile`. They do not read `~/Downloads`, the real config or data directories, or the system Trash.

GitHub Actions runs the same fmt, clippy, and test checks on push and pull request (`.github/workflows/ci.yml`). Tagged releases build an `aarch64-apple-darwin` binary on `macos-14` and attach a `.tar.gz` plus SHA-256 checksum (`.github/workflows/release.yml`).

## License

MIT. Copyright 2026 Jordan Herin.
