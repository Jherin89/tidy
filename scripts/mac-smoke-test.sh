#!/usr/bin/env bash
# Isolated end-to-end check for tidy on a real Mac.
#
# Creates its own mktemp folder and never touches ~/Downloads. launchd, if
# exercised, uses a distinct label (com.github.jherin89.tidy.smoke) so a
# real tidy agent is left alone. Finder Trash is used only for one uniquely
# named temp file; cleanup removes that file from the Trash if it is still
# there, and nothing else.
#
# Usage (from the repo, or with tidy on PATH):
#   ./scripts/mac-smoke-test.sh
#   TIDY_BIN=/path/to/tidy ./scripts/mac-smoke-test.sh
#
# Safe to run on Linux too: launchd and Finder Put Back are skipped.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PASSED=0
FAILED=0
SKIPPED=0
FAILURES=()

WORK=""
INSTALLED=0
BIN=""
SMOKE_LABEL="com.github.jherin89.tidy.smoke"
PUTBACK_NAME=""
USER_HOME="${HOME}"
REAL_TRASH="${HOME}/.Trash"
IS_DARWIN=0
if [[ "$(uname -s)" == "Darwin" ]]; then
  IS_DARWIN=1
fi

pass() {
  printf '[PASS] %s\n' "$1"
  PASSED=$((PASSED + 1))
}

fail() {
  printf '[FAIL] %s\n' "$1"
  FAILED=$((FAILED + 1))
  FAILURES+=("$1")
}

skip() {
  printf '[SKIP] %s\n' "$1"
  SKIPPED=$((SKIPPED + 1))
}

is_our_tidy() {
  local bin="$1"
  [[ -x "$bin" ]] || return 1
  local help
  help="$("$bin" --help 2>/dev/null || true)"
  printf '%s' "$help" | grep -q "Downloads folder"
}

resolve_bin() {
  if [[ -n "${TIDY_BIN:-}" ]]; then
    if is_our_tidy "$TIDY_BIN"; then
      printf '%s\n' "$TIDY_BIN"
      return 0
    fi
    printf 'TIDY_BIN=%s is missing or is not this tidy (HTML Tidy?)\n' "$TIDY_BIN" >&2
    return 1
  fi

  if command -v tidy >/dev/null 2>&1; then
    local on_path
    on_path="$(command -v tidy)"
    if is_our_tidy "$on_path"; then
      printf '%s\n' "$on_path"
      return 0
    fi
  fi

  if is_our_tidy "$ROOT/target/release/tidy"; then
    printf '%s\n' "$ROOT/target/release/tidy"
    return 0
  fi

  if command -v cargo >/dev/null 2>&1; then
    printf 'No tidy on PATH; building ./target/release/tidy\n' >&2
    (cd "$ROOT" && cargo build --release)
    if is_our_tidy "$ROOT/target/release/tidy"; then
      printf '%s\n' "$ROOT/target/release/tidy"
      return 0
    fi
  fi

  printf 'could not find tidy: set TIDY_BIN, put tidy on PATH, or build in %s\n' "$ROOT" >&2
  return 1
}

remove_our_trash_item() {
  local name="$1"
  [[ -n "$name" ]] || return 0
  local candidate
  if [[ -e "${REAL_TRASH}/${name}" ]]; then
    rm -rf "${REAL_TRASH:?}/${name}"
    return 0
  fi
  # Finder sometimes suffixes a collision: "name 10-26-26.ext"
  for candidate in "${REAL_TRASH}/${name}"*; do
    if [[ -e "$candidate" ]]; then
      local base
      base="$(basename "$candidate")"
      case "$base" in
        "${name}" | "${name}"*)
          rm -rf "$candidate"
          ;;
      esac
    fi
  done
}

cleanup() {
  set +e
  if [[ "$INSTALLED" -eq 1 && -n "$BIN" ]]; then
    "$BIN" watch --uninstall >/dev/null 2>&1
    INSTALLED=0
  fi
  if [[ -n "$PUTBACK_NAME" ]]; then
    remove_our_trash_item "$PUTBACK_NAME"
  fi
  if [[ -n "$WORK" && -d "$WORK" ]]; then
    rm -rf "$WORK"
  fi
}

trap cleanup EXIT

# --- isolate every tidy path under $WORK ---------------------------------
WORK="$(mktemp -d "${TMPDIR:-/tmp}/tidy-smoke.XXXXXX")"
HOME_DIR="${WORK}/home"
CONFIG_DIR="${WORK}/config"
DATA_DIR="${WORK}/data"
INBOX="${WORK}/inbox"
DOCS="${HOME_DIR}/Documents/Downloads"
FAKE_TRASH="${WORK}/fake-trash"
RULES="${WORK}/rules.toml"
WATCH_RULES="${WORK}/watch-rules.toml"

