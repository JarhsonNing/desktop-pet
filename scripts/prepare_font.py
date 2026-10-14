"""Prepare a complete, static Chinese font for UI and user-entered nicknames."""
import argparse
import io
import pathlib
import subprocess

from fontTools.ttLib import TTFont
from fontTools import subset
from fontTools.varLib.instancer import instantiateVariableFont

ROOT = pathlib.Path(__file__).resolve().parents[1]
FONT_URL = "https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/Variable/TTF/NotoSansCJKsc-VF.ttf"


def download(url):
    return subprocess.check_output(["curl", "--fail", "--location", "--retry", "3",
                                    "--max-time", "120", "--silent", "--show-error", url])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=pathlib.Path, help="Cached full Noto Sans CJK SC variable TTF")
    arguments = parser.parse_args()
    data = arguments.source.read_bytes() if arguments.source else download(FONT_URL)
    font = TTFont(io.BytesIO(data))
    if "fvar" in font:
        instantiateVariableFont(font, {"wght": 400}, inplace=True, optimize=True)
    font.flavor = None
    # Keep every Unicode character, but remove unused regional/vertical glyph
    # alternates. ttf-parser cannot read a 65,535-glyph font's 65,536-entry loca.
    characters = set(font.getBestCmap())
    options = subset.Options()
    options.layout_features = ["kern", "liga"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(unicodes=characters)
    subsetter.subset(font)
    if set(font.getBestCmap()) != characters:
        raise RuntimeError("Font conversion lost Unicode coverage")
    if len(font.getGlyphOrder()) >= 65535:
        raise RuntimeError("Font still exceeds the renderer's glyph limit")
    names = {1: "Mochi Sans", 2: "Regular", 3: "Mochi Sans Regular", 4: "Mochi Sans Regular",
             6: "MochiSans-Regular", 16: "Mochi Sans", 17: "Regular"}
    for record in font["name"].names:
        if record.nameID in names:
            record.string = names[record.nameID].encode(record.getEncoding())
    cmap = font.getBestCmap()
    for character in "麻薯曦玥珩璟麒麟團圓龍鳳𠮷":
        if ord(character) not in cmap:
            raise RuntimeError(f"Full font is missing U+{ord(character):04X}")
    output = ROOT / "assets" / "mochi-chinese.ttf"
    output.parent.mkdir(exist_ok=True)
    font.save(output)
    license_file = ROOT / "assets" / "FONT-LICENSE.txt"
    license_file.write_bytes(download("https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/LICENSE"))
    print("Prepared complete Mochi Sans:", len(cmap), "characters;", output.stat().st_size, "bytes")


if __name__ == "__main__":
    main()

