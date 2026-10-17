#!/bin/sh
# Runs install.sh then uninstall.sh of a Linux package inside a throwaway home and checks
# what they add and take back. Nothing is written outside that home.
# Usage: sh test-linux-install.sh <package folder> <throwaway home>
set -eu
package=${1:?package folder}
home=${2:?throwaway home}
rm -rf "$home"
mkdir -p "$home"
export HOME="$home"
unset XDG_DATA_HOME XDG_CONFIG_HOME
data="$home/.local/share"
entry="$data/applications/sortypix.desktop"
fail() {
    echo "FAILED: install: $1"
    exit 1
}
folders() {
    xdg-mime query default inode/directory 2>/dev/null || true
}
before=$(command -v xdg-mime >/dev/null 2>&1 && folders || echo none)

sh "$package/install.sh" >/dev/null
[ -x "$home/.local/bin/sortypix" ] || fail "the program is not installed"
[ -f "$data/icons/hicolor/256x256/apps/sortypix.png" ] || fail "the icon is not installed"
grep -q "^Exec=$home/.local/bin/sortypix %f\$" "$entry" || fail "the entry does not pass the folder"
grep -q '^Icon=sortypix$' "$entry" || fail "the entry has no icon"
if command -v xdg-mime >/dev/null 2>&1; then
    [ "$(folders)" != sortypix.desktop ] || fail "Sortypix took over opening folders"
    [ "$(folders)" = "$before" ] || fail "the folder default changed from $before to $(folders)"
    checked="folder default kept: ${before:-none}"
else
    checked="files only: no xdg-mime here"
fi
if grep -q '^MimeType=inode/directory;$' "$entry"; then offered="offered for folders"; else offered="not offered for folders"; fi

sh "$package/uninstall.sh" >/dev/null
[ ! -e "$home/.local/bin/sortypix" ] || fail "the program outlives the removal"
[ ! -e "$entry" ] || fail "the menu entry outlives the removal"
rm -rf "$home"
echo "install: entry and icon added then removed, $offered ($checked)"
