from pathlib import Path
import argparse
import binascii
import math
import struct
import xml.etree.ElementTree as ET
import zlib

ROOT = Path(__file__).resolve().parents[1]
SVG = ROOT / "assets" / "sortypix.svg"
PNG = ROOT / "assets" / "sortypix.png"
ICO = ROOT / "assets" / "sortypix.ico"
SIZES = (16, 24, 32, 48, 256)
SCALE = 4
ALLOWED = {"svg", "g", "rect", "polygon", "polyline"}
INHERITED = ("fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin")


def local_name(tag):
    return tag.rsplit("}", 1)[-1]


def color(value):
    if not value or value == "none":
        return None
    if len(value) != 7 or value[0] != "#":
        raise ValueError(f"Unsupported color: {value}")
    return tuple(int(value[index : index + 2], 16) for index in (1, 3, 5)) + (255,)


def points(value):
    return [tuple(map(float, pair.split(","))) for pair in value.split()]


def read_shapes():
    root = ET.parse(SVG).getroot()
    if root.attrib.get("viewBox") != "0 0 128 128":
        raise ValueError("The icon viewBox must remain 0 0 128 128")
    if not any(element.attrib.get("id") == "photo-forward" for element in root.iter()):
        raise ValueError("Missing photo-forward group")
    shapes = []

    def visit(element, inherited):
        tag = local_name(element.tag)
        if tag not in ALLOWED:
            raise ValueError(f"Unsupported SVG element: {tag}")
        style = dict(inherited)
        for name in INHERITED:
            if name in element.attrib:
                style[name] = element.attrib[name]
        if tag == "rect":
            shapes.append(
                {
                    "kind": tag,
                    "x": float(element.attrib["x"]),
                    "y": float(element.attrib["y"]),
                    "width": float(element.attrib["width"]),
                    "height": float(element.attrib["height"]),
                    "radius": float(element.attrib.get("rx", "0")),
                    "fill": color(style.get("fill")),
                }
            )
        elif tag == "polygon":
            shapes.append({"kind": tag, "points": points(element.attrib["points"]), "fill": color(style.get("fill"))})
        elif tag == "polyline":
            if style.get("stroke-linecap") != "round" or style.get("stroke-linejoin") != "round":
                raise ValueError("Polylines require round caps and joins")
            shapes.append(
                {
                    "kind": tag,
                    "points": points(element.attrib["points"]),
                    "stroke": color(style.get("stroke")),
                    "width": float(style["stroke-width"]),
                }
            )
        for child in element:
            visit(child, style)

    visit(root, {})
    return shapes


def inside_rounded_rect(x, y, shape):
    left, top = shape["x"], shape["y"]
    right, bottom = left + shape["width"], top + shape["height"]
    if not (left <= x <= right and top <= y <= bottom):
        return False
    radius = min(shape["radius"], shape["width"] / 2, shape["height"] / 2)
    center_x = min(max(x, left + radius), right - radius)
    center_y = min(max(y, top + radius), bottom - radius)
    return (x - center_x) ** 2 + (y - center_y) ** 2 <= radius**2


def inside_polygon(x, y, vertices):
    inside = False
    previous_x, previous_y = vertices[-1]
    for current_x, current_y in vertices:
        crosses = (current_y > y) != (previous_y > y)
        if crosses:
            edge_x = (previous_x - current_x) * (y - current_y) / (previous_y - current_y) + current_x
            if x < edge_x:
                inside = not inside
        previous_x, previous_y = current_x, current_y
    return inside


def segment_distance(x, y, start, end):
    start_x, start_y = start
    delta_x, delta_y = end[0] - start_x, end[1] - start_y
    length_squared = delta_x * delta_x + delta_y * delta_y
    if length_squared == 0:
        return math.hypot(x - start_x, y - start_y)
    position = max(0.0, min(1.0, ((x - start_x) * delta_x + (y - start_y) * delta_y) / length_squared))
    return math.hypot(x - (start_x + position * delta_x), y - (start_y + position * delta_y))


def covers(x, y, shape):
    if shape["kind"] == "rect":
        return shape["fill"] is not None and inside_rounded_rect(x, y, shape)
    if shape["kind"] == "polygon":
        return shape["fill"] is not None and inside_polygon(x, y, shape["points"])
    half_width = shape["width"] / 2
    return shape["stroke"] is not None and any(
        segment_distance(x, y, start, end) <= half_width
        for start, end in zip(shape["points"], shape["points"][1:])
    )


def png_chunk(kind, payload):
    body = kind + payload
    return struct.pack(">I", len(payload)) + body + struct.pack(">I", binascii.crc32(body) & 0xFFFFFFFF)


def png_bytes(size):
    shapes = read_shapes()
    high_size = size * SCALE
    pixels = bytearray(high_size * high_size * 4)
    for pixel_y in range(high_size):
        y = (pixel_y + 0.5) * 128 / high_size
        for pixel_x in range(high_size):
            x = (pixel_x + 0.5) * 128 / high_size
            value = (0, 0, 0, 0)
            for shape in shapes:
                if covers(x, y, shape):
                    value = shape.get("fill") or shape.get("stroke")
            offset = (pixel_y * high_size + pixel_x) * 4
            pixels[offset : offset + 4] = bytes(value)
    rows = bytearray()
    for target_y in range(size):
        rows.append(0)
        for target_x in range(size):
            totals = [0, 0, 0, 0]
            for sample_y in range(SCALE):
                for sample_x in range(SCALE):
                    source_x = target_x * SCALE + sample_x
                    source_y = target_y * SCALE + sample_y
                    offset = (source_y * high_size + source_x) * 4
                    for channel in range(4):
                        totals[channel] += pixels[offset + channel]
            rows.extend(round(total / (SCALE * SCALE)) for total in totals)
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", header)
        + png_chunk(b"IDAT", zlib.compress(bytes(rows), 9))
        + png_chunk(b"IEND", b"")
    )


def ico_bytes():
    images = [(size, png_bytes(size)) for size in SIZES]
    offset = 6 + 16 * len(images)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    payload = bytearray()
    for size, image in images:
        encoded_size = 0 if size == 256 else size
        directory.extend(struct.pack("<BBBBHHII", encoded_size, encoded_size, 0, 0, 1, 32, len(image), offset))
        payload.extend(image)
        offset += len(image)
    return bytes(directory + payload)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected = {PNG: png_bytes(256), ICO: ico_bytes()}
    if args.check:
        mismatches = [path for path, data in expected.items() if not path.is_file() or path.read_bytes() != data]
        for path in mismatches:
            print(f"MISMATCH: {path.relative_to(ROOT)}")
        return int(bool(mismatches))
    for path, data in expected.items():
        path.write_bytes(data)
        print(f"WROTE: {path.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
