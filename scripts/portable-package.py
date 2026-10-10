#!/usr/bin/env python3
"""Build exact-source native portable archives; verify them without Rust on PATH.

Local artifacts and local simulation only. No upload, tag, signing, release
approval, real fresh-user evidence, or cross-platform support inference.
"""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import queue
import re
import shutil
import sqlite3
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import zipfile
import zlib

TARGETS = {
    ("Linux", "x86_64"): ("linux-x86_64", "x86_64-unknown-linux-gnu"),
    ("Darwin", "arm64"): ("macos-aarch64", "aarch64-apple-darwin"),
    ("Darwin", "x86_64"): ("macos-x86_64", "x86_64-apple-darwin"),
    ("Windows", "amd64"): ("windows-x86_64", "x86_64-pc-windows-msvc"),
}
MAX_FILES = 32
MAX_MEMBER_BYTES = 256 * 1024 * 1024
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
MANIFEST = "BUILD-MANIFEST.json"
ASSETS = ("LICENSE", "NOTICE", "docs/portable-packages.md", "docs/database-operations.md",
          "docs/tool-reference.md", "examples/agent-llm-mm.example.toml")
NON_CLAIMS = ["real fresh-machine or user-client evidence", "other platform validation",
              "installer, service manager or auto-updater", "signed or published release",
              "Local Alpha, Beta, GA or production readiness"]


class PackageError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise PackageError(message)


def canonical(value):
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def native_target():
    key = (platform.system(), platform.machine().lower())
    require(key in TARGETS, f"unsupported native build/verify host: {key}")
    return TARGETS[key]


def candidate_name(value):
    require(bool(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}", value)) and ".." not in value,
            "candidate must be 1-100 safe letters/numbers/dot/underscore/dash, without traversal")
    return value


def path_name(value):
    require(isinstance(value, str) and value and "\\" not in value and ":" not in value,
            "unsafe archive path")
    path = PurePosixPath(value)
    require(not path.is_absolute() and all(x not in ("", ".", "..") for x in value.split("/")),
            "unsafe archive path")
    return path


def empty_output(path):
    require(not path.is_symlink(), "output directory must not be a symlink")
    require(not path.exists() or (path.is_dir() and not any(path.iterdir())),
            "output directory must be empty; existing evidence is never overwritten")
    path.mkdir(parents=True, exist_ok=True)
    # Reserve before any artifact write. A failed attempt leaves its reservation
    # as diagnostic evidence; concurrent attempts cannot both own this directory.
    try:
        with (path / ".portable-package-reservation").open("x", encoding="ascii") as marker:
            marker.write("reserved by portable-package; do not reuse this output directory\n")
    except FileExistsError as error:
        raise PackageError("output directory is already reserved; existing evidence is never overwritten") from error
    require(not any(item.name != ".portable-package-reservation" for item in path.iterdir()),
            "output changed during reservation; existing evidence is never overwritten")
    return path.resolve()


def write_new(path, data):
    with path.open("xb") as output:
        output.write(data)


def checked(command, cwd=None, env=None):
    result = subprocess.run(command, cwd=cwd, env=env, capture_output=True,
                            text=True, encoding="utf-8", timeout=60)
    require(result.returncode == 0, f"command failed: {command[0]}: {result.stderr.strip()}")
    return result.stdout.strip()


