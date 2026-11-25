#!/usr/bin/env python3
"""Build, sign and verify the LobsterOS package catalogue.

    pkgrepo.py build <outdir> [--packages DIR] [--base-url URL]
    pkgrepo.py sign <outdir> --key KEYFILE
    pkgrepo.py verify <outdir> --pubkey HEX
    pkgrepo.py pubkey --key KEYFILE

`build` compiles every recipe under packages/ into `<name>-<version>.lobpkg`
and writes `index.json` next to them. `sign` writes `index.json.sig`: the
hex-encoded 64-byte ed25519 signature over the exact bytes of index.json
(the kernel's `pkg update` fetches both files and verifies before parsing).

A recipe is `packages/<name>/package.config` (key=value lines, `#` comments)
with these keys; paths are relative to the recipe directory:

    name, version, description, details, category, icon, kind, exec,
    license, maintainer, homepage, dependencies (comma separated)
    build_c = <src.c> [<src2.c> ...] -> <install path>   compile an EL0 program
    copy = <host file> -> <install path>                 ship a prebuilt file
    import_lobpkg = <file.lobpkg>                        repack an existing package

`build_c`, `copy` and `import_lobpkg` may each appear several times.
"""
import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys

REPO_ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
DEFAULT_PACKAGES = os.path.join(REPO_ROOT, "packages")
LINKER_SCRIPT = os.path.join(REPO_ROOT, "tools", "lobster_user.ld")
DEFAULT_BASE_URL = "https://github.com/jlaportebot/lobster-os/releases/download/pkg-repo"
REPO_NAME = "lobster-main"
CC = os.environ.get("CC", "aarch64-linux-gnu-gcc")
CFLAGS = [
    "-nostdlib", "-static", "-ffreestanding", "-fno-builtin",
    "-fno-tree-loop-distribute-patterns", "-O2", "-Wall", "-Wextra",
    "-Wl,-z,max-page-size=0x40", "-Wl,--build-id=none", "-Wl,--no-warn-rwx-segments",
]
LOBPKG_MAGIC = b"LOB\0"
LOBPKG_VERSION = 1
FILE_MODE_EXEC = 0o755
FILE_MODE_DATA = 0o644
ARCH = "aarch64"
REQUIRED_KEYS = ("name", "version", "description", "category", "icon", "exec")
KINDS = ("gui", "cli")
CATEGORIES = ("Games", "Utilities", "Developer", "System")
# The kernel's index parser is a plain string scanner: quotes, backslashes and
# control characters inside values would corrupt it.
FORBIDDEN_CHARS = '"\\\n\r\t'


class RecipeError(Exception):
    """A recipe is malformed or a build step failed."""


def parse_config(text):
    """Parse key=value lines; repeated keys collect into lists."""
    config = {}
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise RecipeError("line %d: expected key=value: %r" % (number, raw))
        key, value = line.split("=", 1)
        config.setdefault(key.strip(), []).append(value.strip())
    return config


def single(config, key, default=""):
    values = config.get(key, [])
    return values[0] if values else default


def validate(config, where):
    for key in REQUIRED_KEYS:
        if not single(config, key):
            raise RecipeError("%s: missing %s" % (where, key))
    for key, values in config.items():
        for value in values:
            if any(ch in value for ch in FORBIDDEN_CHARS):
                raise RecipeError("%s: %s contains a quote, backslash or control character" % (where, key))
    if single(config, "kind", "gui") not in KINDS:
        raise RecipeError("%s: kind must be one of %s" % (where, "/".join(KINDS)))
    if single(config, "category") not in CATEGORIES:
        raise RecipeError("%s: category must be one of %s" % (where, "/".join(CATEGORIES)))
    if not single(config, "exec").startswith("/"):
        raise RecipeError("%s: exec must be an absolute path" % where)