mkdir -p "$HOME_DIR" "$CONFIG_DIR" "$DATA_DIR" "$INBOX" "$FAKE_TRASH" \
  "${HOME_DIR}/Downloads" "${HOME_DIR}/Library/LaunchAgents" "${HOME_DIR}/Library/Logs"

export TIDY_HOME="$HOME_DIR"
export TIDY_CONFIG_DIR="$CONFIG_DIR"
export TIDY_DATA_DIR="$DATA_DIR"
export TIDY_LAUNCHD_LABEL="$SMOKE_LABEL"
export TIDY_WATCH_SETTLE_MS="400"
export NO_COLOR="1"
# Never override HOME: Finder Trash and launchctl belong to the real user.
# Never set TIDY_LAUNCHCTL: the Mac steps use real launchctl.
unset TIDY_LAUNCHCTL || true
unset TIDY_UID || true
unset TIDY_FAKE_TRASH || true

printf 'Work dir  %s\n' "$WORK"
printf 'Label     %s\n' "$SMOKE_LABEL"
printf 'User home %s (untouched Downloads)\n' "$USER_HOME"

# --- 1. locate tidy ------------------------------------------------------
if BIN="$(resolve_bin)"; then
  pass "locate tidy (${BIN})"
else
  fail "locate tidy"
  printf '\nSummary: %s passed, %s failed, %s skipped\n' "$PASSED" "$FAILED" "$SKIPPED"
  exit 1
fi
printf 'Binary    %s\n' "$BIN"
printf 'Version   %s\n' "$("$BIN" --version 2>/dev/null || echo unknown)"

# --- sample files --------------------------------------------------------
# Old-mtime .dmg: mtime is 90 days ago, but Date Added / birth is now.
DMG="${INBOX}/OldInstaller.dmg"
printf 'fake-dmg' >"$DMG"
if date -d '90 days ago' '+%Y%m%d%H%M' >/dev/null 2>&1; then
  touch -d '90 days ago' "$DMG"
else
  touch -t "$(date -v-90d '+%Y%m%d%H%M')" "$DMG"
fi

PAGES="${INBOX}/Quarterly.pages"
mkdir -p "${PAGES}/Contents"
printf '<document/>' >"${PAGES}/index.xml"
printf 'payload' >"${PAGES}/Contents/Data"

SKIP_DIR="${INBOX}/skip-me"
mkdir -p "${SKIP_DIR}/inside"
printf 'nope' >"${SKIP_DIR}/inside/notes.txt"

PDF="${INBOX}/memo.pdf"
printf 'pdf-bytes' >"$PDF"

cat >"$RULES" <<'EOF'
[[rules]]
name = "Old installers"
extensions = ["dmg", "pkg"]
older_than = "30d"
action = "trash"

[[rules]]
name = "Documents"
extensions = ["pages", "numbers", "key", "rtfd", "pdf"]
action = "move"
destination = "~/Documents/Downloads"
EOF

cat >"$WATCH_RULES" <<'EOF'
[[rules]]
name = "PDFs"
extensions = ["pdf"]
action = "move"
destination = "Filed"
EOF

# --- 2. old-mtime .dmg is not trashed ------------------------------------
DRY="$("$BIN" --rules "$RULES" "$INBOX" 2>&1 || true)"
if printf '%s' "$DRY" | grep 'OldInstaller.dmg' | grep -q trash; then
  fail "old-mtime .dmg was planned for trash (Date Added / created should keep it)"
  printf '%s\n' "$DRY"
else
  pass "old-mtime .dmg is not trashed"
fi

# --- 3. iWork package is one item; other dirs skipped --------------------
if printf '%s' "$DRY" | grep -q 'Quarterly.pages' && printf '%s' "$DRY" | grep -q 'Documents'; then
  pass "dry-run lists the .pages package"
else
  fail "dry-run did not list Quarterly.pages as a Documents move"
  printf '%s\n' "$DRY"
fi

if printf '%s' "$DRY" | grep -q 'skip-me' && printf '%s' "$DRY" | grep -qi 'director'; then
  pass "unlisted folder is skipped"
elif printf '%s' "$DRY" | grep -q 'skip-me/inside'; then
  fail "unlisted folder was recursed into"
else
  # skipped section prints "skip-me/"
  if printf '%s' "$DRY" | grep -q 'skip-me'; then
    pass "unlisted folder is skipped"
  else
    fail "unlisted folder skip-me missing from skipped list"
  fi
