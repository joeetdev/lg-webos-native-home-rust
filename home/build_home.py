"""Offline, hash-pinned Home delta builder (Python 3.8+)."""
import hashlib
import json
from pathlib import Path
import argparse
import os
import re
import stat
import sys

TARGET_ROOT = "/usr/palm/applications/com.webos.app.home/qml/"
STAGING_ROOT = "/var/lib/lg-native/"


def safe_relative(value):
    if not isinstance(value, str) or not value or any(
            not re.fullmatch(r"[A-Za-z0-9_-][A-Za-z0-9_.-]*", part)
            or part in (".", "..") or part.endswith(".")
            for part in value.split("/")):
        raise ValueError("unsafe relative path")
    return value


def check_path(path, regular=False):
    """Reject symlinks and Windows reparse points in every path component."""
    path = Path(os.path.abspath(str(path)))
    for component in reversed((path,) + tuple(path.parents)):
        try:
            info = component.lstat()
        except FileNotFoundError:
            if component == path and not regular:
                continue
            raise ValueError("missing input or parent directory")
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError("symlink/reparse-point paths are not permitted")
    if regular and not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("input must be a regular file")
    return path


def validate_spec(specification):
    if (not isinstance(specification, dict) or set(specification) != {"format", "files"}
            or type(specification["format"]) is not int or specification["format"] != 1
            or not isinstance(specification["files"], list) or not specification["files"]):
        raise ValueError("unsupported delta schema")
    seen = set()
    for entry in specification["files"]:
        if not isinstance(entry, dict) or set(entry) != {
                "stock", "output", "target", "sourceSHA256", "targetSHA256", "edits"}:
            raise ValueError("invalid file entry")
        safe_relative(entry["stock"])
        safe_relative(entry["output"])
        if ("/" in entry["output"] or not entry["output"].startswith("native-")
                or entry["output"] != "native-" + entry["stock"].split("/")[-1]
                or entry["target"] != TARGET_ROOT + entry["stock"]):
            raise ValueError("output or target outside allowlist")
        for key in ("stock", "output", "target"):
            identity = (key, entry[key].casefold())
            if identity in seen:
                raise ValueError("duplicate file identity")
            seen.add(identity)
        for key in ("sourceSHA256", "targetSHA256"):
            if not isinstance(entry[key], str) or not re.fullmatch(r"[0-9a-f]{64}", entry[key]):
                raise ValueError("invalid SHA256 allowlist")
        if not isinstance(entry["edits"], list):
            raise ValueError("invalid edits")
    return specification["files"]


def build(stock_dir, out, specification, normalize_crlf=False):
    entries = validate_spec(specification)
    stock_dir, out = check_path(stock_dir), check_path(out)
    if not stock_dir.is_dir():
        raise ValueError("stock directory is missing")
    if out.exists():
        raise ValueError("output must be a new directory; refusing overwrite")
    if stock_dir == out or stock_dir in out.parents:
        raise ValueError("output must be outside stock directory")
    generated, manifest, sources = [], [], []
    # All source hashes must pass before any transform or output write.
    for entry in entries:
        data = check_path(stock_dir / entry["stock"], regular=True).read_bytes()
        if normalize_crlf:
            data = data.replace(b"\r\n", b"\n")
        if hashlib.sha256(data).hexdigest() != entry["sourceSHA256"]:
            raise ValueError("stock hash mismatch: " + entry["stock"])
        sources.append(data)
    for entry, data in zip(entries, sources):
        result = apply_delta(data, entry["edits"])
        if hashlib.sha256(result).hexdigest() != entry["targetSHA256"]:
            raise ValueError("output hash mismatch: " + entry["output"])
        generated.append((entry["output"], result))
        manifest.append({"source": "/var/lib/lg-native/" + entry["output"],
                         "target": entry["target"], "originalHash": entry["sourceSHA256"],
                         "newHash": entry["targetSHA256"]})
    out.mkdir()
    for name, data in generated:
        (out / name).write_bytes(data)
    (out / "home-final-manifest.json").write_bytes(
        (json.dumps(manifest, indent=2) + "\n").encode("utf-8"))
    return manifest


def apply_delta(source, edits):
    """Zero-based offsets refer to original lines, not intermediate results."""
    lines = source.splitlines(keepends=True)
    result, cursor, previous = [], 0, -1
    for edit in edits:
        if not isinstance(edit, dict) or set(edit) != {"start", "delete", "lines"}:
            raise ValueError("invalid edit schema")
        start, count, inserted = edit["start"], edit["delete"], edit["lines"]
        if (type(start) is not int or type(count) is not int or count < 0
                or start < cursor or start <= previous or start > len(lines)
                or start + count > len(lines) or not isinstance(inserted, list)):
            raise ValueError("invalid, overlapping or out-of-range edit")
        for line in inserted:
            if (not isinstance(line, str) or not line or "\r" in line
                    or len(line.encode("utf-8").splitlines(keepends=True)) != 1):
                raise ValueError("replacement entries must be single LF lines")
        result.extend(lines[cursor:start])
        result.extend(line.encode("utf-8") for line in inserted)
        cursor, previous = start + count, start
    result.extend(lines[cursor:])
    return b"".join(result)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stock-dir", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--normalize-crlf", action="store_true",
                        help="explicit CRLF to LF in-memory conversion before hashing")
    args = parser.parse_args(argv)
    try:
        delta_path = check_path(Path(__file__).with_name("deltas.json"), regular=True)
        specification = json.loads(delta_path.read_text(encoding="utf-8"))
        if len(validate_spec(specification)) != 8:
            raise ValueError("production allowlist must contain exactly eight files")
        manifest = build(args.stock_dir, args.out, specification, args.normalize_crlf)
    except (ValueError, OSError) as error:
        print("ERROR: " + (str(error) if isinstance(error, ValueError)
                           else "local filesystem operation failed"), file=sys.stderr)
        return 1
    print("Verified %d stock hashes and %d generated hashes; wrote 8 native sources and manifest."
          % (len(manifest), len(manifest)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
