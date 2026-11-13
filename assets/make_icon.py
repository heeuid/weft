"""Generate weft's individually drawn application icons. Requires Pillow only."""
from pathlib import Path
from io import BytesIO
import struct
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
BG = '#1e1e2e'
BORDER = '#313244'
BLUE = '#89b4fa'
MAUVE = '#cba6f7'
TEAL = '#94e2d5'
TEXT = '#cdd6f4'


def draw_icon(size):
    # Native pixel geometry for taskbar sizes; supersampling for larger icons.
    scale = 1 if size <= 24 else 4
    im = Image.new('RGBA', (size * scale, size * scale))
    d = ImageDraw.Draw(im)
    def rect(box, color):
        x0, y0, x1, y1 = box  # half-open pixel coordinates
        d.rectangle((x0*scale, y0*scale, x1*scale-1, y1*scale-1), fill=color)
    inset = 0 if size <= 24 else 1
    radius = round(size * .22)
    d.rounded_rectangle((inset*scale, inset*scale, (size-inset)*scale-1,
                         (size-inset)*scale-1), radius=radius*scale,
                        fill=BG, outline=BORDER, width=scale)
    # Snap all bands and crossings to the native output pixel grid.
    w = max(2, round(size * .105))
    x0, x1 = round(size*.22), round(size*.44)
    y0, y1 = round(size*.31), round(size*.58)
    left, right = round(size*.14), round(size*.64)
    top, bottom = round(size*.21), round(size*.79)
    for x, color in ((x0, MAUVE), (x1, TEAL)):
        rect((x, top, x+w, bottom), color)
    for y in (y0, y1):
        rect((left, y, right, y+w), BLUE)
    # Opposite crossings pass over: a genuine alternating two-by-two weave.
    # Dark shoulders make the over/under construction legible at larger sizes.
    gap = 0 if size <= 24 else max(1, round(size*.018))
    for x, y, color in ((x0, y0, MAUVE), (x1, y1, TEAL)):
        if gap:
            rect((x-gap, y, x+w+gap, y+w), BG)
        rect((x, y, x+w, y+w), color)
    # A small, bright prompt at the open right edge of the woven grid.
    cx, cy = round(size*.74), round(size*.5)
    reach = max(2, round(size*.115))
    stroke = max(1, round(size*.065))
    if size <= 24:
        # Explicit stair steps avoid softened diagonal strokes at small sizes.
        for step in range(reach+1):
            rect((cx+step, cy-reach+step, cx+step+stroke, cy-reach+step+stroke), TEXT)
            rect((cx+step, cy+reach-step, cx+step+stroke, cy+reach-step+stroke), TEXT)
    else:
        d.line([(cx*scale, (cy-reach)*scale), ((cx+reach)*scale, cy*scale),
                (cx*scale, (cy+reach)*scale)], fill=TEXT,
               width=stroke*scale, joint='curve')
    if scale > 1:
        im = im.resize((size, size), Image.Resampling.LANCZOS)
    return im


def write_ico(images):
    # Write each separately rendered image into an ICO container. Pillow's
    # ordinary ICO save would derive smaller entries from its largest image.
    payloads = []
    for im in images:
        buf = BytesIO()
        im.save(buf, format='PNG')
        payloads.append(buf.getvalue())
    offset = 6 + 16*len(images)
    directory = bytearray(struct.pack('<HHH', 0, 1, len(images)))
    for im, data in zip(images, payloads):
        n = im.width if im.width < 256 else 0
        directory.extend(struct.pack('<BBBBHHII', n, n, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    (ROOT/'weft.ico').write_bytes(bytes(directory) + b''.join(payloads))


def make_preview(images):
    pad, label_h, row_h = 20, 25, 302
    widths = [max(48, n)+pad for n in SIZES]
    canvas = Image.new('RGB', (sum(widths)+pad, row_h*2))
    d = ImageDraw.Draw(canvas)
    font = ImageFont.load_default()
    for row, (background, foreground) in enumerate((('#11111b', TEXT), ('#eff1f5', '#4c4f69'))):
        y = row*row_h
        d.rectangle((0, y, canvas.width, y+row_h), fill=background)
        x = pad
        for n, im, cell in zip(SIZES, images, widths):
            center = x+(cell-pad)//2
            canvas.paste(im, (center-n//2, y+label_h+(256-n)//2), im)
            d.text((center, y+row_h-17), f'{n} px', fill=foreground, font=font, anchor='mm')
            x += cell
    canvas.save(ROOT/'preview.png')


def main():
    images = [draw_icon(n) for n in SIZES]
    images[-1].save(ROOT/'weft.png')
    write_ico(images)
    make_preview(images)
    # Verify the directory exposes every requested size and RGBA is preserved.
    with Image.open(ROOT/'weft.ico') as ico:
        assert ico.ico.sizes() == {(n, n) for n in SIZES}
    with Image.open(ROOT/'weft.png') as png:
        assert png.size == (256, 256) and png.mode == 'RGBA'
    print('Created weft.png, weft.ico, and preview.png')


if __name__ == '__main__':
    main()