fi

# --- 4. apply moves the whole package; undo restores it ------------------
if "$BIN" --apply --rules "$RULES" "$INBOX" >/dev/null; then
  pass "apply of sample folder"
else
  fail "apply of sample folder"
fi

DEST_PAGES="${DOCS}/Quarterly.pages"
if [[ -d "$DEST_PAGES" && -f "${DEST_PAGES}/index.xml" && -f "${DEST_PAGES}/Contents/Data" && ! -e "$PAGES" ]]; then
  if [[ "$(cat "${DEST_PAGES}/Contents/Data")" == "payload" ]]; then
    pass "apply moved the .pages package with contents intact"
  else
    fail ".pages contents changed during the move"
  fi
else
  fail "apply did not move Quarterly.pages as a whole"
fi

if [[ -d "$SKIP_DIR" ]]; then
  pass "unlisted folder was left in place"
else
  fail "unlisted folder was moved or removed"
fi

if [[ -f "$DMG" ]]; then
  pass ".dmg stayed in the inbox"
else
  fail ".dmg was removed from the inbox"
fi

UNDO_OUT="$("$BIN" undo 2>&1 || true)"
if [[ -d "$PAGES" && -f "${PAGES}/Contents/Data" && ! -e "$DEST_PAGES" ]]; then
  pass "undo restored the .pages package"
else
  fail "undo did not restore Quarterly.pages"
  printf '%s\n' "$UNDO_OUT"
fi

# --- 5. watch --install --print honors flags (any OS) --------------------
PRINTED="$("$BIN" watch --install --print --rules "$WATCH_RULES" --older-than 7d "$INBOX" 2>/dev/null || true)"
if printf '%s' "$PRINTED" | grep -q -- '--rules' \
  && printf '%s' "$PRINTED" | grep -q -- '--older-than' \
  && printf '%s' "$PRINTED" | grep -q '<string>7d</string>' \
  && printf '%s' "$PRINTED" | grep -q "$SMOKE_LABEL"; then
  pass "watch --install --print writes --rules and --older-than"
else
  fail "watch --install --print dropped flags or label"
fi

# Reject a flag that cannot live in the agent.
if "$BIN" watch --install --print --json "$INBOX" >/dev/null 2>&1; then
  fail "watch --install --print --json should be rejected"
else
  pass "watch --install rejects --json"
fi

# --- 6. Finder-method trash + undo (macOS) -------------------------------
if [[ "$IS_DARWIN" -eq 1 ]]; then
  PUTBACK_DIR="${WORK}/putback"
  mkdir -p "$PUTBACK_DIR"
  PUTBACK_NAME="tidy-smoke-putback-$$.txt"
  PUTBACK_FILE="${PUTBACK_DIR}/${PUTBACK_NAME}"
  printf 'put-back-payload' >"$PUTBACK_FILE"
  TRASH_RULES="${WORK}/trash-rules.toml"
  cat >"$TRASH_RULES" <<'EOF'
[[rules]]
name = "Trash text"
extensions = ["txt"]
action = "trash"
EOF
  unset TIDY_FAKE_TRASH || true
  if "$BIN" --apply --rules "$TRASH_RULES" "$PUTBACK_DIR" >/dev/null; then
    if [[ ! -e "$PUTBACK_FILE" ]]; then
      pass "Finder-method trash moved the temp file out of the inbox"
    else
      fail "Finder-method trash left the temp file in place"
    fi
  else
    fail "Finder-method trash apply failed"
  fi

  UNDO_TRASH="$("$BIN" undo 2>&1 || true)"
  if printf '%s' "$UNDO_TRASH" | grep -q "$PUTBACK_NAME"; then
    pass "tidy undo names the trashed file for Put Back"
  else
    fail "tidy undo did not name the trashed file"
    printf '%s\n' "$UNDO_TRASH"
  fi

  restored=0
  if [[ -f "$PUTBACK_FILE" ]]; then
    restored=1
  else
    # tidy cannot pull files out of Trash on macOS; simulate the user's
    # Finder Put Back so we confirm the item is ours and still restorable.
    if osascript >/dev/null 2>&1 <<OSA
tell application "Finder"
  set theName to "$PUTBACK_NAME"
  set hits to (every item of trash whose name is theName)
  if (count of hits) > 0 then
    put away item 1 of hits
  end if
