"""Fill a cover cache dir with generated art for every image_url in the fixture (file names = FNV-1a of the URL)."""
import json, sys, os, colorsys, math
from PIL import Image, ImageDraw
fixture, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
def fnv(s):
    h = 0xcbf29ce484222325
    for b in s.encode():
        h = ((h ^ b) * 0x100000001b3) & 0xffffffffffffffff
    return f"{h:016x}"
feed = json.load(open(fixture))
for i, url in enumerate(c["image_url"] for s in feed["shelves"] for c in s["cards"] if c.get("image_url")):
    hue = (i * 0.137) % 1
    a = tuple(int(v * 255) for v in colorsys.hsv_to_rgb(hue, 0.55, 0.75))
    b = tuple(int(v * 255) for v in colorsys.hsv_to_rgb((hue + 0.3) % 1, 0.7, 0.25))
    img = Image.new("RGB", (300, 300), b)
    d = ImageDraw.Draw(img)
    for r in range(140, 0, -10):
        t = r / 140
        col = tuple(int(a[k] * (1 - t) + b[k] * t) for k in range(3))
        d.ellipse([150 - r, 150 - r, 150 + r, 150 + r], fill=col)
    d.text((14, 270), f"#{i:03d}", fill=(240, 240, 240))
    img.save(os.path.join(out, fnv(url)), "JPEG", quality=88)
print("seeded", i + 1)
