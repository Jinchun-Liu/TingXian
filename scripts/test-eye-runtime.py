"""Synthetic preflight rejection tests. Retain every fixture in a new evidence dir.

Hypothesis: a pinned, complete x64 tree passes; incomplete, altered, redirected or
wrong-architecture trees fail before execution. This decides whether the verifier
can be used for component acquisition. Any unexpected acceptance stops that claim.
It supplies no evidence for NVIDIA loading, licensing, performance or real media.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify-eye-runtime.py")
spec = importlib.util.spec_from_file_location("verify_eye_runtime", SCRIPT)
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)
EVIDENCE = None


def sha(data):
    return hashlib.sha256(data).hexdigest()


def synthetic_pe(machine=0x8664):
    # Header fixture only. No executable instructions or real NVIDIA binaries.
    data = bytearray(512)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<H", data, 132, machine)
    struct.pack_into("<H", data, 148, 240)
    struct.pack_into("<H", data, 152, 0x20B)
    return bytes(data)


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix=self._testMethodName + "-", dir=EVIDENCE))
        self.root = self.directory / "中文运行时 空格"
        self.root.mkdir()
        self.manifest = self.directory / "manifest.json"
        self.data = {"schema_version": 1, "provider": "nvidia_maxine_eye_contact",
                     "platform": "windows_x86_64", "packages": [], "files": []}
        for name in sorted(verifier.PACKAGES):
            self.data["packages"].append({"id": name, "version": "0.0.0.0",
                "source_url": "https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/resources/ar_sdk_core/-",
                "archive_sha256": sha(b"synthetic archive metadata only"), "license_id": "SYNTHETIC-NOT-A-LICENSE"})
            contents = {"pe": ("runtime.dll", synthetic_pe()),
                        "license": ("LICENSE.txt", b"synthetic license fixture; grants nothing")}
            if name != "ar_sdk_core":
                contents["model"] = ("model.bin", b"synthetic weights; not a model")
            for kind, (filename, value) in contents.items():
                path = name + "/" + filename
                destination = self.root / path
                destination.parent.mkdir(exist_ok=True)
                destination.write_bytes(value)
                self.data["files"].append({"path": path, "package": name, "kind": kind,
                                           "bytes": len(value), "sha256": sha(value)})

    def pin(self):
        raw = json.dumps(self.data, ensure_ascii=True).encode("utf-8")
        self.manifest.write_bytes(raw)
        return sha(raw)

    def check(self, expected=None):
        digest = self.pin()
        if expected is None:
            return verifier.verify(self.root, self.manifest, digest)
        with self.assertRaises(verifier.Rejected) as context:
            verifier.verify(self.root, self.manifest, digest)
        self.assertEqual(expected, context.exception.code)

    def cli(self, digest):
        outcome = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--root", str(self.root),
            "--manifest", str(self.manifest), "--manifest-sha256", digest], capture_output=True, text=True, check=False)
        self.assertEqual("", outcome.stderr)
        return outcome.returncode, json.loads(outcome.stdout)

    def test_complete_unicode_tree_is_integrity_only(self):
        code, result = self.cli(self.pin())
        self.assertEqual(0, code)
        self.assertEqual("integrity_verified", result["status"])
        self.assertEqual(11, result["files_verified"])
        self.assertEqual(sum(e["bytes"] for e in self.data["files"]), result["bytes_verified"])
        self.assertEqual("not_assessed", result["license_authorization"])
        self.assertEqual("not_tested", result["runtime_load"])
        self.assertEqual("not_started", result["camera_capture"])
        self.assertNotIn("supported", result)

    def test_manifest_must_match_external_pin(self):
        digest = self.pin()
        self.manifest.write_bytes(self.manifest.read_bytes() + b" ")
        code, result = self.cli(digest)
        self.assertEqual(2, code)
        self.assertEqual("manifest_digest_mismatch", result["code"])

    def test_duplicate_json_key_is_rejected(self):
        self.pin()
        raw = self.manifest.read_bytes().replace(b'"schema_version": 1', b'"schema_version": 9, "schema_version": 1')
        self.manifest.write_bytes(raw)
        code, result = self.cli(sha(raw))
        self.assertEqual(2, code)
        self.assertEqual("duplicate_json_key", result["code"])

    def test_missing_component_dependency(self):
        self.data["packages"] = [p for p in self.data["packages"] if p["id"] != "nvARFaceBoxDetection"]
        self.check("missing_package")

    def test_missing_model_and_license(self):
        original = copy.deepcopy(self.data)
        for kind in ("model", "license"):
            with self.subTest(kind=kind):
                self.data = copy.deepcopy(original)
                self.data["files"] = [f for f in self.data["files"]
                    if not (f["package"] == "nvARGazeRedirection" and f["kind"] == kind)]
                self.check("incomplete_package")

    def test_missing_physical_file(self):
        source = self.root / self.data["files"][0]["path"]
        source.rename(self.directory / "preserved-missing-file.dll")
        self.check("missing_file")

    def test_unlisted_dll(self):
        (self.root / "unexpected.dll").write_bytes(synthetic_pe())
        self.check("unexpected_file")

    def test_same_size_tampering(self):
        entry = next(f for f in self.data["files"] if f["kind"] == "model")
        (self.root / entry["path"]).write_bytes(b"x" * entry["bytes"])
        self.check("digest_mismatch")

    def test_truncated_model(self):
        entry = next(f for f in self.data["files"] if f["kind"] == "model")
        (self.root / entry["path"]).write_bytes(b"x")
        self.check("size_mismatch")

    def test_arm64_binary_with_matching_digest(self):
        entry = self.data["files"][0]
        value = synthetic_pe(0xAA64)
        (self.root / entry["path"]).write_bytes(value)
        entry["sha256"] = sha(value)
        self.check("wrong_pe_architecture")

    def test_malformed_pe_with_matching_digest(self):
        entry = self.data["files"][0]
        value = bytearray(synthetic_pe())
        struct.pack_into("<I", value, 0x3C, 0xFFFFFFFF)
        (self.root / entry["path"]).write_bytes(value)
        entry["sha256"] = sha(value)
        self.check("invalid_pe")

    def test_pe_cannot_be_disguised_as_data(self):
        self.data["files"][0]["kind"] = "data"
        self.check("invalid_pe_role")

    def test_windows_aliases_and_traversal(self):
        for path in ("../outside", "C:/outside", "//server/share", "a\\b.dll", "file:stream",
                     "CON.txt", "AUX/model.bin", "com1.dll", "dir/../file", "dir./x", "dir /x",
                     "/absolute", "a//b", "name\x00", "name\ud800"):
            with self.subTest(path=ascii(path)):
                self.data["files"][0]["path"] = path
                self.check("invalid_path")

    def test_case_collisions(self):
        duplicate = copy.deepcopy(self.data["files"][0])
        duplicate["path"] = duplicate["path"].upper()
        self.data["files"].append(duplicate)
        self.check("duplicate_path")

    def test_unknown_schema_and_unpinned_version(self):
        self.data["schema_version"] = True
        self.check("unsupported_schema")
        self.data["schema_version"] = 1
        self.data["packages"][0]["version"] = "latest"
        self.check("unpinned_version")

    def test_source_does_not_embed_credentials(self):
        for url in ("https://user:password@catalog.ngc.nvidia.com/file", "https://catalog.ngc.nvidia.com/file?token=synthetic",
                    "https://example.invalid/file", "http://catalog.ngc.nvidia.com/file", "https://catalog.ngc.nvidia.com:bad/file"):
            with self.subTest(url=url):
                self.data["packages"][0]["source_url"] = url
                self.check("invalid_source_url")

    def test_hardlink_rejected(self):
        outside = self.directory / "outside.dll"
        outside.write_bytes(synthetic_pe())
        os.link(outside, self.root / "linked.dll")
        self.check("hardlinked_file")

    def link_directory(self, source, destination):
        if os.name == "nt":
            # Python's Windows junction helper needs no developer-mode privilege.
            import _winapi
            _winapi.CreateJunction(str(source), str(destination))
        else:
            os.symlink(source, destination, target_is_directory=True)

    def test_junction_or_symlink_subdirectory(self):
        outside = self.directory / "outside"
        outside.mkdir()
        (outside / "private.txt").write_bytes(b"synthetic outside sentinel")
        self.link_directory(outside, self.root / "redirect")
        self.check("linked_path")
        self.assertEqual(b"synthetic outside sentinel", (outside / "private.txt").read_bytes())

    def test_runtime_root_cannot_hide_a_junction(self):
        link = self.directory / "linked-root"
        self.link_directory(self.root, link)
        self.root = link
        self.check("linked_path")

    def test_cli_missing_root_has_structured_failure(self):
        self.root = self.directory / "absent"
        code, result = self.cli(self.pin())
        self.assertEqual(2, code)
        self.assertEqual("io_error", result["code"])
        self.assertNotIn(str(self.directory), json.dumps(result))


class LegacyRuntimeTests(unittest.TestCase):
    # Synthetic files exercise only our manifest contract, never NVIDIA code.
    pin = RuntimeTests.pin
    check = RuntimeTests.check
    cli = RuntimeTests.cli

    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix=self._testMethodName + "-", dir=EVIDENCE))
        self.root = self.directory / "旧版运行时"
        self.root.mkdir()
        self.manifest = self.directory / "manifest.json"
        self.data = {"schema_version": 2, "distribution": verifier.LEGACY_PACKAGE,
            "provider": "nvidia_maxine_eye_contact", "platform": "windows_x86_64",
            "packages": [{"id": verifier.LEGACY_PACKAGE, "version": "0.8.7.0",
                "source_url": verifier.LEGACY_SOURCE, "archive_sha256": sha(b"synthetic metadata"),
                "license_id": "SYNTHETIC-NOT-A-LICENSE"}], "files": []}
        for path, kind in verifier.LEGACY_REQUIRED.items():
            value = synthetic_pe() if kind == "pe" else b"synthetic content; no model or license"
            destination = self.root / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(value)
            self.data["files"].append({"path": path, "package": verifier.LEGACY_PACKAGE,
                "kind": kind, "bytes": len(value), "sha256": sha(value)})

    def test_legacy_complete_tree_is_integrity_only(self):
        code, result = self.cli(self.pin())
        self.assertEqual(0, code)
        self.assertEqual({verifier.LEGACY_PACKAGE: "0.8.7.0"}, result["versions"])
        self.assertEqual("not_assessed", result["license_authorization"])
        self.assertEqual("not_tested", result["runtime_load"])

    def test_legacy_decoder_cannot_be_omitted_or_relabelled(self):
        path = "models/gazeredir_decoder_fp16_89.engine.trtpkg"
        entry = next(f for f in self.data["files"] if f["path"] == path)
        entry["kind"] = "data"
        self.check("incomplete_legacy_runtime")
        self.data["files"].remove(entry)
        self.check("incomplete_legacy_runtime")

    def test_legacy_required_cuda_dependency(self):
        self.data["files"] = [f for f in self.data["files"] if f["path"] != "cudart64_12.dll"]
        self.check("incomplete_legacy_runtime")

    def test_legacy_version_and_architecture_source_are_pinned(self):
        package = self.data["packages"][0]
        package["version"] = "0.8.2.0"
        self.check("unsupported_legacy_version")
        package["version"] = "0.8.7.0"
        package["source_url"] = verifier.LEGACY_SOURCE.replace("_ada.exe", "_ampere.exe")
        self.check("invalid_source_url")
        package["source_url"] = verifier.LEGACY_SOURCE + "?key=synthetic"
        self.check("invalid_source_url")

    def test_legacy_cannot_mix_ngc_packages(self):
        self.data["packages"][0]["id"] = "ar_sdk_core"
        self.check("invalid_package")

    def test_legacy_unknown_distribution_does_not_downgrade(self):
        self.data["distribution"] = "latest"
        self.check("unsupported_distribution")

    def test_legacy_x86_helper_is_not_an_x64_runtime(self):
        entry = self.data["files"][0]
        value = synthetic_pe(machine=0x14c)
        (self.root / entry["path"]).write_bytes(value)
        entry["sha256"] = sha(value)
        self.check("wrong_pe_architecture")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", type=Path, required=True, help="New directory; never overwrite or remove fixtures")
    args = parser.parse_args()
    EVIDENCE = args.evidence_dir.absolute()
    EVIDENCE.mkdir(parents=True, exist_ok=False)
    suite = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromTestCase(case)
                               for case in (RuntimeTests, LegacyRuntimeTests))
    outcome = unittest.TextTestRunner(verbosity=2).run(suite)
    (EVIDENCE / "result.json").write_text(json.dumps({"tests": outcome.testsRun, "failures": len(outcome.failures),
        "errors": len(outcome.errors), "skipped": len(outcome.skipped), "synthetic_only": True,
        "fixture_cleanup": "retained", "camera_started": False, "sdk_loaded": False}, indent=2), encoding="utf-8")
    sys.exit(0 if outcome.wasSuccessful() else 1)