def validate_binary(data, target):
    require(len(data) >= 64, "executable payload is truncated")
    if target == "linux-x86_64":
        require(data[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", data, 16)[0] in (2, 3)
                and struct.unpack_from("<H", data, 18)[0] == 62,
                "expected a native x86_64 ELF executable")
    elif target in ("macos-aarch64", "macos-x86_64"):
        cpu = 0x0100000C if target == "macos-aarch64" else 0x01000007
        require(data[:4] == b"\xcf\xfa\xed\xfe" and struct.unpack_from("<I", data, 4)[0] == cpu
                and struct.unpack_from("<I", data, 12)[0] == 2,
                "expected a native Mach-O executable for the named architecture")
    elif target == "windows-x86_64":
        offset = struct.unpack_from("<I", data, 60)[0]
        require(data[:2] == b"MZ" and offset + 24 <= len(data)
                and data[offset:offset + 4] == b"PE\x00\x00"
                and struct.unpack_from("<H", data, offset + 4)[0] == 0x8664,
                "expected a native x86_64 PE executable")
    else:
        raise PackageError("unsupported package target")


def payload_entries(payload):
    return [{"path": name, "sha256": sha(data), "size_bytes": len(data), "mode": mode}
            for name, (data, mode) in sorted(payload.items())]


def write_archive(path, root, payload, epoch):
    """Stable metadata/order/compression for identical payloads; not a compiler reproducibility claim."""
    if path.name.endswith(".zip"):
        date = time.gmtime(max(315532800, min(epoch, 4354819199)))[:6]
        with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for name, (data, mode) in sorted(payload.items()):
                entry = zipfile.ZipInfo(root + "/" + name, date)
                entry.create_system = 3
                entry.external_attr = (stat.S_IFREG | mode) << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(entry, data)
    else:
        with path.open("xb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                for name, (data, mode) in sorted(payload.items()):
                    entry = tarfile.TarInfo(root + "/" + name)
                    entry.size, entry.mode, entry.mtime = len(data), mode, epoch
                    entry.uid = entry.gid = 0
                    entry.uname = entry.gname = ""
                    archive.addfile(entry, io.BytesIO(data))


def source_snapshot(repo, ref, destination):
    require(not ref.startswith("-"), "source ref must not be an option")
    commit = checked(["git", "rev-parse", "--verify", ref + "^{commit}"], cwd=repo)
    tree = checked(["git", "rev-parse", commit + "^{tree}"], cwd=repo)
    epoch = int(checked(["git", "show", "-s", "--format=%ct", commit], cwd=repo))
    # git archive reads committed blobs, never the caller's dirty/untracked files.
    archive_path = destination.parent / "source.tar"
    subprocess.run(["git", "archive", "--format=tar", "--output", str(archive_path), commit],
                   cwd=repo, check=True, timeout=60)
    with tarfile.open(archive_path, "r:") as archive:
        for entry in archive:
            path_name(entry.name.rstrip("/"))
            require(entry.isfile() or entry.isdir(), "source snapshot contains links or special files")
            target = destination / entry.name
            if entry.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(entry) as src, target.open("xb") as dst:
                    shutil.copyfileobj(src, dst)
                target.chmod(entry.mode & 0o777)
    require((destination / "Cargo.lock").is_file(), "source snapshot must contain Cargo.lock")
    return {"commit": commit, "tree": tree, "commit_epoch": epoch,
            "snapshot_kind": "git_archive_committed_tree", "dirty_worktree_used": False}


def build_environment(original, source_dir, target_dir, epoch):
    env = original.copy()
    for name in list(env):
        if (name in ("RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                     "RUSTUP_TOOLCHAIN", "CARGO_ENCODED_RUSTFLAGS")
                or name.startswith(("CARGO_BUILD_", "CARGO_TARGET_", "CARGO_PROFILE_"))):
            env.pop(name)
    # Explicit empty env values plus command-line Cargo config overrides defeat
    # inherited wrapper settings from both environment and ambient Cargo config.
    env.update({"CARGO_TARGET_DIR": str(target_dir), "SOURCE_DATE_EPOCH": str(epoch),
                "CARGO_ENCODED_RUSTFLAGS": f"--remap-path-prefix={source_dir}=/ledger-source",
                "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "",
                "CARGO_BUILD_RUSTC_WRAPPER": "", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER": ""})
    return env


def build(args):
    candidate = candidate_name(args.candidate)
    target, triple = native_target()
    repo = args.repo.resolve(strict=True)
    output = empty_output(args.output)
    cargo, rustc = shutil.which("cargo"), shutil.which("rustc")
    require(cargo and rustc, "build requires cargo and rustc; unpacked verification does not")
    with tempfile.TemporaryDirectory(prefix="ledger-source-") as temporary:
        source_dir = Path(temporary) / "source"
        source_dir.mkdir()
        source = source_snapshot(repo, args.source_ref, source_dir)
        payload = {}
        for name in ASSETS:
            source_file = source_dir / name
            require(source_file.is_file(), f"source commit is missing required package asset: {name}")
            payload[name] = (source_file.read_bytes(), 0o644)
        script_in_source = source_dir / "scripts/portable-package.py"
        packager_bytes = Path(__file__).read_bytes()
        require(script_in_source.is_file() and script_in_source.read_bytes() == packager_bytes,
                "packager must match scripts/portable-package.py in the selected source commit")
        target_dir = (args.target_dir or Path(os.environ.get("CARGO_TARGET_DIR", repo / "target"))).resolve()
        env = os.environ.copy()
        env = build_environment(env, source_dir, target_dir, source["commit_epoch"])
        # Query in the committed snapshot, not a potentially dirty caller checkout.
        rust_version = checked([rustc, "-vV"], cwd=source_dir, env=env)
        require(f"host: {triple}" in rust_version.splitlines(), "native Rust host does not match package target")
        pinned = re.search(r'^channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"',
                           (source_dir / "rust-toolchain.toml").read_text(encoding="utf-8"), re.MULTILINE)
        require(pinned is not None and f"release: {pinned[1]}" in rust_version.splitlines(),
                "rustc must match the exact version pinned in the committed source")
        cargo_version = checked([cargo, "-V"], cwd=source_dir, env=env)
        require(cargo_version.startswith("cargo " + pinned[1] + " "), "Cargo version does not match pinned toolchain")
        overrides = ["--config", "build.rustc=" + json.dumps(rustc),
                     "--config", 'build.rustc-wrapper=""',
                     "--config", 'build.rustc-workspace-wrapper=""']
        command = [cargo, "build", "--locked", "--release", "--bin", "agent_llm_mm", "--target", triple] + overrides
        if args.offline:
            command.append("--offline")
        subprocess.run(command, cwd=source_dir, env=env, check=True)
        binary_name = "agent_llm_mm.exe" if target.startswith("windows") else "agent_llm_mm"
        binary = target_dir / triple / "release" / binary_name
        binary_data = binary.read_bytes()
        validate_binary(binary_data, target)
        payload["bin/" + binary_name] = (binary_data, 0o755)
        manifest = {"schema_version": 1, "kind": "source_built_portable_package", "candidate": candidate,
                    "target": target, "rust_target": triple, "source": source,
                    "toolchain": {"rustc": rust_version, "cargo": cargo_version},
                    "build_command": ["cargo"] + [item.replace(json.dumps(rustc), json.dumps("<verified-rustc>")) for item in command[1:]],
                    "compiler_wrappers_disabled": True, "local_only": True,
                    "packager_sha256": sha(packager_bytes),
                    "non_claims": NON_CLAIMS, "files": payload_entries(payload)}
        payload[MANIFEST] = (canonical(manifest), 0o644)
        name = f"agent-llm-mm-{candidate}-{target}"
        archive = output / (name + (".zip" if target.startswith("windows") else ".tar.gz"))
        write_archive(archive, name, payload, source["commit_epoch"])
        receipt = {"schema_version": 1, "kind": "portable_package_build_receipt", "candidate": candidate,
                   "target": target, "source": source, "local_only": True, "non_claims": NON_CLAIMS,
                   "archive": {"name": archive.name, "size_bytes": archive.stat().st_size,
                               "sha256": sha(archive.read_bytes()), "root": name},
                   "manifest_sha256": sha(payload[MANIFEST][0])}
        write_new(output / (name + ".sha256"),
                  (receipt["archive"]["sha256"] + "  " + archive.name + "\n").encode("ascii"))
        write_new(output / (name + ".build.json"), canonical(receipt))
    print(json.dumps(receipt))


def bounded_zip_directory(path):
    # Our writer uses ordinary, single-disk ZIP with no comments/extra fields.
    # ZipFile materializes the full directory in its constructor, so enforce
    # its budget from the fixed EOF record before giving it untrusted bytes.
    with path.open("rb") as file:
        file.seek(0, 2)
        size = file.tell()
        require(size >= 22, "truncated ZIP end record")
        file.seek(-22, 2)
        fields = struct.unpack("<4s4H2IH", file.read(22))
        if size >= 42:
            file.seek(-42, 2)
            require(file.read(4) != b"PK\x06\x07", "ZIP64 locator is unsupported")
        signature, disk, directory_disk, disk_count, count, directory_size, offset, comment_size = fields
        require(signature == b"PK\x05\x06" and comment_size == 0,
                "ZIP requires an unambiguous comment-free end record")
        require(disk == directory_disk == 0 and disk_count == count,
                "multidisk ZIP is unsupported")
        require(0 < count <= MAX_FILES, "ZIP directory member count limit exceeded")
        require(directory_size <= MAX_FILES * (46 + 512) and offset + directory_size == size - 22,
                "ZIP directory size/offset invalid; ZIP64 and metadata extensions unsupported")
        file.seek(offset)
        directory = file.read(directory_size)
    position = 0
    for _ in range(count):
        require(position + 46 <= len(directory), "truncated ZIP directory entry")
        entry = struct.unpack_from("<4s6H3I5H2I", directory, position)
        name_size, extra_size, entry_comment_size, start_disk = entry[10:14]
        require(entry[0] == b"PK\x01\x02" and entry[2] <= 20 and entry[4] == 8,
                "unsupported ZIP directory entry format")
        require(0 < name_size <= 512 and extra_size == entry_comment_size == start_disk == 0,
                "ZIP directory metadata limit exceeded")
        require(entry[8] != 0xFFFFFFFF and entry[9] != 0xFFFFFFFF and entry[16] != 0xFFFFFFFF,
                "ZIP64 is unsupported")
        filename = directory[position + 46:position + 46 + name_size]
        require(bool(re.fullmatch(rb"[A-Za-z0-9._/-]+", filename)), "unexpected ZIP filename bytes")
        path_name(filename.decode("ascii"))
        position += 46 + name_size
        require(position <= len(directory), "truncated ZIP directory filename")
    require(position == len(directory), "ZIP directory count/size mismatch")


def read_archive(path, root):
    """Validate before extraction. Never call extractall or permit links/devices/duplicate paths."""
    require(path.is_file() and 0 < path.stat().st_size <= MAX_ARCHIVE_BYTES, "archive size is invalid")
    payload = {}
    total = 0

    def add(name, size, mode, reader):
        nonlocal total
        path_name(name)
        require(name.startswith(root + "/"), "archive has an unexpected root")
        relative = name[len(root) + 1:]
        require(relative not in payload, "duplicate archive member")
        require(len(payload) < MAX_FILES and 0 < size <= MAX_MEMBER_BYTES, "archive member limit exceeded")
        total += size
        require(total <= MAX_ARCHIVE_BYTES, "archive expanded size limit exceeded")
        require(mode in (0o644, 0o755), "unsafe or unexpected archive permissions")
        data = reader.read(size + 1)
        require(len(data) == size, "archive member length mismatch")
        payload[relative] = (data, mode)

    if path.name.endswith(".zip"):
        bounded_zip_directory(path)
        with zipfile.ZipFile(path) as archive:
            for entry in archive.infolist():
                mode = entry.external_attr >> 16
                require(stat.S_ISREG(mode) and not entry.flag_bits & 1,
                        "archive contains a link, directory, special or encrypted entry")
                with archive.open(entry) as reader:
                    add(entry.filename, entry.file_size, stat.S_IMODE(mode), reader)
    elif path.name.endswith(".tar.gz"):
        # tarfile consumes GNU/PAX metadata internally, before yielding members.
        # Bound the entire decompressed stream and inspect raw USTAR headers first.
        with tempfile.TemporaryFile() as expanded:
            expanded_size = 0
            with gzip.open(path, "rb") as compressed:
                while True:
                    block = compressed.read(65536)
                    if not block:
                        break
                    expanded_size += len(block)
                    require(expanded_size <= MAX_ARCHIVE_BYTES, "full decompressed tar size limit exceeded")
                    expanded.write(block)
            expanded.seek(0)
            while True:
                header = expanded.read(512)
                require(len(header) == 512, "truncated tar header or missing terminator")
                if header == bytes(512):
                    require(expanded.read(512) == bytes(512), "tar needs two zero terminator blocks")
                    while True:
                        padding = expanded.read(65536)
                        if not padding:
                            break
                        require(not any(padding), "unexpected trailing tar data")
                    break
                require(header[156:157] in (b"0", b"\0") and header[257:265] == b"ustar\x0000",
                        "archive contains a non-USTAR, special or extended header")
                size_text = header[124:136].strip(b"\0 ")
                require(bool(re.fullmatch(b"[0-7]+", size_text)), "invalid USTAR size")
                member_size = int(size_text, 8)
                require(0 < member_size <= MAX_MEMBER_BYTES, "archive member size limit exceeded")
                skip = (member_size + 511) // 512 * 512
                require(expanded.tell() + skip <= expanded_size, "truncated tar payload")
                expanded.seek(skip, 1)
            expanded.seek(0)
            with tarfile.open(fileobj=expanded, mode="r:") as archive:
                for entry in archive:
                    require(entry.isfile() and not entry.pax_headers,
                            "archive contains a link, directory, special or extended entry")
                    with archive.extractfile(entry) as reader:
                        add(entry.name, entry.size, entry.mode, reader)
    else:
        raise PackageError("expected .tar.gz or .zip package")
    return payload


def validate_package(archive, receipt, expected_commit, expected_tree):
    require(receipt.get("schema_version") == 1 and receipt.get("kind") == "portable_package_build_receipt"
            and receipt.get("local_only") is True, "invalid build receipt")
    candidate_name(receipt["candidate"])
    source = receipt["source"]
    require(source.get("commit") == expected_commit and source.get("tree") == expected_tree,
            "source commit/tree does not match expected identity")
    require(bool(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", expected_commit))
            and bool(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", expected_tree)), "invalid source identity")
    require(source.get("snapshot_kind") == "git_archive_committed_tree" and source.get("dirty_worktree_used") is False,
            "source is not an exact committed snapshot")
    info = receipt["archive"]
    require(archive.name == info["name"] and archive.is_file(), "archive filename mismatch or missing file")
    require(0 < archive.stat().st_size <= MAX_ARCHIVE_BYTES and archive.stat().st_size == info["size_bytes"],
            "archive size mismatch")
    require(sha(archive.read_bytes()) == info["sha256"], "archive checksum mismatch")
    path_name(info["root"])
    payload = read_archive(archive, info["root"])
    require(MANIFEST in payload, "missing package manifest")
    manifest_bytes = payload.pop(MANIFEST)[0]
    require(sha(manifest_bytes) == receipt["manifest_sha256"], "package manifest checksum mismatch")
    manifest = json.loads(manifest_bytes)
    require(manifest.get("kind") == "source_built_portable_package" and manifest.get("schema_version") == 1
            and manifest.get("source") == source and manifest.get("target") == receipt["target"]
            and manifest.get("candidate") == receipt["candidate"] and manifest.get("local_only") is True,
            "package manifest identity mismatch")
    require(manifest.get("files") == payload_entries(payload), "payload manifest does not match archive contents")
    binary = "bin/agent_llm_mm.exe" if receipt["target"] == "windows-x86_64" else "bin/agent_llm_mm"
    require(set(payload) == set(ASSETS) | {binary}, "package is missing required files or has unexpected payload")
    require(payload[binary][1] == 0o755, "binary must be executable")
    validate_binary(payload[binary][0], receipt["target"])
    payload[MANIFEST] = (manifest_bytes, 0o644)
    return payload, binary


def runtime_environment(home, config):
    # Keep only Windows loader essentials; no inherited model keys, config, PATH, preload or proxy settings.
    env = {key: value for key, value in os.environ.items() if key.upper() in ("SYSTEMROOT", "WINDIR", "SYSTEMDRIVE")}
    no_tools = home / "empty-path"
    no_tools.mkdir()
    env.update({"PATH": str(no_tools), "HOME": str(home), "USERPROFILE": str(home),
                "TMPDIR": str(home), "TMP": str(home), "TEMP": str(home),
                "AGENT_LLM_MM_CONFIG": str(config), "RUST_LOG": "warn"})
    require(not shutil.which("cargo", path=env["PATH"]) and not shutil.which("rustc", path=env["PATH"]),
            "Rust toolchain unexpectedly available on child PATH")
    return env


class McpClient:
    def __init__(self, binary, cwd, env, transcript):
        self.transcript = transcript
        self.counter = 0
        self.stderr = tempfile.TemporaryFile()
        self.proc = subprocess.Popen([str(binary), "serve"], cwd=cwd, env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=self.stderr, text=True, encoding="utf-8")
        self.lines = queue.Queue()
        def reader():
            try:
                for line in self.proc.stdout:
                    self.lines.put(line)
            finally:
                self.lines.put(None)
        threading.Thread(target=reader, daemon=True).start()

    def __enter__(self):
        return self

    def __exit__(self, *unused):
        self.proc.stdin.close()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=10)
        self.proc.stdout.close()
        self.stderr.close()

    def send(self, value):
        self.proc.stdin.write(json.dumps(value, ensure_ascii=False) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params):
        self.counter += 1
        request = {"jsonrpc": "2.0", "id": self.counter, "method": method, "params": params}
        self.send(request)
        deadline = time.monotonic() + 30
        while True:
            try:
                line = self.lines.get(timeout=max(0.001, deadline - time.monotonic()))
            except queue.Empty as error:
                raise PackageError("MCP request timed out") from error
            require(line is not None, "MCP process exited before reply")
            reply = json.loads(line)
            require(time.monotonic() <= deadline, "MCP request timed out")
            if reply.get("id") == self.counter:
                break
        self.transcript.append({"request": request, "response": reply})
        require("error" not in reply and "result" in reply, "MCP returned an error")
        return reply["result"]

    def initialize(self):
        result = self.request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                              "clientInfo": {"name": "portable-package-local-simulation", "version": "1"}})
        require("serverInfo" in result, "MCP initialize lacks server information")
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def tool(self, name, arguments):
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        require(not result.get("isError") and "structuredContent" in result, "MCP tool failed")
        return result["structuredContent"]


def smoke(binary, root, output):
    state = root / "state"
    state.mkdir()
    database, config = state / "memory.sqlite", state / "memory.toml"
    config.write_text("transport = \"stdio\"\ndatabase_url = " + json.dumps("sqlite://" + database.as_posix())
                      + '\n[model]\nprovider = "mock"\n[dashboard]\nenabled = false\n[daemon]\nenabled = false\n',
                      encoding="utf-8")
    env = runtime_environment(state, config)
    def command(args):
        result = subprocess.run([str(binary)] + args, cwd=state, env=env, capture_output=True,
                                text=True, encoding="utf-8", timeout=60)
        require(result.returncode == 0, f"unpacked {args[0]} failed: {result.stderr}")
        return json.loads(result.stdout)
    init = command(["init"])
    require(database.is_file(), "init did not create isolated SQLite database")
    before = sha(database.read_bytes())
    doctor = command(["doctor", "--read-only"])
    require(doctor.get("status") == "ok" and doctor.get("provider") == "mock", "doctor status/provider mismatch")
    require(doctor.get("daemon_enabled") is False, "daemon must remain disabled")
    require(sha(database.read_bytes()) == before, "read-only doctor changed database bytes")
    transcript = []
    with McpClient(binary, state, env, transcript) as client:
        client.initialize()
        names = {tool["name"] for tool in client.request("tools/list", {})["tools"]}
        require({"ingest_interaction", "recall_memory", "get_memory", "supersede_memory"} <= names,
                "required MCP tools missing")
        for namespace in ("project/package/a", "project/package/b"):
            result = client.tool("ingest_interaction", {"request_id": namespace + "-seed", "event": {
                "owner": "World", "namespace": namespace, "kind": "Observation", "summary": "portable amber memory"},
                "claim_drafts": [{"owner": "World", "namespace": namespace, "subject": "package-test",
                                  "predicate": "color", "object": "amber", "mode": "Observed"}],
                "episode_reference": "episode:" + namespace})
            if namespace == "project/package/a":
                event_id = result["event_id"]
    with McpClient(binary, state, env, transcript) as client:
        client.initialize()
        recalled = client.tool("recall_memory", {"namespace": "project/package/a", "query": "amber"})
        require(bool(recalled["records"]) and all(record["record"]["namespace"] == "project/package/a"
                for record in recalled["records"]), "restart recall missing or scope leakage detected")
        old = event_id + ":claim:0"
        inspected = client.tool("get_memory", {"namespace": "project/package/a", "record_type": "Claim", "id": old})
        require(bool(inspected["record"]["provenance"]["evidence_event_references"]), "missing Claim provenance")
        feedback = client.tool("ingest_interaction", {"request_id": "package-correction", "event": {
            "owner": "World", "namespace": "project/package/a", "kind": "Observation", "summary": "portable violet correction"},
            "claim_drafts": [], "episode_reference": "episode:package/correction"})
        client.tool("supersede_memory", {"request_id": "package-supersede", "namespace": "project/package/a",
            "claim_reference": old, "replacement_claim": {"owner": "World", "namespace": "project/package/a",
            "subject": "package-test", "predicate": "color", "object": "violet", "mode": "Observed"},
            "replacement_evidence_event_ids": [feedback["event_id"]], "summary": "synthetic package smoke correction"})
        current = client.tool("recall_memory", {"namespace": "project/package/a", "query": "violet"})
        require(any(x["record"].get("object") == "violet" and x["record"].get("status") == "Active"
                    for x in current["records"]), "corrected Claim not returned")
        history = client.tool("get_reflection_history", {"namespace": "project/package/a", "claim_reference": old})
        require(bool(history["reflections"]), "correction audit history missing")
    backup, restored = state / "backup.sqlite", state / "restored.sqlite"
    # Reserve brand-new paths, then use SQLite's online backup API. No live-file
    # copy, existing-database overwrite, or automatic production-path switch.
    with backup.open("xb"), restored.open("xb"):
        pass
    with sqlite3.connect(f"file:{database.as_posix()}?mode=ro", uri=True) as db, sqlite3.connect(backup) as dst:
        db.backup(dst)
    with sqlite3.connect(f"file:{backup.as_posix()}?mode=ro", uri=True) as db, sqlite3.connect(restored) as dst:
        db.backup(dst)
    with sqlite3.connect(f"file:{database.as_posix()}?mode=ro", uri=True) as db, \
            sqlite3.connect(f"file:{restored.as_posix()}?mode=ro", uri=True) as restored_db:
        require(list(db.iterdump()) == list(restored_db.iterdump()), "backup/restore content mismatch")
        require(restored_db.execute("PRAGMA integrity_check").fetchone()[0] == "ok", "SQLite integrity check failed")
        require(not restored_db.execute("PRAGMA foreign_key_check").fetchall(), "SQLite foreign key check failed")
    # Explicitly switch only this synthetic smoke configuration to the new path.
    config.write_text(config.read_text(encoding="utf-8").replace(database.as_posix(), restored.as_posix()),
                      encoding="utf-8")
    restored_before = sha(restored.read_bytes())
    restored_doctor = command(["doctor", "--read-only"])
    require(restored_doctor.get("status") == "ok" and sha(restored.read_bytes()) == restored_before,
            "restored read-only doctor failed or changed database bytes")
    with McpClient(binary, state, env, transcript) as client:
        client.initialize()
        after = client.tool("recall_memory", {"namespace": "project/package/a", "query": "violet"})
        require(after == current, "restored MCP recall changed the recorded result")
    # Avoid persisting private absolute temporary paths in portable evidence.
    def write_evidence(name, value):
        data = canonical(value).decode("utf-8")
        for representation in (str(root), root.as_posix()):
            data = data.replace(json.dumps(representation, ensure_ascii=False)[1:-1], "<fresh-temporary-root>")
        write_new(output / name, data.encode("utf-8"))
    write_evidence("init.json", init)
    write_evidence("doctor.json", doctor)
    write_evidence("doctor-restored.json", restored_doctor)
    write_evidence("mcp-transcript.json", transcript)
    return {"init": True, "doctor_read_only": True, "mcp_initialize": True, "mcp_tools_list": True,
            "mcp_ingest_restart_recall": True, "mcp_inspect_correct_history": True,
            "sqlite_integrity": True, "backup_restore_to_new_path": True,
            "restored_read_only_doctor_and_mcp_recall": True,
            "namespace_leaks": 0, "provider": "mock", "remote_model_calls": 0,
            "child_path_has_cargo": False, "child_path_has_rustc": False}


def verify(args):
    require(args.receipt.stat().st_size <= 1024 * 1024, "receipt is too large")
    receipt = json.loads(args.receipt.read_text(encoding="utf-8"))
    payload, binary = validate_package(args.archive, receipt, args.expected_commit, args.expected_tree)
    target, _ = native_target()
    require(receipt["target"] == target, "unpacked runtime verification requires the matching native host")
    output = empty_output(args.output)
    with tempfile.TemporaryDirectory(prefix="ledger-unpack-") as temporary:
        root = Path(temporary)
        unpacked = root / "package"
        for name, (data, mode) in payload.items():
            destination = unpacked / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            destination.chmod(mode)
        checks = smoke(unpacked / binary, root, output)
    summary = {"schema_version": 1, "kind": "portable_package_local_unpack_simulation", "status": "passed",
               "local_only": True, "real_fresh_machine_evidence": False, "real_user_client_evidence": False,
               "fresh_temporary_directory": True, "source": receipt["source"], "target": target,
               "archive_sha256": receipt["archive"]["sha256"], "checks": checks, "non_claims": NON_CLAIMS}
    summary_bytes = canonical(summary)
    evidence_hashes = {path.name: sha(path.read_bytes()) for path in output.iterdir()
                       if not path.name.startswith(".")}
    evidence_hashes["summary.json"] = sha(summary_bytes)
    hashes = "".join(value + "  " + name + "\n" for name, value in sorted(evidence_hashes.items()))
    write_new(output / "SHA256SUMS", hashes.encode("ascii"))
    # The passed summary is the final success marker, only after all evidence
    # and checksums have been written successfully.
    write_new(output / "summary.json", summary_bytes)
    print(json.dumps(summary))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    builder = commands.add_parser("build", help="compile an exact committed source snapshot on the native host")
    builder.add_argument("--candidate", required=True)
    builder.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    builder.add_argument("--source-ref", default="HEAD")
    builder.add_argument("--output", required=True, type=Path)
    builder.add_argument("--target-dir", type=Path)
    builder.add_argument("--offline", action="store_true", help="require already-cached Cargo dependencies")
    verifier = commands.add_parser("verify", help="strictly validate and run an unpacked archive with no Rust on PATH")
    verifier.add_argument("--archive", required=True, type=Path)
    verifier.add_argument("--receipt", required=True, type=Path)
    verifier.add_argument("--expected-commit", required=True)
    verifier.add_argument("--expected-tree", required=True)
    verifier.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        build(args) if args.command == "build" else verify(args)
    except (PackageError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError,
            tarfile.TarError, zipfile.BadZipFile, EOFError, zlib.error) as error:
        print(f"portable package failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
