#!/usr/bin/env python3
"""Unit tests for pkgrepo.py: python3 -m unittest tools/test_pkgrepo.py"""
import hashlib
import json
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pkgrepo  # noqa: E402

CONFIG = """# demo
name=demo
version=1.2.3
description=Demo package.
category=Games
icon=AppGeneric
kind=gui
exec=/apps/demo
copy=payload.bin -> /apps/demo
"""


def make_recipe(root, config=CONFIG, payload=b"\x7fELFdemo"):
    recipe = os.path.join(root, "packages", "demo")
    os.makedirs(recipe)
    with open(os.path.join(recipe, "package.config"), "w") as handle:
        handle.write(config)
    with open(os.path.join(recipe, "payload.bin"), "wb") as handle:
        handle.write(payload)
    return os.path.join(root, "packages")


class PkgrepoTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(dir=os.environ.get("TMPDIR"))
        self.addCleanup(shutil.rmtree, self.root, True)

    def build(self, **kwargs):
        out = os.path.join(self.root, "out")
        pkgrepo.build(out, make_recipe(self.root, **kwargs), "http://127.0.0.1:8000")
        return out

    def test_parse_config_collects_repeated_keys(self):
        config = pkgrepo.parse_config("a=1\n# c\n\ncopy=x -> /a\ncopy=y -> /b\n")
        self.assertEqual(config["copy"], ["x -> /a", "y -> /b"])

    def test_parse_config_rejects_line_without_equals(self):
        with self.assertRaises(pkgrepo.RecipeError):
            pkgrepo.parse_config("nonsense")

    def test_validate_rejects_missing_field_and_bad_values(self):
        for broken in (CONFIG.replace("name=demo\n", ""),
                       CONFIG.replace("Games", "Fruit"),
                       CONFIG.replace("Demo package.", 'Say "hi"'),
                       CONFIG.replace("kind=gui", "kind=tui"),
                       CONFIG.replace("exec=/apps/demo", "exec=apps/demo")):
            with self.assertRaises(pkgrepo.RecipeError, msg=broken):
                pkgrepo.validate(pkgrepo.parse_config(broken), "t")

    def test_index_lists_package_with_matching_checksum_and_size(self):
        out = self.build()
        index = json.load(open(os.path.join(out, "index.json")))
        entry = index["packages"][0]
        blob = open(os.path.join(out, "demo-1.2.3.lobpkg"), "rb").read()
        self.assertEqual(entry["checksum"], hashlib.sha256(blob).hexdigest())
        self.assertEqual(entry["size"], len(blob))
        self.assertEqual(entry["download_url"], "http://127.0.0.1:8000/demo-1.2.3.lobpkg")
        self.assertEqual(entry["category"], "Games")
        self.assertEqual(entry["dependencies"], [])

    def test_lobpkg_roundtrips_files_with_absolute_paths(self):
        out = self.build()
        blob = open(os.path.join(out, "demo-1.2.3.lobpkg"), "rb").read()
        self.assertEqual(pkgrepo.read_lobpkg_files(blob), [("/apps/demo", b"\x7fELFdemo", 0o755)])

    def test_reads_legacy_layout_ncc_package(self):
        legacy = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ncc-1.0.0.lobpkg")
        paths = [path for path, _, _ in pkgrepo.read_lobpkg_files(open(legacy, "rb").read())]
        self.assertIn("/usr/bin/ncc", paths)
        self.assertIn("/usr/lib/libc.a", paths)

    def test_package_checksum_field_covers_file_data(self):
        out = self.build()
        blob = open(os.path.join(out, "demo-1.2.3.lobpkg"), "rb").read()
        self.assertIn(hashlib.sha256(b"\x7fELFdemo").digest(), blob)

    def test_build_is_reproducible(self):
        first = open(os.path.join(self.build(), "demo-1.2.3.lobpkg"), "rb").read()
        shutil.rmtree(os.path.join(self.root, "out"))
        shutil.rmtree(os.path.join(self.root, "packages"))
        second = open(os.path.join(self.build(), "demo-1.2.3.lobpkg"), "rb").read()
        self.assertEqual(first, second)

    def test_exec_file_is_listed_first(self):
        config = pkgrepo.parse_config("exec=/usr/bin/tool\ncopy=p -> /usr/bin/tool\n")
        recipe = os.path.join(self.root, "r")
        os.makedirs(recipe)
        open(os.path.join(recipe, "p"), "wb").write(b"b")
        config["copy"].append("p -> /usr/share/a")
        got = pkgrepo.collect_files(config, recipe, self.root)
        self.assertEqual(got[0][0], "/usr/bin/tool")

    def test_missing_exec_file_is_an_error(self):
        config = pkgrepo.parse_config("exec=/apps/nothing\ncopy=p -> /apps/x\n")
        recipe = os.path.join(self.root, "r")
        os.makedirs(recipe)
        open(os.path.join(recipe, "p"), "wb").write(b"b")
        with self.assertRaises(pkgrepo.RecipeError):
            pkgrepo.collect_files(config, recipe, self.root)

    def sign(self, out):
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        key = Ed25519PrivateKey.generate()
        data = open(os.path.join(out, "index.json"), "rb").read()
        sig = pkgrepo.sign_bytes(key, data)
        open(os.path.join(out, "index.json.sig"), "w").write(sig + "\n")
        return pkgrepo.public_key_hex(key)

    def test_signature_verifies_and_rejects_tampering(self):
        out = self.build()
        pub = self.sign(out)
        self.assertTrue(pkgrepo.verify_dir(out, pub))
        path = os.path.join(out, "index.json")
        data = bytearray(open(path, "rb").read())
        data[10] ^= 0x01
        open(path, "wb").write(bytes(data))
        with self.assertRaises(pkgrepo.RecipeError):
            pkgrepo.verify_dir(out, pub)

    def test_signature_rejected_for_other_key(self):
        out = self.build()
        self.sign(out)
        other = self.sign_other()
        with self.assertRaises(pkgrepo.RecipeError):
            pkgrepo.verify_dir(out, other)

    def sign_other(self):
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        return pkgrepo.public_key_hex(Ed25519PrivateKey.generate())

    def test_tampered_package_fails_verification(self):
        out = self.build()
        pub = self.sign(out)
        with open(os.path.join(out, "demo-1.2.3.lobpkg"), "ab") as handle:
            handle.write(b"x")
        with self.assertRaises(pkgrepo.RecipeError):
            pkgrepo.verify_dir(out, pub)

    def test_verify_signature_rejects_garbage(self):
        self.assertFalse(pkgrepo.verify_signature("00" * 32, b"data", "zz"))

    def test_duplicate_package_names_are_rejected(self):
        packages = make_recipe(self.root)
        shutil.copytree(os.path.join(packages, "demo"), os.path.join(packages, "demo2"))
        with self.assertRaises(pkgrepo.RecipeError):
            pkgrepo.build(os.path.join(self.root, "out"), packages, "http://x")


if __name__ == "__main__":
    unittest.main()
