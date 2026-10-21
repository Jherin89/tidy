# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-10-21

### Added

- `tidy explain FILE` shows each rule condition (pass or fail) and the same winner and destination the plan would use. `--json` prints the structured form. A file that matches nothing exits 0.
- Rule condition `duplicate`. `true` matches an extra byte-identical copy; `false` matches a file that is not an extra copy. The built-in **Duplicates** rule is first and sends extras to the Trash. Zero-byte files are ignored. The keeper is the oldest file, then the shortest name, then the lexicographic name.
- `tidy watch --install`, `--uninstall`, `--status`, and `--install --print` for a macOS launchd agent. `--print` writes the plist to stdout on any OS.
- A short explanation when reading a folder fails with "Operation not permitted", including where to grant Full Disk Access.
- Release archives for a universal macOS binary and static Linux musl binaries (x86_64 and aarch64), plus `scripts/bump-homebrew.sh` to render the Homebrew formula.

### Changed

- `--apply`, `--older-than`, `--rules`, and `--json` are accepted only on the commands that use them. `tidy init --help` and `tidy undo --help` no longer list the plan flags.
- CI runs fmt, clippy, and tests on Ubuntu and macOS 14. The macOS job lints the generated plist and, only in CI, exercises the real Trash with `TIDY_TRASH_METHOD=nsfilemanager`.
- The default Trash method on macOS is still Finder (so Put Back works). Set `TIDY_TRASH_METHOD=nsfilemanager` for a headless runner.

## [0.1.0] - 2026-10-21

### Added

- First public release. `tidy` previews a folder against built-in or TOML rules and moves or trashes matches only when `--apply` is passed.
- Conditions for extension, glob, age, and size. Actions are move (optional `YYYY-MM` folders) and trash.
- `tidy undo`, `tidy watch`, `tidy init`, `tidy rules`, and `--json`.
- Dry-run by default, no overwrite, partial downloads and hidden files skipped, and tests that never touch a real home directory or the system Trash.
