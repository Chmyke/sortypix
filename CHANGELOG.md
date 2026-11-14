# Changelog

What changed in Sortypix, as you would notice it. Newest first.

## 0.4.1 · 2026-11-14

- Folders open much faster, RAW folders and memory cards most of all: Sortypix now reads only the few bytes it needs from each photo instead of the whole file, and reads several photos at once.
- Undo is instant: the photo comes back to its place without the whole folder being read again.
- Thumbnails are decoded at the size of their cell, so the grid fills faster and scrolling back finds them still there. Your eyes will not notice the difference, your laptop fan will.
- Sortypix now sleeps when you do nothing: no more quiet redrawing in the background ten times a second.
- Sortypix AI keeps its model busy while photos are decoded, so the analysis of a folder finishes sooner.
- **New** · Sortypix AI tags your photos (dog, beach, night, crowd, 1girl...): up to four words per photo from a small layer trained on labels checked by people, a couple per drawing or generated image. Click in Search to see the folder's tags with their counts, pick one or type a few separated by commas, like a prompt. Nothing is written in your files.
- The score stops pretending: instead of a mysterious 4.7, a suggested photo says Bottom 8% of this folder, and the photo details explain that it rates visual appeal, not what the photo means to you.

## 0.4.0 · 2026-10-30

- **New** · A new order menu: oldest, newest, by name (IMG_2 comes before IMG_10, as it should), largest or smallest files first. Tiny files are often the misfires.
- **New** · Keep and reject as fast as your fingers go: the photo leaves at once while the file moves behind the scenes, even on a slow USB stick.
- Huge folders show their first photos almost instantly, and a folder of 100,000 files opens more than twice as fast.
- RAW photos appear about three times faster in the grid and full screen. The full sensor still answers at 100% zoom.
- **New** · JPEG XL photos are welcome, along with .jfif, .jpe and a few more RAW flavours.
- **New** · Sortypix AI keeps bursts together in the grid, and a Bursts order shows only them, one burst after the other.
- **Fixed** · A locked SD card or a read-only drive is now said at opening, instead of a puzzling error at the first click.
- **Fixed** · A photo another program takes away while you sort simply leaves the grid, and a move only counts once the file has really arrived.

## 0.3.0 · 2026-10-23

- **New** · Meet Sortypix AI (beta), a separate app that installs next to Sortypix: a model running on your own computer suggests which photos to reject, and one click sends them away. One Ctrl+Z brings them all back.
- **New** · Sortypix AI also finds photos by word ("dog", "sunset"), sorts them by score or by variety, and tells you why a photo looks weak. Your photos never leave the machine, not even for a coffee break.
- Camera photos show up two to three times faster in the grid. Sortypix itself stays exactly as light as before.
- **Fixed** · Full screen no longer keeps your processor busy while you are simply looking at a photo.

## 0.2.1 · 2026-10-22

- **New** · Up arrow keeps, down arrow rejects, left and right flip through photos, in the grid and full screen. One hand on the arrows, the other on your coffee.
- Left click now keeps and right click rejects, which is what most hands expected anyway. Prefer the old way? The mouse icon swaps them back.

## 0.2.0 · 2026-10-17

- **New** · Sortypix now runs on Linux too, on regular PCs and on ARM boards like the Raspberry Pi.
- **New** · Animated GIFs actually move, in the grid and full screen. Sorting memes is serious work.
- **New** · RAW photos show up even without a RAW codec, using the preview your camera tucked inside the file.
- Rotate sits on the photo itself, the folder you just sorted into lights up, full screen tells you where you are ("12 / 340"), and there's a new icon that's easier to spot.

## 0.1.8 · 2026-09-29

First public release.

- **New** · Open a folder, left-click to reject, right-click to keep. Nothing is ever deleted, and Ctrl+Z undoes the whole last gesture.
- **New** · RAW + JPEG pairs travel together, and identical copies are set aside on their own.
- **New** · Huge folders open straight away, and you pick up right where you stopped.
- **New** · Right-click any folder in Windows and pick "Open with Sortypix".
