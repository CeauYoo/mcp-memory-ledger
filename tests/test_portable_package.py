"""Offline packaging regression tests; synthetic fixtures are never release evidence."""
import importlib.util
import json
import os
from pathlib import Path
import stat
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import io
import gzip
from concurrent.futures import ThreadPoolExecutor

SPEC = importlib.util.spec_from_file_location("portable_package", Path(__file__).parents[1] / "scripts/portable-package.py")
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)
COMMIT, TREE = "1" * 40, "2" * 40


def fake_binary(target="linux-x86_64"):
    # Header-only inert fixture. Tests never execute this or call the runtime smoke.
    data = bytearray(256)
    if target == "linux-x86_64":
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<HH", data, 16, 3, 62)
    elif target.startswith("macos"):
        data[:4] = b"\xcf\xfa\xed\xfe"
        struct.pack_into("<I", data, 4, 0x0100000C if target.endswith("aarch64") else 0x01000007)
        struct.pack_into("<I", data, 12, 2)
    else:
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 60, 128)
        data[128:132] = b"PE\x00\x00"
        struct.pack_into("<H", data, 132, 0x8664)
    return bytes(data)


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.target = "linux-x86_64"
        self.source = {"commit": COMMIT, "tree": TREE, "commit_epoch": 1700000000,
                       "snapshot_kind": "git_archive_committed_tree", "dirty_worktree_used": False}
        self.payload = {name: (b"synthetic test fixture\n", 0o644) for name in package.ASSETS}
        self.payload["bin/agent_llm_mm"] = (fake_binary(), 0o755)
        self.path = self.root / "fixture.tar.gz"

    def tearDown(self):
        self.temp.cleanup()

    def archive(self, payload=None, target=None, suffix=None):
        payload = dict(self.payload if payload is None else payload)
        target = target or self.target
        if suffix:
            self.path = self.root / ("fixture" + suffix)
        manifest = {"kind": "source_built_portable_package", "schema_version": 1,
                    "candidate": "test-only", "target": target, "source": self.source,
                    "local_only": True, "files": package.payload_entries(payload)}
        manifest_bytes = package.canonical(manifest)
        payload[package.MANIFEST] = (manifest_bytes, 0o644)
        if self.path.exists():
            self.path.unlink()
        package.write_archive(self.path, "fixture", payload, 1700000000)
        return {"kind": "portable_package_build_receipt", "schema_version": 1,
                "candidate": "test-only", "target": target, "source": self.source,
                "local_only": True, "manifest_sha256": package.sha(manifest_bytes),
                "archive": {"name": self.path.name, "size_bytes": self.path.stat().st_size,
                            "sha256": package.sha(self.path.read_bytes()), "root": "fixture"}}

    def validate(self, receipt):
        return package.validate_package(self.path, receipt, COMMIT, TREE)

    def test_native_header_fixtures_validate_without_execution(self):
        for target in ("linux-x86_64", "macos-aarch64", "macos-x86_64", "windows-x86_64"):
            with self.subTest(target=target):
                package.validate_binary(fake_binary(target), target)

    def test_plain_text_and_wrong_architecture_are_rejected(self):
        for data in (b"#!/bin/sh\necho stub\n", b"not a binary" * 20, fake_binary("macos-x86_64")):
            with self.subTest(data=data[:10]), self.assertRaises(package.PackageError):
                package.validate_binary(data, "linux-x86_64")

    def test_tar_package_integrity(self):
        receipt = self.archive()
        payload, binary = self.validate(receipt)
        self.assertEqual(binary, "bin/agent_llm_mm")
        self.assertEqual(payload[binary][0], fake_binary())

    def test_windows_zip_package_integrity(self):
        payload = dict(self.payload)
        payload.pop("bin/agent_llm_mm")
        payload["bin/agent_llm_mm.exe"] = (fake_binary("windows-x86_64"), 0o755)
        receipt = self.archive(payload, "windows-x86_64", ".zip")
        self.assertEqual(self.validate(receipt)[1], "bin/agent_llm_mm.exe")

    def test_archive_is_deterministic_for_same_payload_and_source_epoch(self):
        for suffix in (".tar.gz", ".zip"):
            with self.subTest(suffix=suffix):
                self.archive(suffix=suffix)
                first = self.path.read_bytes()
                self.archive(suffix=suffix)
                self.assertEqual(first, self.path.read_bytes())

    def test_source_commit_and_tree_are_both_required(self):
        receipt = self.archive()
        for commit, tree in (("3" * 40, TREE), (COMMIT, "3" * 40)):
            with self.subTest(commit=commit, tree=tree), self.assertRaisesRegex(package.PackageError, "identity"):
                package.validate_package(self.path, receipt, commit, tree)

    def test_dirty_snapshot_claim_rejected(self):
        self.source["dirty_worktree_used"] = True
        with self.assertRaisesRegex(package.PackageError, "committed snapshot"):
            self.validate(self.archive())

    def test_changed_archive_hash_rejected_before_extraction(self):
        receipt = self.archive()
        data = bytearray(self.path.read_bytes())
        data[-5] ^= 1
        self.path.write_bytes(data)
        with self.assertRaisesRegex(package.PackageError, "checksum"):
            self.validate(receipt)

    def test_receipt_size_and_manifest_hash_mismatch_rejected(self):
        for field in ("size", "manifest"):
            with self.subTest(field=field):
                receipt = self.archive()
                if field == "size":
                    receipt["archive"]["size_bytes"] += 1
                else:
                    receipt["manifest_sha256"] = "0" * 64
                with self.assertRaises(package.PackageError):
                    self.validate(receipt)

    def test_missing_required_assets_and_extra_payload_rejected(self):
        for missing in package.ASSETS + ("bin/agent_llm_mm", "extra-file"):
            with self.subTest(missing=missing):
                payload = dict(self.payload)
                if missing == "extra-file":
                    payload[missing] = (b"unexpected\n", 0o644)
                else:
                    payload.pop(missing)
                with self.assertRaisesRegex(package.PackageError, "required files|unexpected payload"):
                    self.validate(self.archive(payload))

    def test_nonexecutable_binary_rejected(self):
        self.payload["bin/agent_llm_mm"] = (fake_binary(), 0o644)
        with self.assertRaisesRegex(package.PackageError, "executable"):
            self.validate(self.archive())

    def test_payload_manifest_tampering_rejected(self):
        receipt = self.archive()
        payload = package.read_archive(self.path, "fixture")
        payload["LICENSE"] = (b"tampered\n", 0o644)
        if self.path.exists():
            self.path.unlink()
        package.write_archive(self.path, "fixture", payload, 1700000000)
        receipt["archive"]["sha256"] = package.sha(self.path.read_bytes())
        receipt["archive"]["size_bytes"] = self.path.stat().st_size
        with self.assertRaisesRegex(package.PackageError, "payload manifest"):
            self.validate(receipt)

    def raw_tar(self, entries):
        with tarfile.open(self.path, "w:gz") as archive:
            for name, kind, mode in entries:
                entry = tarfile.TarInfo(name)
                entry.type, entry.mode = kind, mode
                entry.size = 1 if kind == tarfile.REGTYPE else 0
                entry.linkname = "outside" if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE) else ""
                archive.addfile(entry, io.BytesIO(b"x") if entry.size else None)

    def test_tar_traversal_absolute_and_windows_paths_rejected(self):
        for name in ("fixture/../escape", "/tmp/escape", "fixture/C:/escape", "fixture/a\\b", "fixture//a"):
            with self.subTest(name=name):
                self.raw_tar([(name, tarfile.REGTYPE, 0o644)])
                with self.assertRaises(package.PackageError):
                    package.read_archive(self.path, "fixture")
        self.assertFalse((self.root / "escape").exists())

    def test_tar_links_special_files_duplicates_and_unsafe_modes_rejected(self):
        fixtures = [[("fixture/a", kind, 0o644)] for kind in
                    (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.CHRTYPE, tarfile.DIRTYPE)]
        fixtures += [[("fixture/a", tarfile.REGTYPE, 0o644)] * 2,
                     [("fixture/a", tarfile.REGTYPE, 0o4755)]]
        for entries in fixtures:
            with self.subTest(entries=entries):
                self.raw_tar(entries)
                with self.assertRaises(package.PackageError):
                    package.read_archive(self.path, "fixture")

    def test_zip_symlink_and_traversal_rejected(self):
        self.path = self.root / "bad.zip"
        for name, mode in (("fixture/link", stat.S_IFLNK | 0o777), ("fixture/../escape", stat.S_IFREG | 0o644)):
            with self.subTest(name=name):
                with zipfile.ZipFile(self.path, "w") as archive:
                    entry = zipfile.ZipInfo(name)
                    entry.create_system = 3
                    entry.external_attr = mode << 16
                    archive.writestr(entry, b"outside")
                with self.assertRaises(package.PackageError):
                    package.read_archive(self.path, "fixture")

    def test_empty_truncated_and_oversized_archives_rejected(self):
        for value in (b"", b"not an archive", b"\x1f\x8b\x08"):
            with self.subTest(value=value):
                self.path.write_bytes(value)
                with self.assertRaises((package.PackageError, tarfile.TarError, OSError, EOFError)):
                    package.read_archive(self.path, "fixture")
        self.archive()
        with patch.object(package, "MAX_MEMBER_BYTES", 3), self.assertRaisesRegex(package.PackageError, "limit"):
            package.read_archive(self.path, "fixture")

    def test_archive_member_count_bound(self):
        self.raw_tar([("fixture/" + str(i), tarfile.REGTYPE, 0o644) for i in range(package.MAX_FILES + 1)])
        with self.assertRaisesRegex(package.PackageError, "limit"):
            package.read_archive(self.path, "fixture")

    def test_existing_evidence_never_overwritten(self):
        path = self.root / "evidence"
        package.empty_output(path)
        (path / "keep").write_text("valuable", encoding="utf-8")
        with self.assertRaisesRegex(package.PackageError, "never overwritten"):
            package.empty_output(path)
        self.assertEqual((path / "keep").read_text(encoding="utf-8"), "valuable")

    @unittest.skipIf(os.name == "nt", "symlink creation requires separate Windows privilege")
    def test_symlink_output_rejected(self):
        path = self.root / "linked"
        path.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(package.PackageError, "symlink"):
            package.empty_output(path)

    def test_runtime_environment_removes_toolchain_config_credentials_and_preloads(self):
        dangerous = {"PATH": "/toolchain", "AGENT_LLM_MM_DATABASE_URL": "sqlite:///real.db",
                     "AGENT_LLM_MM_CONFIG": "/real.toml", "OPENAI_API_KEY": "synthetic-only",
                     "LD_PRELOAD": "bad.so", "DYLD_INSERT_LIBRARIES": "bad.dylib"}
        with patch.dict(os.environ, dangerous):
            env = package.runtime_environment(self.root, self.root / "fresh.toml")
        self.assertEqual(env["AGENT_LLM_MM_CONFIG"], str(self.root / "fresh.toml"))
        self.assertEqual(list(Path(env["PATH"]).iterdir()), [])
        for key in dangerous:
            if key not in ("PATH", "AGENT_LLM_MM_CONFIG"):
                self.assertNotIn(key, env)

    def test_native_host_mismatch_rejected_before_smoke(self):
        receipt = self.archive()
        receipt_path = self.root / "receipt.json"
        receipt_path.write_bytes(package.canonical(receipt))
        args = type("Args", (), dict(archive=self.path, receipt=receipt_path, expected_commit=COMMIT,
                                    expected_tree=TREE, output=self.root / "evidence"))()
        with patch.object(package, "native_target", return_value=("macos-aarch64", "aarch64-apple-darwin")), \
             patch.object(package, "smoke") as smoke, self.assertRaisesRegex(package.PackageError, "matching native"):
            package.verify(args)
        smoke.assert_not_called()
        self.assertFalse(args.output.exists())

    def test_failed_runtime_never_writes_passed_summary(self):
        receipt = self.archive()
        receipt_path = self.root / "receipt.json"
        receipt_path.write_bytes(package.canonical(receipt))
        args = type("Args", (), dict(archive=self.path, receipt=receipt_path, expected_commit=COMMIT,
                                    expected_tree=TREE, output=self.root / "evidence"))()
        with patch.object(package, "native_target", return_value=("linux-x86_64", "x86_64-unknown-linux-gnu")), \
             patch.object(package, "smoke", side_effect=package.PackageError("synthetic injected failure")), \
             self.assertRaisesRegex(package.PackageError, "injected failure"):
            package.verify(args)
        self.assertFalse((args.output / "summary.json").exists())

    def test_zip_directory_count_bounded_before_zipfile_allocates_metadata(self):
        path = self.root / "many-entries.zip"
        with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for index in range(20000):
                archive.writestr(f"fixture/{index}", b"x")
        self.assertLess(path.stat().st_size, package.MAX_ARCHIVE_BYTES)
        with patch.object(package.zipfile, "ZipFile") as constructor, \
             self.assertRaisesRegex(package.PackageError, "directory member count limit"):
            package.read_archive(path, "fixture")
        constructor.assert_not_called()

    def test_zip_directory_size_and_count_mismatch_rejected_before_zipfile(self):
        self.archive(suffix=".zip")
        original = self.path.read_bytes()
        for offset, value in ((-12, 50000), (-6, 1)):
            with self.subTest(offset=offset):
                data = bytearray(original)
                struct.pack_into("<I", data, len(data) + offset, value)
                self.path.write_bytes(data)
                with patch.object(package.zipfile, "ZipFile") as constructor, self.assertRaises(package.PackageError):
                    package.read_archive(self.path, "fixture")
                constructor.assert_not_called()

    def test_hidden_zip64_locator_cannot_override_bounded_directory(self):
        def central(name):
            return struct.pack("<4s6H3I5H2I", b"PK\x01\x02", 20, 20, 0, 8, 0, 0,
                               0, 1, 1, len(name), 0, 0, 0, 0, 0, 0) + name
        prefix = b"".join(central(f"root/{index}".encode()) for index in range(20000))
        zip64 = struct.pack("<4sQ2H2I4Q", b"PK\x06\x06", 44, 20, 20, 0, 0,
                            20000, 20000, len(prefix) + 46, 0)
        locator = struct.pack("<4sIQI", b"PK\x06\x07", 0, len(prefix) + 46, 1)
        last = central(zip64 + locator)
        end = struct.pack("<4s4H2IH", b"PK\x05\x06", 0, 0, 1, 1, len(last), len(prefix), 0)
        path = self.root / "hidden-zip64.zip"
        path.write_bytes(prefix + last + end)
        with patch.object(package.zipfile, "ZipFile") as constructor, \
             self.assertRaisesRegex(package.PackageError, "ZIP64 locator"):
            package.read_archive(path, "root")
        constructor.assert_not_called()

    def test_checksum_write_failure_does_not_leave_passed_summary(self):
        receipt = self.archive()
        receipt_path = self.root / "receipt.json"
        receipt_path.write_bytes(package.canonical(receipt))
        args = type("Args", (), dict(archive=self.path, receipt=receipt_path, expected_commit=COMMIT,
                                    expected_tree=TREE, output=self.root / "evidence"))()
        original_write = package.write_new
        def injected_failure(path, data):
            if path.name == "SHA256SUMS":
                raise OSError("synthetic checksum write failure")
            original_write(path, data)
        with patch.object(package, "native_target", return_value=("linux-x86_64", "x86_64-unknown-linux-gnu")), \
             patch.object(package, "smoke", return_value={}), patch.object(package, "write_new", injected_failure), \
             self.assertRaisesRegex(OSError, "checksum write failure"):
            package.verify(args)
        self.assertFalse((args.output / "summary.json").exists())

    def test_gnu_metadata_bomb_is_bounded_before_tarfile_parses_it(self):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode="w", format=tarfile.GNU_FORMAT) as archive:
            entry = tarfile.TarInfo("././@LongLink")
            entry.type = tarfile.GNUTYPE_LONGNAME
            metadata = b"fixture/LICENSE\0" + bytes(4 * 1024 * 1024)
            entry.size = len(metadata)
            archive.addfile(entry, io.BytesIO(metadata))
            entry = tarfile.TarInfo("fixture/LICENSE")
            entry.size = 1
            archive.addfile(entry, io.BytesIO(b"x"))
        self.path.write_bytes(gzip.compress(data.getvalue()))
        self.assertLess(self.path.stat().st_size, 8192)
        with patch.object(package, "MAX_MEMBER_BYTES", 1024), patch.object(package, "MAX_ARCHIVE_BYTES", 8192), \
             self.assertRaisesRegex(package.PackageError, "full decompressed tar"):
            package.read_archive(self.path, "fixture")

    def test_small_gnu_extension_header_rejected(self):
        self.raw_tar([("fixture/name", tarfile.GNUTYPE_LONGNAME, 0o644)])
        with self.assertRaisesRegex(package.PackageError, "extended header"):
            package.read_archive(self.path, "fixture")

    def test_full_gzip_footer_validated(self):
        self.archive()
        valid = self.path.read_bytes()
        for invalid in (valid[:-8], valid[:-8] + bytes(8)):
            with self.subTest(invalid=invalid[-8:]):
                self.path.write_bytes(invalid)
                with self.assertRaises((EOFError, OSError)):
                    package.read_archive(self.path, "fixture")

    def test_concurrent_output_reservations_have_only_one_winner(self):
        path = self.root / "concurrent"
        def reserve():
            try:
                package.empty_output(path)
                return True
            except package.PackageError:
                return False
        with ThreadPoolExecutor(max_workers=8) as executor:
            results = list(executor.map(lambda _: reserve(), range(8)))
        self.assertEqual(sum(results), 1)

    def test_final_artifact_creation_is_exclusive(self):
        self.archive()
        before = self.path.read_bytes()
        with self.assertRaises(FileExistsError):
            package.write_archive(self.path, "fixture", self.payload, 1700000000)
        self.assertEqual(before, self.path.read_bytes())
        with self.assertRaises(FileExistsError):
            package.write_new(self.path, b"replacement")
        self.assertEqual(before, self.path.read_bytes())

    def test_build_environment_ignores_compiler_wrappers_and_toolchain_overrides(self):
        original = {"RUSTUP_TOOLCHAIN": "nightly", "RUSTC": "other-rustc", "RUSTC_WRAPPER": "wrapper",
                    "CARGO_BUILD_RUSTC_WRAPPER": "wrapper", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER": "wrapper",
                    "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS": "bad", "CARGO_HOME": "cache"}
        env = package.build_environment(original, self.root, self.root / "target", 1)
        self.assertNotIn("RUSTUP_TOOLCHAIN", env)
        self.assertNotIn("RUSTC", env)
        self.assertNotIn("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS", env)
        self.assertEqual(env["CARGO_BUILD_RUSTC_WRAPPER"], "")
        self.assertEqual(env["CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"], "")
        self.assertEqual(env["RUSTC_WRAPPER"], "")
        self.assertEqual(env["CARGO_HOME"], "cache")

    def test_candidate_path_injection_rejected(self):
        for candidate in ("../bad", ".hidden", "one/two", "one\\two", "", "a..b", "x" * 101):
            with self.subTest(candidate=candidate), self.assertRaises(package.PackageError):
                package.candidate_name(candidate)
        self.assertEqual(package.candidate_name("local-mvp-ci-rc.1"), "local-mvp-ci-rc.1")


if __name__ == "__main__":
    unittest.main()