def split_arrow(spec, where):
    if "->" not in spec:
        raise RecipeError("%s: expected '<source> -> <path>': %r" % (where, spec))
    left, right = spec.split("->", 1)
    return left.split(), right.strip()


def compile_c(sources, recipe_dir, out_path):
    paths = [os.path.join(recipe_dir, s) for s in sources]
    cmd = [CC] + CFLAGS + ["-T", LINKER_SCRIPT, "-o", out_path] + paths
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        raise RecipeError("compile failed for %s:\n%s" % (sources, result.stderr))
    with open(out_path, "rb") as handle:
        return handle.read()


def read_u32(data, offset):
    return struct.unpack_from("<I", data, offset)[0], offset + 4


def read_string(data, offset):
    length, offset = read_u32(data, offset)
    return data[offset:offset + length].decode("utf-8"), offset + length


def skip_strings(data, offset, count):
    for _ in range(count):
        _, offset = read_string(data, offset)
    return offset


def skip_metadata(data, offset):
    """Skip the metadata block of a current-layout package."""
    offset = skip_strings(data, offset, 7)
    for _ in range(4):  # dependencies, provides, conflicts, replaces
        count, offset = read_u32(data, offset)
        offset = skip_strings(data, offset, count)
    offset += 8 + 8 + 32
    for _ in range(2):  # install and remove scripts
        length, offset = read_u32(data, offset)
        offset += length
    count, offset = read_u32(data, offset)
    return skip_strings(data, offset, count) + 8


def skip_legacy_metadata(data, offset):
    """Skip the metadata block written by tools/build_lobpkg.py (no provides/conflicts)."""
    offset = skip_strings(data, offset, 7)
    count, offset = read_u32(data, offset)
    offset = skip_strings(data, offset, count)
    offset += 8 + 8 + 32 + 8 + 2 + 4
    _, offset = read_string(data, offset)
    return offset


def read_file_table(data, offset):
    """Read the file entries and file data that follow the metadata block."""
    count, offset = read_u32(data, offset)
    entries = []
    for _ in range(count):
        path, offset = read_string(data, offset)
        mode, offset = read_u32(data, offset)
        offset += 4 + 4
        size = struct.unpack_from("<Q", data, offset)[0]
        offset += 8 + 32 + 1
        is_symlink = data[offset]
        offset += 1
        if is_symlink:
            _, offset = read_string(data, offset)
        entries.append((path, size, mode))
    total, offset = read_u32(data, offset)
    if offset + total != len(data) or sum(size for _, size, _ in entries) != total:
        raise ValueError("file table does not match the file data")
    files = []
    for path, size, mode in entries:
        files.append(("/" + path.lstrip("/"), data[offset:offset + size], mode))
        offset += size
    return files


def read_lobpkg_files(data):
    """Return [(absolute path, bytes, mode)] from a .lobpkg of either layout."""
    if data[:4] != LOBPKG_MAGIC:
        raise RecipeError("not a lobpkg")
    for skip in (skip_metadata, skip_legacy_metadata):
        try:
            return read_file_table(data, skip(data, 6))
        except (ValueError, UnicodeDecodeError, struct.error):
            continue
    raise RecipeError("unrecognised lobpkg layout")


def collect_files(config, recipe_dir, scratch):
    """Resolve a recipe into [(absolute install path, bytes, mode)]."""
    files = []
    for index, spec in enumerate(config.get("build_c", [])):
        sources, target = split_arrow(spec, "build_c")
        out_path = os.path.join(scratch, "build-%d" % index)
        files.append((target, compile_c(sources, recipe_dir, out_path), FILE_MODE_EXEC))
    for spec in config.get("copy", []):
        sources, target = split_arrow(spec, "copy")
        with open(os.path.join(recipe_dir, sources[0]), "rb") as handle:
            files.append((target, handle.read(), FILE_MODE_EXEC))
    for spec in config.get("import_lobpkg", []):
        with open(os.path.join(recipe_dir, spec), "rb") as handle:
            files.extend(read_lobpkg_files(handle.read()))
    if not files:
        raise RecipeError("recipe produces no files")
    exec_path = single(config, "exec")
    files.sort(key=lambda entry: entry[0] != exec_path)
    if files[0][0] != exec_path:
        raise RecipeError("exec %s is not among the package files" % exec_path)
    return files