end tell
OSA
    then
      :
    fi
    if [[ -f "$PUTBACK_FILE" ]]; then
      restored=1
    elif [[ -e "${REAL_TRASH}/${PUTBACK_NAME}" ]]; then
      mkdir -p "$PUTBACK_DIR"
      mv "${REAL_TRASH}/${PUTBACK_NAME}" "$PUTBACK_FILE"
      if [[ -f "$PUTBACK_FILE" ]]; then
        restored=1
      fi
    fi
  fi
  if [[ "$restored" -eq 1 ]]; then
    pass "trashed temp file is restored (Put Back / recovered from Trash)"
    PUTBACK_NAME=""
  else
    fail "could not restore ${PUTBACK_NAME} from the Trash"
  fi
else
  skip "Finder-method trash + Put Back (needs macOS)"
fi

# --- 7. real launchd watch (macOS) ---------------------------------------
if [[ "$IS_DARWIN" -eq 1 ]]; then
  UID_NUM="$(id -u)"
  DEFAULT_TARGET="gui/${UID_NUM}/com.github.jherin89.tidy"
  if launchctl print "$DEFAULT_TARGET" >/dev/null 2>&1; then
    printf '[warn] a real tidy agent is loaded at %s; this test uses %s and will not touch it\n' \
      "$DEFAULT_TARGET" "$SMOKE_LABEL"
  fi

  SMOKE_TARGET="gui/${UID_NUM}/${SMOKE_LABEL}"
  if launchctl print "$SMOKE_TARGET" >/dev/null 2>&1; then
    fail "smoke launchd label ${SMOKE_LABEL} is already loaded; refusing to clobber it"
  else
    WATCH_DIR="${WORK}/watchbox"
    mkdir -p "$WATCH_DIR"
    if "$BIN" watch --install --rules "$WATCH_RULES" --older-than 7d "$WATCH_DIR"; then
      INSTALLED=1
      pass "watch --install loaded the smoke agent"
    else
      fail "watch --install failed"
    fi

    STATUS_OUT="$("$BIN" watch --status 2>&1 || true)"
    if printf '%s' "$STATUS_OUT" | grep -q -- '--older-than\|Age' \
      && printf '%s' "$STATUS_OUT" | grep -q 'watch-rules.toml\|Rules'; then
      pass "watch --status shows configured arguments"
    else
      fail "watch --status did not show --rules / --older-than"
      printf '%s\n' "$STATUS_OUT"
    fi

    if launchctl print "$SMOKE_TARGET" >/dev/null 2>&1; then
      pass "launchctl print ${SMOKE_TARGET} shows the job loaded"
    else
      fail "launchctl print did not show ${SMOKE_TARGET}"
    fi

    # --older-than 7d is in the plist (checked above). A just-dropped file
    # would be skipped by that filter, so bounce the agent without it.
    "$BIN" watch --uninstall >/dev/null 2>&1 || true
    INSTALLED=0
    if "$BIN" watch --install --rules "$WATCH_RULES" "$WATCH_DIR"; then
      INSTALLED=1
    else
      fail "watch --install (no age filter) failed"
    fi

    DROP="${WATCH_DIR}/incoming.pdf"
    printf 'watched-pdf' >"$DROP"
    found=0
    i=0
    while [[ "$i" -lt 25 ]]; do
      if [[ -f "${WATCH_DIR}/Filed/incoming.pdf" ]]; then
        found=1
        break
      fi
      i=$((i + 1))
      sleep 0.4
    done
    if [[ "$found" -eq 1 ]]; then
      pass "watch agent filed a dropped PDF"
    else
      fail "watch agent did not file incoming.pdf (grant Full Disk Access?)"
      printf 'watch dir listing:\n'
      ls -la "$WATCH_DIR" || true
      printf 'status:\n%s\n' "$("$BIN" watch --status 2>&1 || true)"
    fi

    "$BIN" watch --uninstall >/dev/null 2>&1 || true
    INSTALLED=0
    if launchctl print "$SMOKE_TARGET" >/dev/null 2>&1; then
      fail "watch --uninstall left the smoke agent loaded"
    else
      pass "watch --uninstall removed the smoke agent"
    fi
  fi
else
  skip "watch --install with real launchctl (needs macOS)"
  skip "launchctl print gui/\$UID/<label> (needs macOS)"
  skip "drop a file into the watched folder (needs macOS)"
fi

# --- summary -------------------------------------------------------------
printf '\nSummary: %s passed, %s failed, %s skipped\n' "$PASSED" "$FAILED" "$SKIPPED"
if [[ "$FAILED" -gt 0 ]]; then
  printf 'Failed steps:\n'
  for item in "${FAILURES[@]}"; do
    printf '  - %s\n' "$item"
  done
  exit 1
fi
exit 0
