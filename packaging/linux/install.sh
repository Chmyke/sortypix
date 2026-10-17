#!/bin/sh
# Installs Sortypix for the current user only: no root, nothing outside $HOME.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
apps="$data/applications"
icons="$data/icons/hicolor"
mkdir -p "$bin" "$apps" "$icons/256x256/apps" "$icons/scalable/apps"
# What opens folders today: Sortypix offers itself for folders, it must never take over.
query_folders() {
    if command -v xdg-mime >/dev/null 2>&1; then
        xdg-mime query default inode/directory 2>/dev/null || true
    fi
}
before=$(query_folders)
install -m 755 "$here/sortypix" "$bin/sortypix"
install -m 644 "$here/icons/sortypix.png" "$icons/256x256/apps/sortypix.png"
install -m 644 "$here/icons/sortypix.svg" "$icons/scalable/apps/sortypix.svg"
entry() {
    cat <<EOF
[Desktop Entry]
Type=Application
Name=Sortypix
GenericName=Photo sorter
Comment=Sort large folders of photos quickly
Exec=$bin/sortypix %f
Icon=sortypix
Terminal=false
Categories=Graphics;Photography;Viewer;
StartupWMClass=sortypix
EOF
    # "Open With > Sortypix" on a folder in the file manager.
    [ "${1:-}" = folders ] && echo 'MimeType=inode/directory;'
    return 0
}
entry folders > "$apps/sortypix.desktop"
refresh() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$apps" >/dev/null 2>&1 || true
    fi
}
refresh
after=$(query_folders)
if [ "$after" = sortypix.desktop ] && [ "$before" != sortypix.desktop ]; then
    if [ -n "$before" ]; then
        # Folders keep opening where they did.
        xdg-mime default "$before" inode/directory
    else
        # No default to keep: Sortypix stays out of the folder handlers.
        entry > "$apps/sortypix.desktop"
        refresh
    fi
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -f "$icons/index.theme" ]; then
    gtk-update-icon-cache -q "$icons" 2>/dev/null || true
fi
echo "Sortypix installed in $bin."
echo "Open it from your applications menu, or run: $bin/sortypix <folder>"
