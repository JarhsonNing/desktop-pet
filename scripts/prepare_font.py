import io
import pathlib
import re
import subprocess
import urllib.parse

from fontTools.ttLib import TTFont


ROOT = pathlib.Path(__file__).resolve().parents[1]


def download(url):
    return subprocess.check_output(["curl", "--fail", "--location", "--retry", "3", "--retry-all-errors", "--max-time", "90", "--silent", "--show-error", url])


def main():
    text = "".join(path.read_text() for path in (ROOT / "src").rglob("*.rs"))
    characters = "".join(sorted(set(text + "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz，。！？·：；（） /+-_%")))
    url = "https://fonts.googleapis.com/css2?family=Noto+Sans+SC:wght@400&text=" + urllib.parse.quote(characters)
    css = download(url).decode()
    font_url = re.search(r"url\((https://[^)]+)\)", css).group(1)
    font = TTFont(io.BytesIO(download(font_url)))
    font.flavor = None
    for record in font["name"].names:
        if record.nameID in (1, 3, 4, 6, 16):
            record.string = "Mochi Sans".encode(record.getEncoding())
    (ROOT / "assets").mkdir(exist_ok=True)
    font.save(ROOT / "assets" / "mochi-chinese.ttf")
    license_file = ROOT / "assets" / "FONT-LICENSE.txt"
    if not license_file.exists():
        license_file.write_bytes(download("https://raw.githubusercontent.com/google/fonts/main/ofl/notosanssc/OFL.txt"))
    print("Prepared Mochi Sans, a renamed Noto Sans SC subset:", (ROOT / "assets" / "mochi-chinese.ttf").stat().st_size, "bytes")


if __name__ == "__main__":
    main()

