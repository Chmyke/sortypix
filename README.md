# Sortypix

Sort thousands of photos fast: keep the good ones, set the rest aside.
Desktop app for Windows and Linux, written in Rust. Download and demos: https://mickaelmancini.com/sortypix/

Preview 0.2.0 for Windows, and for Linux (x86_64 and ARM64). English interface, offline, free and open source (MIT).

Open one folder, then left-click to reject or right-click to keep. Photos move into
`rejected/` and `keep/`; nothing is deleted or overwritten. Swap mouse buttons using the mouse icon or the shortcut editor.
RAW + JPEG pairs stay together. Identical copies are set aside automatically.

| Shortcut | Action |
|---|---|
| K / X | Keep / reject |
| Ctrl+click or middle click | Reject through this photo |
| Ctrl+Z | Undo the whole last gesture, across folders, until the app closes |
| Enter / Escape | Full screen / grid |
| Z | Fit / 100% |
| R / Shift+R | Rotate the view clockwise / counterclockwise |

All controls live in one icon toolbar at the top. Hover an icon for its action and shortcut.
The keyboard icon opens the shortcut editor: click a binding, press your keys, and it is saved immediately. Conflicts are rejected; clear a binding or restore defaults at any time. The mouse icon swaps left/right actions.
The green/red checklist icons keep/reject all remaining photos. The four folder icons are grouped on the left; the arrow icon opens the source folder, and the green/red ones open `keep/` and `rejected/` in the file manager once those folders exist.
Photos fill justified rows with their original proportions, without cropping. The last
partial row stays at the requested scale. The size slider reaches very large previews,
with resolution adjusted for the thumbnail size and display scale. Scroll over the thumbnail-size slider or use Ctrl+wheel over photos to resize the gallery. Expand a photo using its
bottom-right icon; the matching bottom-right control returns from full screen.

The app remembers the current photo position when reopening the same folder, plus the window size, position and screen. If that screen is disconnected, the window returns to an available screen. A local preview cache (up to 512 MiB) speeds up reopening; full-resolution inspection still reads the original. The next two rows are prefetched in the scrolling direction.

Drag the image to pan at 100%. Rotation affects only this session, not the original file.
RAW and HEIC decoding use the codecs installed in Windows; without a RAW codec, and on Linux,
RAW photos show the full-size preview the camera embedded in the file. HEIC and AVIF are not
shown on Linux yet. Unsupported images remain visible as placeholders and can still be sorted.
The application never downloads codecs.
For RAW + JPEG pairs, 100% inspects the native RAW when available; a JPEG fallback is explicitly labelled.
Windows 11 integration uses a small, signed Explorer extension to offer “Open with Sortypix” directly in the modern folder context menu. The classic menu also retains the command, including on folder backgrounds. The identity package must be signed with a certificate trusted by Windows; local development certificates are never trusted automatically.

Build and verify with `powershell -NoProfile -ExecutionPolicy Bypass -File check.ps1`.
Requires Rust, Visual Studio C++ tools and Windows SDK. Full verification launches isolated
UI diagnostics and performance probes; all generated photographs stay under `target/`.
The RAW test also requires the Windows Raw Image Extension.

Build the installer after a successful check and clean commit:
`powershell -NoProfile -ExecutionPolicy Bypass -File packaging/build-installer.ps1`.
Requires Inno Setup 6 and a signed Explorer identity package. Run `packaging/build-explorer-package.ps1 -CertificateThumbprint <publisher-certificate>` before building the installer (the signing certificate must be in CurrentUser/My). Installer output: `packaging/out/`. No signing key is stored in the repository.
On a machine without an existing Sortypix installation, `check.ps1 -Installer` also builds,
installs into a test directory, checks Explorer commands and uninstalls the test copy.

## Linux

The Linux build is cross-compiled from Windows with `cargo-zigbuild` (glibc 2.31):
`powershell -NoProfile -ExecutionPolicy Bypass -File packaging/build-linux.ps1` writes
`sortypix-<version>-linux-{x86_64,arm64}.tar.gz` to `packaging/out/`. Each archive installs
for the current user with `sh install.sh` (no root) and adds a menu entry and “Open With >
Sortypix” on folders, without ever becoming the default folder handler.
`scripts/check-linux.ps1 -Diagnostic` runs the tests, the native UI probe and the install
scripts on an ARM64 Linux desktop over SSH; `check.ps1` includes it (`-SkipLinux` to skip).
Needs `rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu`,
`cargo install cargo-zigbuild` and `pip install ziglang`.

Bug reports and ideas: see [CONTRIBUTING.md](CONTRIBUTING.md). Security: [SECURITY.md](SECURITY.md).
