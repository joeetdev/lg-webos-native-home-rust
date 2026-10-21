import importlib.util
import unittest
import tempfile
import hashlib
import json
import sys
import os

sys.dont_write_bytecode = True
from pathlib import Path

MODULE = Path(__file__).resolve().parents[1] / "build_home.py"


class DeltaTests(unittest.TestCase):
    def test_offset_replacement_preserves_untouched_bytes(self):
        self.assertTrue(MODULE.is_file(), "offline builder is not implemented")
        spec = importlib.util.spec_from_file_location("build_home", MODULE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertEqual(module.apply_delta(b"alpha\nbeta\ngamma\n", [
            {"start": 1, "delete": 1, "lines": ["custom\n"]},
            {"start": 3, "delete": 0, "lines": ["tail"]},
        ]), b"alpha\ncustom\ngamma\ntail")


class BuilderTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("build_home", MODULE)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.stock = self.root / "stock"
        self.stock.mkdir()
        (self.stock / "part.qml").write_bytes(b"stock\n")
        self.out = self.root / "output"
        digest = lambda data: hashlib.sha256(data).hexdigest()
        self.entry = {"stock": "part.qml", "output": "native-part.qml",
                      "target": "/usr/palm/applications/com.webos.app.home/qml/part.qml",
                      "sourceSHA256": digest(b"stock\n"), "targetSHA256": digest(b"custom\n"),
                      "edits": [{"start": 0, "delete": 1, "lines": ["custom\n"]}]}

    def test_build_emits_exact_bytes_and_manual_staging_manifest(self):
        self.assertTrue(hasattr(self.module, "build"), "bundle builder is missing")
        self.module.build(self.stock, self.out, {"format": 1, "files": [self.entry]})
        self.assertEqual((self.out / "native-part.qml").read_bytes(), b"custom\n")
        manifest = json.loads((self.out / "home-final-manifest.json").read_text())
        self.assertEqual(manifest, [{"source": "/var/lib/lg-native/native-part.qml",
                                    "target": self.entry["target"],
                                    "originalHash": self.entry["sourceSHA256"],
                                    "newHash": self.entry["targetSHA256"]}])

    def test_rejects_unsafe_schema_before_creating_output(self):
        import copy
        cases = [("stock", "../part.qml"), ("stock", "/part.qml"),
                 ("stock", "C:/part.qml"), ("stock", "a\\part.qml"),
                 ("output", "../native-part.qml"), ("output", "part.qml"),
                 ("target", "/etc/config"), ("sourceSHA256", "invalid"),
                 ("targetSHA256", "0" * 64), ("edits", [{"start": -1, "delete": 0, "lines": []}]),
                 ("edits", [{"start": 0, "delete": 9, "lines": []}]),
                 ("edits", [{"start": True, "delete": 0, "lines": []}]),
                 ("edits", [{"start": 0, "delete": 1, "lines": "custom\n"}]),
                 ("edits", [{"start": 0, "delete": 1, "lines": ["a\nb\n"]}]),
                 ("edits", [{"start": 1, "delete": 0, "lines": []},
                            {"start": 0, "delete": 0, "lines": []}])]
        for key, value in cases:
            with self.subTest(key=key, value=value):
                entry = copy.deepcopy(self.entry)
                entry[key] = value
                with self.assertRaises(ValueError):
                    self.module.build(self.stock, self.out, {"format": 1, "files": [entry]})
                self.assertFalse(self.out.exists())
        for specification in [{"format": 2, "files": [self.entry]},
                              {"format": 1, "files": []},
                              {"format": 1, "files": [self.entry, self.entry]}]:
            with self.subTest(specification=specification):
                with self.assertRaises(ValueError):
                    self.module.build(self.stock, self.out, specification)
                self.assertFalse(self.out.exists())

    def test_all_source_hashes_checked_before_any_transform(self):
        import copy
        second = copy.deepcopy(self.entry)
        second.update(stock="second.qml", output="native-second.qml",
                      target="/usr/palm/applications/com.webos.app.home/qml/second.qml")
        (self.stock / "second.qml").write_bytes(b"wrong firmware\n")
        self.entry["edits"] = [{"start": 99, "delete": 0, "lines": []}]
        with self.assertRaisesRegex(ValueError, "stock hash mismatch: second.qml"):
            self.module.build(self.stock, self.out, {"format": 1, "files": [self.entry, second]})
        self.assertFalse(self.out.exists())

    def test_explicit_crlf_opt_in(self):
        (self.stock / "part.qml").write_bytes(b"stock\r\n")
        with self.assertRaisesRegex(ValueError, "stock hash mismatch"):
            self.module.build(self.stock, self.out, {"format": 1, "files": [self.entry]})
        self.assertFalse(self.out.exists())
        self.module.build(self.stock, self.out, {"format": 1, "files": [self.entry]}, normalize_crlf=True)
        self.assertEqual((self.out / "native-part.qml").read_bytes(), b"custom\n")

    def test_missing_file_rejected_without_output(self):
        (self.stock / "part.qml").unlink()
        with self.assertRaises((ValueError, FileNotFoundError)):
            self.module.build(self.stock, self.out, {"format": 1, "files": [self.entry]})
        self.assertFalse(self.out.exists())

    def test_symlink_input_rejected_even_if_hash_matches(self):
        data = self.root / "real.qml"
        data.write_bytes(b"stock\n")
        link = self.stock / "part.qml"
        link.unlink()
        try:
            link.symlink_to(data)
        except OSError as error:
            self.skipTest("OS does not permit symlink creation: " + str(error.errno))
        with self.assertRaises(ValueError):
            self.module.build(self.stock, self.out, {"format": 1, "files": [self.entry]})
        self.assertFalse(self.out.exists())


if __name__ == "__main__":
    unittest.main()
