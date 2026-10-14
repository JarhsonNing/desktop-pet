import argparse
import pathlib
import plistlib
import shutil
import subprocess
import sys
import tomllib
import zipfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--version")
    arguments = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    if arguments.target not in {"aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc"}:
        raise SystemExit("Unsupported package target")
    if arguments.version is None:
        with (root / "Cargo.toml").open("rb") as source:
            arguments.version = tomllib.load(source)["package"]["version"]
    output = root / "dist"
    output.mkdir(exist_ok=True)
    executable = "desktop-pet.exe" if "windows" in arguments.target else "desktop-pet"
    binary = root / "target" / arguments.target / "release" / executable
    if not binary.is_file():
        binary = root / "target" / "release" / executable
    if not binary.is_file():
        raise SystemExit(f"Build the release binary first: {binary}")
    archive_name = f"Mochi-{arguments.version}-{arguments.target}"
    staging = output / archive_name
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir()
    if "apple" in arguments.target:
        application = staging / "Mochi.app"
        contents = application / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        resources = contents / "Resources"
        resources.mkdir()
        shutil.copy2(binary, contents / "MacOS" / "desktop-pet")
        icon = root / "assets" / "Mochi.icns"
        if icon.is_file():
            shutil.copy2(icon, resources / icon.name)
        metadata = {
            "CFBundleName": "Mochi", "CFBundleDisplayName": "麻薯 Mochi",
            "CFBundleIdentifier": "dev.jarhsonning.mochi", "CFBundleExecutable": "desktop-pet",
            "CFBundlePackageType": "APPL", "CFBundleShortVersionString": arguments.version,
            "CFBundleVersion": arguments.version, "LSMinimumSystemVersion": "12.0",
            "LSUIElement": True, "NSHighResolutionCapable": True,
        }
        if icon.is_file():
            metadata["CFBundleIconFile"] = "Mochi.icns"
        with (contents / "Info.plist").open("wb") as destination:
            plistlib.dump(metadata, destination)
        subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(application)], check=True)
    else:
        shutil.copy2(binary, staging / "Mochi.exe")
        (staging / "Restore-Desktop.cmd").write_text('@echo off\r\n"%~dp0Mochi.exe" --restore-desktop\r\n', encoding="utf-8")
    skin_output = staging / "Skins"
    skin_output.mkdir()
    for skin_name in ["default", "blue", "png"]:
        subprocess.run([str(binary), "--pack-skin", str(root / "skins" / skin_name), "--output", str(skin_output / f"{skin_name}.mochi")], check=True)
    for source in ["README.md", "LICENSE", "docs/RECOVERY.md", "docs/SKINS.md", "docs/CONTENT.md", "assets/FONT-LICENSE.txt"]:
        shutil.copy2(root / source, staging / pathlib.Path(source).name)
    archive = output / f"{archive_name}.zip"
    if sys.platform == "darwin":
        subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(staging), str(archive)], check=True)
    else:
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as destination:
            for path in staging.rglob("*"):
                if path.is_file():
                    destination.write(path, path.relative_to(output))
    print(archive)


if __name__ == "__main__":
    main()

