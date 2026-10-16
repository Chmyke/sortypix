from pathlib import Path
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parents[1] / 'tests' / 'fixtures'
root.mkdir(parents=True, exist_ok=True)
image = Image.new('RGB', (80, 40))
draw = ImageDraw.Draw(image)
for box, color in [((0,0,39,19),(240,20,20)), ((40,0,79,19),(20,240,20)),
                   ((0,20,39,39),(20,20,240)), ((40,20,79,39),(240,240,20))]:
    draw.rectangle(box, fill=color)
for orientation in range(1, 9):
    exif = Image.Exif()
    exif[274] = orientation
    exif[306] = '2025:06:01 12:30:00'
    image.save(root / f'orientation-{orientation}.jpg', quality=100, subsampling=0, exif=exif)
