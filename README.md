# Sortypix

Sort thousands of photos fast: keep the good ones, set the rest aside.
Desktop app for Windows and Linux, written in Rust. Download and demos: https://mickaelmancini.com/sortypix/

Version 0.3.0 for Windows, and for Linux (x86_64 and ARM64); Sortypix AI 0.3.0 beta for Windows. English interface, offline, free and open source (MIT).

Open one folder, then left-click to keep or right-click to reject. Photos move into
`rejected/` and `keep/`; nothing is deleted or overwritten. Swap mouse buttons using the mouse icon or the shortcut editor.
RAW + JPEG pairs stay together. Identical copies are set aside automatically.

| Shortcut | Action |
|---|---|
| Up / Down arrows | Keep / reject |
| Left / Right arrows | Previous / next photo |
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

## Sortypix or Sortypix AI?

Sortypix AI is a second app, in beta: the same Sortypix, plus a model that runs on your
computer, looks at every photo of the open folder and suggests which ones to set aside.
Windows only for now. While it analyses, a thin blue line runs under the toolbar;
then the photos it would set aside get a red dashed outline with their score (or an amber
"Off-topic" badge), "Suggest 20%" picks how many, and "Reject suggested" moves them all to
`rejected/` in one gesture (one Ctrl+Z puts them back). A search field filters the grid by word,
and the order menu adds "Lowest score first", "Highest score first" and "Most varied first", can
show only the low scores, the off-topic photos or the bursts, and hides a kind of badge you do
not want. Press I (or click a badge) for a photo's card: its score and where it stands in the
folder, and a likely reason when the score is low (an estimate: only blur is told apart reliably
for now). On Windows the analysis progress also shows on the taskbar button.

- **Pick Sortypix** if you want the lightest and fastest sorter. It knows nothing about what is
  in your photos, and that is the point: you decide everything, nothing runs in the background.
- **Pick Sortypix AI** if your folders are huge and you would like a second opinion: a score for
  each photo, bursts of near-identical shots, photos that do not fit with the rest, and search by
  word ("dog", "sunset").

| | Sortypix | Sortypix AI (beta) |
|---|---|---|
| Download | about 5 MB | about 400 MB |
| On disk | about 16 MB | about 0.6 GB, plus up to 1 GB of analysis cache (3 KB per photo) |
| Analysis | none | about 10 to 16 photos per second on a 16-core desktop |
| Network | never | never |

At that pace, 1,000 photos take one to two minutes, 30,000 photos 30 to 50 minutes and
300,000 photos five to eight hours (projection from the measured speed). Each photo is analysed
once: moving it into `keep/` or `rejected/` does not start over.

Good to know before you pick Sortypix AI:

- **Everything stays on your computer.** The model ships inside the installer, the analysis runs
  locally and the app never goes online, not even to update itself.
- **The model only suggests.** It never moves, renames or deletes a photo by itself. You still
  keep with a left click and reject with a right click; a suggested photo goes to `rejected/`
  only when you say so, and Ctrl+Z brings it back.
- **Bursts are only marked in this beta.** The model is not yet reliable enough at picking the
  best shot of a burst, so it points the burst out and leaves the choice to you.
- **It keeps your processor busy while it analyses.** The analysis runs below normal priority, so
  scrolling and sorting stay smooth, but fans may spin up and a laptop on battery drains faster.
  Expect about 1.5 GB of memory in use during the analysis.
- **Both install side by side.** Each has its own folder, Start menu entry and "Open with" entry
  on folders, and they share their settings. Installing or removing one never touches the other.

## Build

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
