import hashlib
import json
import pathlib
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import prepare_pi


class RuntimePackagingTests(unittest.TestCase):
    def test_checksum_failure_preserves_existing_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            artifact = pathlib.Path(directory) / "pi.zip"
            artifact.write_bytes(b"old-cache")
            def retrieve(_url, destination):
                pathlib.Path(destination).write_bytes(b"tampered")
            with patch.object(prepare_pi.urllib.request, "urlretrieve", retrieve):
                with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                    prepare_pi.download("fixture", artifact, hashlib.sha256(b"expected").hexdigest())
            self.assertEqual(artifact.read_bytes(), b"old-cache")
            self.assertFalse(artifact.with_suffix(".download").exists())

    def test_archive_escape_is_rejected(self):
        self.prepare_fixture({"../outside": b"escape", "pi/pi.exe": b"binary"}, reject=True)

    def test_complete_runtime_and_license_have_file_hashes(self):
        self.prepare_fixture({"pi/pi.exe": b"binary", "pi/package.json": b"{}", "pi/native/helper.dll": b"native"})

    def prepare_fixture(self, files, reject=False):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            archive = root / "source.zip"
            with zipfile.ZipFile(archive, "w") as output:
                for name, content in files.items():
                    output.writestr(name, content)
            license_content = b"MIT fixture license"
            manifest = {"version": "1.2.0", "release": "fixture:", "targets": {
                "x86_64-pc-windows-msvc": {"asset": "pi.zip", "sha256": prepare_pi.digest(archive)}},
                "license": {"url": "license:", "sha256": hashlib.sha256(license_content).hexdigest()}}
            def retrieve(url, destination):
                pathlib.Path(destination).write_bytes(license_content if url == "license:" else archive.read_bytes())
            with patch.object(prepare_pi, "ROOT", root), patch.object(prepare_pi, "MANIFEST", manifest), patch.object(prepare_pi.urllib.request, "urlretrieve", retrieve):
                destination = root / "runtime/pi"
                if reject:
                    with self.assertRaisesRegex(ValueError, "Unsafe Pi archive"):
                        prepare_pi.prepare("x86_64-pc-windows-msvc", destination)
                    self.assertFalse((root / "outside").exists())
                    return
                prepare_pi.prepare("x86_64-pc-windows-msvc", destination)
                metadata = json.loads((destination / "mochi-runtime.json").read_text())
                self.assertEqual(metadata["files"]["native/helper.dll"], hashlib.sha256(b"native").hexdigest())
                self.assertEqual(metadata["files"]["LICENSE"], hashlib.sha256(license_content).hexdigest())


if __name__ == "__main__":
    unittest.main()
