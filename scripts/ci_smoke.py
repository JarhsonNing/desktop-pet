import argparse
import json
import pathlib
import struct
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    arguments = parser.parse_args()
    binary = pathlib.Path(arguments.binary).resolve()
    output = pathlib.Path(arguments.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"binary": str(binary), "passed": False}
    with tempfile.TemporaryDirectory(prefix="mochi-smoke-") as directory:
        data = pathlib.Path(directory)
        (data / "config.json").write_text("{}", encoding="utf-8")
        screenshot = output / "control-panel.png"
        try:
            captured = subprocess.run([str(binary), "--data-dir", str(data), "--exit-after", "6", "--screenshot", str(screenshot)], capture_output=True, timeout=45)
            report["capture_exit_code"] = captured.returncode
            report["capture_stdout"] = captured.stdout.decode("utf-8", errors="replace")
            report["capture_stderr"] = captured.stderr.decode("utf-8", errors="replace")
            if captured.returncode != 0:
                raise RuntimeError("Native GUI screenshot failed; inspect stderr for renderer/display limitations")
            image = screenshot.read_bytes()
            if image[:8] != b"\x89PNG\r\n\x1a\n" or len(image) < 10000:
                raise RuntimeError("Missing or empty native framebuffer screenshot")
            width, height = struct.unpack(">II", image[16:24])
            report["screenshot_size"] = [width, height]
            pet = subprocess.run([str(binary), "--data-dir", str(data), "--exit-after", "5"], capture_output=True, timeout=30)
            report["pet_exit_code"] = pet.returncode
            report["pet_stderr"] = pet.stderr.decode("utf-8", errors="replace")
            if pet.returncode != 0:
                raise RuntimeError("Transparent native pet window failed")
            report["passed"] = True
        except (OSError, subprocess.TimeoutExpired, RuntimeError) as error:
            report["error"] = str(error)
    (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