def pack_string(text):
    raw = text.encode("utf-8")
    return struct.pack("<I", len(raw)) + raw


def pack_list(items):
    return struct.pack("<I", len(items)) + b"".join(pack_string(i) for i in items)


def split_deps(config):
    raw = single(config, "dependencies")
    return [d.strip() for d in raw.split(",") if d.strip()]


def write_lobpkg(config, files):
    """Serialise a package in the layout `parse_package` reads (src/pkg/install.rs)."""
    file_data = b"".join(data for _, data, _ in files)
    installed_size = len(file_data)
    out = bytearray(LOBPKG_MAGIC + struct.pack("<H", LOBPKG_VERSION))
    for key in ("name", "version"):
        out += pack_string(single(config, key))
    out += pack_string(single(config, "arch", ARCH))
    out += pack_string(single(config, "description"))
    out += pack_string(single(config, "maintainer", "LobsterOS"))
    out += pack_string(single(config, "license", "MIT"))
    out += pack_string(single(config, "homepage"))
    out += pack_list(split_deps(config))
    out += pack_list([]) + pack_list([]) + pack_list([])  # provides, conflicts, replaces
    out += struct.pack("<QQ", installed_size, 0)
    out += hashlib.sha256(file_data).digest()
    out += struct.pack("<II", 0, 0)  # no install / remove script
    out += pack_list([])  # config files
    out += struct.pack("<Q", 0)  # build timestamp: fixed so builds are reproducible
    out += struct.pack("<I", len(files))
    for path, data, mode in files:
        out += pack_string(path) + struct.pack("<III", mode, 0, 0)
        out += struct.pack("<Q", len(data)) + hashlib.sha256(data).digest() + b"\0\0"
    out += struct.pack("<I", len(file_data)) + file_data
    return bytes(out), installed_size


def package_entry(config, blob, installed_size, base_url):
    name, version = single(config, "name"), single(config, "version")
    return {
        "name": name,
        "version": version,
        "arch": single(config, "arch", ARCH),
        "description": single(config, "description"),
        "details": single(config, "details", single(config, "description")),
        "category": single(config, "category"),
        "icon": single(config, "icon"),
        "kind": single(config, "kind", "gui"),
        "exec": single(config, "exec"),
        "dependencies": split_deps(config),
        "size": len(blob),
        "installed_size": installed_size,
        "checksum": hashlib.sha256(blob).hexdigest(),
        "download_url": "%s/%s-%s.lobpkg" % (base_url.rstrip("/"), name, version),
    }


def render_index(entries, base_url):
    index = {"name": REPO_NAME, "url": base_url.rstrip("/"), "packages": entries}
    return (json.dumps(index, indent=2, ensure_ascii=True) + "\n").encode("ascii")


def build_recipe(recipe_dir, outdir, base_url, scratch):
    config_path = os.path.join(recipe_dir, "package.config")
    with open(config_path, encoding="utf-8") as handle:
        config = parse_config(handle.read())
    validate(config, config_path)
    files = collect_files(config, recipe_dir, scratch)
    blob, installed_size = write_lobpkg(config, files)
    name, version = single(config, "name"), single(config, "version")
    with open(os.path.join(outdir, "%s-%s.lobpkg" % (name, version)), "wb") as handle:
        handle.write(blob)
    return package_entry(config, blob, installed_size, base_url)


def recipe_dirs(packages_dir):
    names = sorted(os.listdir(packages_dir))
    return [os.path.join(packages_dir, n) for n in names
            if os.path.isfile(os.path.join(packages_dir, n, "package.config"))]


