#!/usr/bin/env python3
"""Converte assets/font.png (exportado de assets/font.aseprite) em assets/pdrop.ttf.

Folha de 16 colunas, células de 6x11 px. Linha 9 da célula é a base (rows 0-8 acima, 9-10 descendente).
Cada pixel opaco vira um quadrado de 100 unidades; a largura do glifo é a última coluna usada + 1 px de respiro.
Uso: python3 scripts/font.py   (precisa de fontTools e Pillow)
"""
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from PIL import Image

CHARS = [chr(c) for c in range(32, 127)] + list("áàâãéêíóôõúüçÁÀÂÃÉÊÍÓÔÕÚÇ…›·↑↓←→")
CW, CH, BASE, U = 6, 11, 9, 100
ROOT = Path(__file__).resolve().parent.parent

im = Image.open(ROOT / "assets/font.png").convert("RGBA")
names, glyphs, metrics, cmap = [".notdef"], {}, {}, {}
empty = TTGlyphPen(None)
glyphs[".notdef"], metrics[".notdef"] = empty.glyph(), (3 * U, 0)

for k, ch in enumerate(CHARS):
    ox, oy = (k % 16) * CW, (k // 16) * CH
    pen = TTGlyphPen(None)
    right = -1
    for y in range(CH):
        x = 0
        while x < CW:
            if im.getpixel((ox + x, oy + y))[3] < 128:
                x += 1
                continue
            x0 = x
            while x < CW and im.getpixel((ox + x, oy + y))[3] >= 128:
                x += 1
            right = max(right, x - 1)
            top, bot = (BASE - y) * U, (BASE - y - 1) * U
            pen.moveTo((x0 * U, bot))
            pen.lineTo((x0 * U, top))
            pen.lineTo((x * U, top))
            pen.lineTo((x * U, bot))
            pen.closePath()
    name = f"uni{ord(ch):04X}"
    names.append(name)
    glyphs[name] = pen.glyph()
    adv = 4 if ch == " " else right + 2
    metrics[name] = (adv * U, 0)
    cmap[ord(ch)] = name

fb = FontBuilder(CH * U, isTTF=True)
fb.setupGlyphOrder(names)
fb.setupCharacterMap(cmap)
fb.setupGlyf(glyphs)
fb.setupHorizontalMetrics(metrics)
fb.setupHorizontalHeader(ascent=BASE * U, descent=-(CH - BASE) * U)
fb.setupNameTable({"familyName": "pdrop pixel", "styleName": "Regular"})
fb.setupOS2(sTypoAscender=BASE * U, sTypoDescender=-(CH - BASE) * U, sTypoLineGap=0, usWinAscent=BASE * U, usWinDescent=(CH - BASE) * U)
fb.setupPost()
fb.save(ROOT / "assets/pdrop.ttf")
print(f"assets/pdrop.ttf: {len(CHARS)} glifos")
