#!/usr/bin/env bash
# Copy mods/*/ into the herdr-app home
set -euo pipefail
shopt -s nullglob
SRC_DIR="$(cd -- "$(dirname "${BASH_SOURCE[0]}")" >/dev/null && pwd)"
HOME_DIR="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
mkdir -p "$HOME_DIR/mods"
for src in "$SRC_DIR"/mods/*/; do
  dest="$HOME_DIR/mods/$(basename "$src")"
  if [ -L "$dest" ]; then rm "$dest"; fi
  # --delete only applies inside this mod folder. Exclude the type files the app writes so they are kept (don't use --delete-excluded)
  rsync -a --delete --exclude=jsconfig.json --exclude='*.d.ts' "$src" "$dest/"
  echo "mod: $dest"
done