def build(outdir, packages_dir=DEFAULT_PACKAGES, base_url=DEFAULT_BASE_URL):
    os.makedirs(outdir, exist_ok=True)
    scratch = os.path.join(outdir, ".build")
    os.makedirs(scratch, exist_ok=True)
    entries = [build_recipe(d, outdir, base_url, scratch) for d in recipe_dirs(packages_dir)]
    names = [e["name"] for e in entries]
    if len(set(names)) != len(names):
        raise RecipeError("duplicate package names: %s" % names)
    with open(os.path.join(outdir, "index.json"), "wb") as handle:
        handle.write(render_index(entries, base_url))
    for leftover in os.listdir(scratch):
        os.remove(os.path.join(scratch, leftover))
    os.rmdir(scratch)
    return entries


def load_private_key(path):
    from cryptography.hazmat.primitives import serialization
    with open(path, "rb") as handle:
        return serialization.load_pem_private_key(handle.read(), password=None)


def public_key_hex(private_key):
    from cryptography.hazmat.primitives import serialization
    raw = private_key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    return raw.hex()


def sign_bytes(private_key, data):
    return private_key.sign(data).hex()


def verify_signature(pubkey_hex, data, signature_hex):
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    try:
        key = Ed25519PublicKey.from_public_bytes(bytes.fromhex(pubkey_hex))
        key.verify(bytes.fromhex(signature_hex.strip()), data)
    except (InvalidSignature, ValueError):
        return False
    return True


def verify_dir(outdir, pubkey_hex):
    """Check the signature, then every package's size and sha256 against the index."""
    with open(os.path.join(outdir, "index.json"), "rb") as handle:
        index_bytes = handle.read()
    with open(os.path.join(outdir, "index.json.sig"), encoding="ascii") as handle:
        signature = handle.read()
    if not verify_signature(pubkey_hex, index_bytes, signature):
        raise RecipeError("index.json signature does not verify")
    for pkg in json.loads(index_bytes)["packages"]:
        path = os.path.join(outdir, os.path.basename(pkg["download_url"]))
        with open(path, "rb") as handle:
            blob = handle.read()
        if len(blob) != pkg["size"] or hashlib.sha256(blob).hexdigest() != pkg["checksum"]:
            raise RecipeError("%s does not match the index" % pkg["name"])
    return True


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p_build = sub.add_parser("build")
    p_build.add_argument("outdir")
    p_build.add_argument("--packages", default=DEFAULT_PACKAGES)
    p_build.add_argument("--base-url", default=DEFAULT_BASE_URL)
    p_sign = sub.add_parser("sign")
    p_sign.add_argument("outdir")
    p_sign.add_argument("--key", required=True)
    p_verify = sub.add_parser("verify")
    p_verify.add_argument("outdir")
    p_verify.add_argument("--pubkey", required=True)
    p_pub = sub.add_parser("pubkey")
    p_pub.add_argument("--key", required=True)
    args = parser.parse_args(argv)
    try:
        return run(args)
    except (RecipeError, OSError) as err:
        print("pkgrepo: %s" % err, file=sys.stderr)
        return 1


def run(args):
    if args.command == "build":
        entries = build(args.outdir, args.packages, args.base_url)
        for entry in entries:
            print("%-14s %-8s %7d bytes" % (entry["name"], entry["version"], entry["size"]))
    elif args.command == "sign":
        key = load_private_key(args.key)
        with open(os.path.join(args.outdir, "index.json"), "rb") as handle:
            signature = sign_bytes(key, handle.read())
        with open(os.path.join(args.outdir, "index.json.sig"), "w", encoding="ascii") as handle:
            handle.write(signature + "\n")
        print("signed index.json (public key %s)" % public_key_hex(key))
    elif args.command == "verify":
        verify_dir(args.outdir, args.pubkey)
        print("index.json signature and package checksums OK")
    else:
        print(public_key_hex(load_private_key(args.key)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
