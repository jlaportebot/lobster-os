#!/usr/bin/env python3
"""Regenerate the signed-index test vector used by the kernel's host tests.

    python3 tools/gen_pkg_fixture.py

Writes tests/fixtures/pkgrepo/{index.json,index.json.sig,public-key.hex}. The
key is derived from a fixed public seed: it only signs this fixture and is
unrelated to the catalogue key.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pkgrepo  # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402

TEST_SEED = b"lobster-pkgrepo-test-key-000000!"
OUT = os.path.join(pkgrepo.REPO_ROOT, "tests", "fixtures", "pkgrepo")
BASE = "http://127.0.0.1:8000"


def entry(name, version, kind, deps):
    data = (name + version).encode()
    return {
        "name": name, "version": version, "arch": "aarch64",
        "description": "Fixture package %s." % name, "details": "Longer text for %s." % name,
        "category": "Games" if kind == "gui" else "Developer", "icon": "AppGeneric",
        "kind": kind, "exec": "/apps/" + name, "dependencies": deps,
        "size": len(data), "installed_size": 2 * len(data),
        "checksum": pkgrepo.hashlib.sha256(data).hexdigest(),
        "download_url": "%s/%s-%s.lobpkg" % (BASE, name, version),
    }


def main():
    os.makedirs(OUT, exist_ok=True)
    index = pkgrepo.render_index(
        [entry("alpha", "1.10.0", "gui", []), entry("beta", "2.0.0", "cli", ["alpha"])], BASE)
    key = Ed25519PrivateKey.from_private_bytes(TEST_SEED)
    with open(os.path.join(OUT, "index.json"), "wb") as handle:
        handle.write(index)
    with open(os.path.join(OUT, "index.json.sig"), "w") as handle:
        handle.write(pkgrepo.sign_bytes(key, index) + "\n")
    with open(os.path.join(OUT, "public-key.hex"), "w") as handle:
        handle.write(pkgrepo.public_key_hex(key) + "\n")


if __name__ == "__main__":
    main()
