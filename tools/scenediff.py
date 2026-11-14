"""Two sets of scenes (`cargo test -p weft scenes -- --ignored`, drawn by
two versions of weft) compared pixel by pixel: which are the same, how much
of the others differs and in which rows; and for those, both pictures and
the difference side by side.

    python tools/scenediff.py <folder a> <folder b> [<folder for the comparisons>]

What differs is either meant (something was added) or a regression. Needs
Pillow (`pip install pillow`).
"""
import os, sys
from PIL import Image, ImageChops, ImageDraw


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    a_dir, b_dir = sys.argv[1], sys.argv[2]
    out = sys.argv[3] if len(sys.argv) > 3 else os.path.join(os.path.dirname(os.path.abspath(b_dir)), 'diff')
    os.makedirs(out, exist_ok=True)
    same = 0
    names = sorted(n for n in os.listdir(a_dir) if n.endswith('.png'))
    for name in names:
        if not os.path.exists(os.path.join(b_dir, name)):
            print(f'{name:28} only in {a_dir}')
            continue
        a = Image.open(os.path.join(a_dir, name)).convert('RGB')
        b = Image.open(os.path.join(b_dir, name)).convert('RGB')
        if a.size != b.size:
            print(f'{name:28} sizes differ: {a.size} and {b.size}')
            continue
        # A pixel differs when any colour is off by more than a tenth.
        diff = ImageChops.difference(a, b).convert('L').point(lambda v: 255 if v > 24 else 0)
        box = diff.getbbox()
        if not box:
            print(f'{name:28} same')
            same += 1
            continue
        n = diff.histogram()[255]
        rows = [y for y in range(diff.height) if diff.crop((0, y, diff.width, y + 1)).getbbox()]
        bands, start, prev = [], rows[0], rows[0]
        for y in rows[1:]:
            if y > prev + 6:
                bands.append((start, prev))
                start = y
            prev = y
        bands.append((start, prev))
        print(f'{name:28} {n:6} px differ ({100.0 * n / (a.width * a.height):.2f} %), x {box[0]}..{box[2]}, rows {bands[:8]}')
        sheet = Image.new('RGB', (a.width * 3, a.height + 16), 'white')
        d = ImageDraw.Draw(sheet)
        for i, (img, label) in enumerate([(a, a_dir), (b, b_dir), (diff.convert('RGB'), 'what differs')]):
            d.text((i * a.width + 4, 2), label, fill='black')
            sheet.paste(img, (i * a.width, 16))
        sheet.save(os.path.join(out, name))
    print(f'{same} of {len(names)} the same; comparisons of the others in {out}')


if __name__ == '__main__':
    main()
