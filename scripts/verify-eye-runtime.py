"""Read-only, offline integrity inspection for an optional Windows eye runtime.

This does not load DLLs, open media, accept a license, or establish GPU support.
The expected manifest digest must come from a separately reviewed source.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys
from urllib.parse import urlsplit


PACKAGES = {"ar_sdk_core", "nvARGazeRedirection", "nvARLandmarkDetection", "nvARFaceBoxDetection"}
MAX_MANIFEST_BYTES = 4 * 1024 * 1024
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
VERSION = re.compile(r"[0-9]+(?:\.[0-9]+){1,3}(?:[_-][A-Za-z0-9.]+)?\Z")
RESERVED = re.compile(r"(?:CON|PRN|AUX|NUL|COM[1-9¹²³]|LPT[1-9¹²³])(?:\..*)?\Z", re.I)


class Rejected(Exception):
    def __init__(self, code, file=None):
        super().__init__(code)
        self.code = code
        self.file = file


def require(condition, code, file=None):
    if not condition:
        raise Rejected(code, file)


def exact_keys(value, keys):
    require(isinstance(value, dict) and set(value) == set(keys), "invalid_schema")


def no_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def is_hash(value):
    return isinstance(value, str) and SHA256.fullmatch(value) is not None


def safe_relative(value):
    require(isinstance(value, str) and 0 < len(value) <= 2048, "invalid_path")
    parts = value.split("/")
    require(all(part not in {"", ".", ".."} for part in parts), "invalid_path")
    require(not any(ord(c) < 32 or 0xD800 <= ord(c) <= 0xDFFF or c in '\\:<>"|?*' for c in value), "invalid_path")
    require(all(not part.endswith((" ", ".")) and not RESERVED.fullmatch(part) for part in parts), "invalid_path")
    return parts


def metadata(path):
    info = path.lstat()
    require(not stat.S_ISLNK(info.st_mode) and not (
        getattr(info, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT
    ), "linked_path")
    require(stat.S_ISDIR(info.st_mode) or stat.S_ISREG(info.st_mode), "non_regular_file")
    if stat.S_ISREG(info.st_mode):
        require(info.st_nlink == 1, "hardlinked_file")
    return info


def plain_path(path):
    # Check before resolve() so a junction in the supplied root cannot disappear.
    absolute = Path(os.path.abspath(path))
    for ancestor in reversed(absolute.parents):
        metadata(ancestor)
    metadata(absolute)
    return absolute


def manifest_data(path, expected_hash):
    require(is_hash(expected_hash), "invalid_manifest_digest")
    path = plain_path(path)
    require(path.is_file(), "manifest_not_file")
    with path.open("rb") as stream:
        data = stream.read(MAX_MANIFEST_BYTES + 1)
    require(len(data) <= MAX_MANIFEST_BYTES, "manifest_too_large")
    require(hashlib.sha256(data).hexdigest() == expected_hash, "manifest_digest_mismatch")
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=no_duplicate_keys)
    except (ValueError, UnicodeError, RecursionError):
        raise Rejected("invalid_manifest_json") from None


def validate_manifest(manifest):
    exact_keys(manifest, ("schema_version", "provider", "platform", "packages", "files"))
    require(type(manifest["schema_version"]) is int and manifest["schema_version"] == 1, "unsupported_schema")
    require(manifest["provider"] == "nvidia_maxine_eye_contact", "unsupported_provider")
    require(manifest["platform"] == "windows_x86_64", "unsupported_platform")
    require(isinstance(manifest["packages"], list) and len(manifest["packages"]) == len(PACKAGES), "missing_package")
    packages = {}
    for package in manifest["packages"]:
        exact_keys(package, ("id", "version", "source_url", "archive_sha256", "license_id"))
        name = package["id"]
        require(isinstance(name, str) and name in PACKAGES and name not in packages, "invalid_package")
        require(isinstance(package["version"], str) and VERSION.fullmatch(package["version"]), "unpinned_version")
        require(is_hash(package["archive_sha256"]), "invalid_archive_digest")
        require(isinstance(package["license_id"], str) and 0 < len(package["license_id"].strip()) <= 256, "missing_license_id")
        source = package["source_url"]
        require(isinstance(source, str) and len(source) <= 2048 and not any(c.isspace() or ord(c) < 32 for c in source), "invalid_source_url")
        try:
            url = urlsplit(source)
            require(url.scheme == "https" and url.hostname in {"catalog.ngc.nvidia.com", "api.ngc.nvidia.com"}
                    and not url.username and not url.password and not url.query and not url.fragment
                    and url.port in {None, 443} and url.path.startswith("/"), "invalid_source_url")
        except ValueError:
            raise Rejected("invalid_source_url") from None
        packages[name] = package

    files = manifest["files"]
    require(isinstance(files, list) and 0 < len(files) <= 10000, "invalid_files")
    paths = set()
    roles = {name: set() for name in PACKAGES}
    for entry in files:
        exact_keys(entry, ("path", "package", "kind", "bytes", "sha256"))
        safe_relative(entry["path"])
        key = entry["path"].casefold()
        require(key not in paths, "duplicate_path")
        paths.add(key)
        require(isinstance(entry["package"], str) and entry["package"] in PACKAGES, "invalid_file_package")
        require(isinstance(entry["kind"], str) and entry["kind"] in {"pe", "model", "license", "data"}, "invalid_file_kind")
        require(type(entry["bytes"]) is int and entry["bytes"] > 0, "invalid_file_size")
        require(is_hash(entry["sha256"]), "invalid_file_digest")
        binary = key.endswith((".dll", ".exe"))
        require(binary == (entry["kind"] == "pe"), "invalid_pe_role")
        roles[entry["package"]].add(entry["kind"])
    for name, kinds in roles.items():
        expected = {"pe", "license"} if name == "ar_sdk_core" else {"pe", "model", "license"}
        require(expected <= kinds, "incomplete_package", name)
    return files


def inventory(root):
    found = set()
    pending = [root]
    while pending:
        directory = pending.pop()
        for path in directory.iterdir():
            info = metadata(path)
            if stat.S_ISDIR(info.st_mode):
                pending.append(path)
            else:
                relative = path.relative_to(root).as_posix()
                safe_relative(relative)
                key = relative.casefold()
                require(key not in found, "duplicate_path")
                found.add(key)
    return found


def verify_pe(stream, size, name):
    # Inspect the PE signature/machine/optional-header format; never load it.
    header = stream.read(64)
    require(len(header) == 64 and header[:2] == b"MZ", "invalid_pe", name)
    offset = struct.unpack_from("<I", header, 0x3C)[0]
    require(64 <= offset <= size - 26, "invalid_pe", name)
    stream.seek(offset)
    pe = stream.read(26)
    require(len(pe) == 26 and pe[:4] == b"PE\0\0", "invalid_pe", name)
    require(struct.unpack_from("<H", pe, 4)[0] == 0x8664, "wrong_pe_architecture", name)
    optional_size = struct.unpack_from("<H", pe, 20)[0]
    require(optional_size >= 2 and offset + 24 + optional_size <= size
            and struct.unpack_from("<H", pe, 24)[0] == 0x20B, "invalid_pe", name)


def verify_file(root, entry):
    name = entry["path"]
    path = root.joinpath(*safe_relative(name))
    # Repeat the parent checks immediately before reading each component.
    plain_path(path)
    before = metadata(path)
    require(stat.S_ISREG(before.st_mode) and before.st_size == entry["bytes"], "size_mismatch", name)
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        require((before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino), "file_changed", name)
        if entry["kind"] == "pe":
            verify_pe(stream, entry["bytes"], name)
        stream.seek(0)
        digest = hashlib.sha256()
        remaining = entry["bytes"]
        while remaining:
            data = stream.read(min(1024 * 1024, remaining))
            require(bool(data), "size_mismatch", name)
            digest.update(data)
            remaining -= len(data)
        require(not stream.read(1), "size_mismatch", name)
        after = os.fstat(stream.fileno())
        require((opened.st_size, opened.st_mtime_ns, opened.st_ctime_ns) ==
                (after.st_size, after.st_mtime_ns, after.st_ctime_ns), "file_changed", name)
    require(digest.hexdigest() == entry["sha256"], "digest_mismatch", name)


def verify(root, manifest, expected_hash):
    data = manifest_data(manifest, expected_hash)
    files = validate_manifest(data)
    root = plain_path(root)
    require(root.is_dir(), "runtime_not_directory")
    expected = {entry["path"].casefold() for entry in files}
    actual = inventory(root)
    require(not (expected - actual), "missing_file")
    require(not (actual - expected), "unexpected_file")
    for entry in files:
        verify_file(root, entry)
    require(inventory(root) == expected, "inventory_changed")
    return {
        "status": "integrity_verified", "manifest_sha256": expected_hash,
        "platform": data["platform"], "files_verified": len(files),
        "bytes_verified": sum(entry["bytes"] for entry in files),
        "versions": {entry["id"]: entry["version"] for entry in data["packages"]},
        "license_authorization": "not_assessed", "archive_hashes": "recorded_not_verified",
        "gpu_compatibility": "not_tested", "runtime_load": "not_tested",
        "camera_capture": "not_started", "virtual_camera": "not_tested",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--manifest-sha256", required=True)
    args = parser.parse_args()
    try:
        result = verify(args.root, args.manifest, args.manifest_sha256)
    except Rejected as error:
        result = {"status": "rejected", "code": error.code}
        if error.file:
            result["file"] = error.file
    except OSError as error:
        result = {"status": "rejected", "code": "io_error", "errno": error.errno}
    print(json.dumps(result, ensure_ascii=True, sort_keys=True))
    return 0 if result["status"] == "integrity_verified" else 2


if __name__ == "__main__":
    sys.exit(main())
