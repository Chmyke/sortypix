#!/bin/sh
# Removes what install.sh added. Photos, sorted folders and settings are kept.
set -eu
data="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f "$HOME/.local/bin/sortypix" "$data/applications/sortypix.desktop" \
    "$data/icons/hicolor/256x256/apps/sortypix.png" "$data/icons/hicolor/scalable/apps/sortypix.svg"
if command -v update-desktop-database >/dev/null 2>&1 && [ -d "$data/applications" ]; then
    update-desktop-database "$data/applications" >/dev/null 2>&1 || true
fi
echo "Sortypix removed. Settings and previews stay in ~/.config/sortypix and ~/.cache/sortypix."
