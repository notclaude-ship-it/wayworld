#!/usr/bin/env python3
import sys
from PIL import Image

ram = " .:-=+*#%@"
im = Image.open(sys.argv[1]).convert("RGB")
W = int(sys.argv[2]) if len(sys.argv) > 2 else 120
H = int(W * im.height / im.width * 0.5)
im = im.resize((W, H))
for y in range(H):
    row = ""
    for x in range(W):
        r, g, b = im.getpixel((x, y))
        l = (0.3 * r + 0.59 * g + 0.11 * b) / 255
        row += ram[min(9, int(l ** 0.6 * 10))]
    print(row)
st = Image.open(sys.argv[1]).convert("RGB").resize((1, 1)).getpixel((0, 0))
print("moyenne RGB:", st)