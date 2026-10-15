"""Prepare the only supported Pi runtime; never consult a user's Pi installation."""
import argparse
import hashlib
import json
import pathlib
import shutil
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = json.loads((ROOT / "scripts/pi-runtime.json").read_text(encoding="utf-8"))


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def download(url, path, expected):
    if not path.is_file() or digest(path) != expected:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(".download")
        try:
            urllib.request.urlretrieve(url, temporary)
            if digest(temporary) != expected:
                raise ValueError(f"Pi SHA-256 mismatch: {path.name}")
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)


def prepare(target, destination):
    entry = MANIFEST["targets"][target]
    cache = ROOT / ".artifacts/pi-downloads"
    archive = cache / entry["asset"]
    download(MANIFEST["release"] + entry["asset"], archive, entry["sha256"])
    destination = pathlib.Path(destination).resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pi-", dir=destination.parent) as temporary:
        extracted = pathlib.Path(temporary)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as source:
                for item in source.infolist():
                    if not (extracted / item.filename).resolve().is_relative_to(extracted):
                        raise ValueError("Unsafe Pi archive entry")
                source.extractall(extracted)
        else:
            with tarfile.open(archive) as source:
                source.extractall(extracted, filter="data")
        executable = "pi.exe" if "windows" in target else "pi"
        candidates = list(extracted.rglob(executable))
        if len(candidates) != 1:
            raise ValueError("Pi archive must contain exactly one runtime executable")
        source_root = candidates[0].parent
        license_entry = MANIFEST["license"]
        download(license_entry["url"], cache / "LICENSE", license_entry["sha256"])
        shutil.copy2(cache / "LICENSE", source_root / "LICENSE")
        files = {p.relative_to(source_root).as_posix(): digest(p)
                 for p in sorted(source_root.rglob("*")) if p.is_file()}
        metadata = {"version": MANIFEST["version"], "target": target,
                    "archive_sha256": entry["sha256"], "files": files}
        (source_root / "mochi-runtime.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
        destination.mkdir(parents=True, exist_ok=True)
        shutil.copytree(source_root, destination, dirs_exist_ok=True)
    return destination


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=MANIFEST["targets"])
    parser.add_argument("--destination", required=True)
    args = parser.parse_args()
    print(prepare(args.target, args.destination))
