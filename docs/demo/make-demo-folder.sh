#!/usr/bin/env bash
# Builds a realistic *fake* Downloads folder under $DEMO (default /tmp/tidy-demo)
# and prints the env vars that point tidy at it. Never touches your real folders.
set -euo pipefail
DEMO="${DEMO:-/tmp/tidy-demo}"
rm -rf "$DEMO"
mkdir -p "$DEMO/home/Downloads" "$DEMO/home/.config" "$DEMO/home/Library/Application Support" "$DEMO/home/.Trash"
D="$DEMO/home/Downloads"
mk() { # name size_kb age
  head -c $(( $2 * 1024 )) /dev/zero > "$D/$1"
  touch -d "$3" "$D/$1"
}
mk "invoice-2026-03.pdf"                       184 "3 days ago"
# Exact extra copy. The original is older, so it is the keeper.
cp "$D/invoice-2026-03.pdf" "$D/invoice-2026-03 (1).pdf"
touch -d "1 day ago" "$D/invoice-2026-03 (1).pdf"
mk "Lease_Agreement_signed.pdf"                912 "40 days ago"
mk "IMG_4821.HEIC"                            2310 "6 days ago"
mk "IMG_4822.HEIC"                            2275 "6 days ago"
mk "Screenshot 2026-10-01 at 9.14.22 AM.png"   488 "20 days ago"
mk "Screenshot 2026-10-17 at 4.02.51 PM.png"   517 "4 days ago"
mk "zoom-installer.pkg"                      48200 "95 days ago"
mk "Slack-4.41.97-macOS.dmg"                 92100 "62 days ago"
mk "Docker.dmg"                             612000 "5 days ago"
mk "Q3-pipeline-forecast.xlsx"                 341 "8 days ago"
mk "customers_export.csv"                     1880 "2 days ago"
mk "Board Deck v7 FINAL.pptx"                 6400 "11 days ago"
mk "meeting-notes.docx"                         58 "1 day ago"
mk "team-offsite-recap.mp4"                  88000 "15 days ago"
mk "podcast-ep42.mp3"                        41000 "9 days ago"
mk "brand-assets.zip"                        15500 "45 days ago"
mk "project-backup-2026-08.tar.gz"           73000 "70 days ago"
mk "The-Pragmatic-Programmer.epub"            2900 "120 days ago"
mk "random-notes.key"                          730 "13 days ago"
mk "unknown-file.bin"                           12 "1 day ago"
mk "Figma-agent-installer.zip"                 220 "1 day ago"
# things tidy always skips
mk "ubuntu-24.04.iso.crdownload"            900000 "1 minute ago"
mk ".DS_Store"                                   6 "1 day ago"
mkdir -p "$D/old-projects"
cat <<VARS
export TIDY_HOME=$DEMO/home
export TIDY_CONFIG_DIR="$DEMO/home/.config"
export TIDY_DATA_DIR="$DEMO/home/Library/Application Support"
export TIDY_FAKE_TRASH="$DEMO/home/.Trash"
VARS
