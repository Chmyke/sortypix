Sortypix for Linux (preview)

Install:   sh install.sh      (current user only, no root)
Remove:    sh uninstall.sh

Open:      from your applications menu, with "Open With > Sortypix" on a folder
           in your file manager, or with: ~/.local/bin/sortypix <folder>
           You can also drop a folder on the window.
Folders:   the Open button uses zenity (GNOME and most desktops) or kdialog (KDE).

RAW photos show the full-size preview the camera embedded in the file.
Not yet on Linux: HEIC and AVIF photos (they still sort, without a picture).

Requires a 64-bit desktop with OpenGL 2 or later, on Wayland or X11, and
glibc 2.31 or later (Ubuntu 20.04, Debian 11, Fedora 32 and later).

Settings: ~/.config/sortypix. Previews: ~/.cache/sortypix (safe to delete).
Sortypix never uses the network.

License and source code: see ABOUT.txt.
