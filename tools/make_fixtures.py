"""Generate tiny synthetic test fonts into fixtures/.

Every glyph is a box, so the files are a few KB and carry no licence. Each font
exercises one indexer edge case: typographic names, caps-only coverage,
Cyrillic-only coverage, a symbol font, a junk family name, a variable font,
OpenType features and a .ttc collection.

Run: python3 -I tools/make_fixtures.py fixtures
"""

import sys
from pathlib import Path

from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTCollection

UPM = 1000


def box(pen, x0, y0, x1, y1):
    pen.moveTo((x0, y0))
    pen.lineTo((x0, y1))
    pen.lineTo((x1, y1))
    pen.lineTo((x1, y0))
    pen.closePath()


def glyph_name(cp):
    return "space" if cp == 0x20 else f"uni{cp:04X}"


def build(chars, names, *, cff=False, weight=400, italic=False, features=None, variable=False):
    cps = sorted({0x20, *map(ord, chars)})
    order = [".notdef"] + [glyph_name(cp) for cp in cps]
    if features:
        order += ["f_i", "uni0061.ss01"]
    fb = FontBuilder(UPM, isTTF=not cff)
    fb.setupGlyphOrder(order)
    cmap = {cp: glyph_name(cp) for cp in cps}
    fb.setupCharacterMap(cmap)

    def draw(pen, name):
        if name == "space":
            return
        top = 500 if name.startswith("uni00") and name[-2:].islower() else 700
        box(pen, 50, 0, 550, top)

    if cff:
        charstrings = {}
        for name in order:
            pen = T2CharStringPen(600, None)
            draw(pen, name)
            charstrings[name] = pen.getCharString()
        fb.setupCFF(names["psName"], {"FullName": names["fullName"]}, charstrings, {})
    else:
        glyphs = {}
        for name in order:
            pen = TTGlyphPen(None)
            draw(pen, name)
            glyphs[name] = pen.glyph()
        fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (600, 50) for n in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable(names)
    fs_selection = 0x01 if italic else 0x40
    fb.setupOS2(
        usWeightClass=weight,
        fsSelection=fs_selection,
        achVendID="TSHF",
        sxHeight=500,
        sCapHeight=700,
        version=4,
    )
    fb.setupPost()
    if variable:
        fb.setupFvar([("wght", 100, 400, 900, "Weight")], [])
        fb.setupGvar({})
    if features:
        addOpenTypeFeaturesFromString(fb.font, features)
    return fb.font


def names(family, style="Regular", **extra):
    out = {
        "familyName": family,
        "styleName": style,
        "uniqueFontIdentifier": f"{family}-{style}",
        "fullName": f"{family} {style}",
        "psName": f"{family.replace(' ', '')}-{style}",
        "version": "Version 1.000",
    }
    out.update(extra)
    return out


LATIN = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.,"
UPPER = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
CYRILLIC = "".join(chr(c) for c in range(0x410, 0x450)) + "ё"
DINGBATS = "".join(chr(c) for c in range(0x2701, 0x2730))
FEATURES = """
languagesystem DFLT dflt;
languagesystem latn dflt;
feature liga { sub uni0066 uni0069 by f_i; } liga;
feature ss01 { sub uni0061 by uni0061.ss01; } ss01;
"""


def main(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    fonts = {
        # Typographic family (ID 16/17) differs from legacy family (ID 1/2).
        "FixtureSans-Light.ttf": build(
            LATIN,
            names(
                "Fixture Sans Light",
                "Regular",
                typographicFamily="Fixture Sans",
                typographicSubfamily="Light",
                psName="FixtureSans-Light",
            ),
            weight=300,
            features=FEATURES,
        ),
        "FixtureSans-Italic.otf": build(LATIN, names("Fixture Sans", "Italic"), cff=True, italic=True),
        "FixtureCaps-Regular.otf": build(UPPER, names("Fixture Caps"), cff=True),
        "FixtureCyr-Regular.ttf": build(CYRILLIC, names("Fixture Cyr")),
        "FixtureDingbats-Regular.ttf": build(DINGBATS, names("Fixture Dingbats")),
        "Junky-Regular.ttf": build(LATIN, names("-", "Regular", fullName="-", psName="-")),
        "FixtureVar[wght].ttf": build(LATIN, names("Fixture Var"), variable=True),
    }
    for file, font in fonts.items():
        font.save(out / file)

    ttc = TTCollection()
    ttc.fonts = [
        build(LATIN, names("Fixture Duo", "Regular")),
        build(LATIN, names("Fixture Duo", "Bold"), weight=700),
    ]
    ttc.save(out / "FixtureDuo.ttc")
    (out / "not-a-font.ttf").write_bytes(b"this is not a font file")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "fixtures")
