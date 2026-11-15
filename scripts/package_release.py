"""Build a native development archive; no tag or release publication."""
from __future__ import annotations
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-unknown-linux-gnu": {"runner": "ubuntu-24.04", "baseline": "Ubuntu 24.04, glibc 2.39"},
    "aarch64-apple-darwin": {"runner": "macos-15", "baseline": "macOS 15, Apple Silicon"},
}
APPROVED_LICENSES = {
    "MIT", "MIT OR Apache-2.0", "Apache-2.0 OR MIT", "Apache-2.0/MIT",
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT", "Unlicense OR MIT",
    "BSD-3-Clause", "(MIT OR Apache-2.0) AND Unicode-3.0",
}

def command(args, **kwargs):
    return subprocess.check_output(args, cwd=ROOT, text=True, **kwargs).strip()

def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n")

def production_packages(metadata):
    """Metadata is already filtered for the native target; dev edges stay out."""
    packages = {item["id"]: item for item in metadata["packages"]}
    nodes = {item["id"]: item for item in metadata["resolve"]["nodes"]}
    pending = [next(item["id"] for item in metadata["packages"] if item["name"] == "kiarni")]
    seen = set()
    while pending:
        item = pending.pop()
        if item in seen:
            continue
        seen.add(item)
        pending.extend(dep["pkg"] for dep in nodes[item]["deps"]
                       if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]))
    return sorted((packages[item] for item in seen), key=lambda p: (p["name"], p["version"]))

def notices(packages, output, root=ROOT):
    inventory = []
    sections = ["Third-party notices for Kiarni\n\n"
                "Normal and build dependencies of the selected target; development dependencies excluded.\n"
                "License alternatives are preserved in full. Kiarni remains MIT.\n"
                "Rust standard library notices are in toolchain/COPYRIGHT-library.html.\n"]
    for package in packages:
        license_id = package.get("license")
        if license_id not in APPROVED_LICENSES:
            raise ValueError(f"unreviewed license: {package['name']}")
        source = Path(package["manifest_path"]).parent
        internal = package["source"] is None
        if internal:
            if not (source.is_relative_to(root / "apps") or source.is_relative_to(root / "crates")):
                raise ValueError("product graph includes an unexpected workspace package")
            files = [root / "LICENSE"]
        else:
            files = sorted(p for p in source.iterdir()
                           if p.name.lower().startswith(("license", "licence", "notice", "copying", "copyright")))
            if package.get("license_file"):
                extra = Path(package["license_file"])
                if not extra.is_absolute():
                    extra = source / extra
                if not extra.resolve().is_relative_to(source.resolve()):
                    raise ValueError("license material outside package")
                files = sorted(set(files + [extra]))
        if not files:
            raise ValueError(f"license material missing: {package['name']}")
        material = []
        for file in files:
            if file.is_symlink() or not file.is_file() or not 0 < file.stat().st_size <= 16 * 1024 * 1024:
                raise ValueError("invalid license material")
            text = file.read_text(encoding="utf-8")
            sections.append(f"\n--- {package['name']} {package['version']} / {file.name} ---\n{text}\n")
            material.append({"name": file.name, "sha256": sha256(file)})
        if "Unicode-3.0" in license_id and not any("unicode" in f["name"].lower() for f in material):
            raise ValueError("Unicode notice missing")
        inventory.append({"name": package["name"], "version": package["version"],
                          "license": license_id, "workspace": internal, "notice_files": material})
    (output / "THIRD_PARTY_NOTICES.txt").write_text("".join(sections), encoding="utf-8")
    write_json(output / "DEPENDENCIES.json", {"scope": "normal+build; target-filtered; dev excluded", "packages": inventory})

def rust_notices(sysroot, output):
    source = sysroot / "share/doc/rust"
    destination = output / "toolchain"
    destination.mkdir()
    copyright_file = source / "COPYRIGHT-library.html"
    if not copyright_file.is_file() or not (source / "licenses").is_dir():
        raise ValueError("Rust standard library notices missing from pinned toolchain")
    shutil.copyfile(copyright_file, destination / copyright_file.name)
    shutil.copytree(source / "licenses", destination / "licenses")
    if any(p.is_symlink() for p in destination.rglob("*")):
        raise ValueError("linked toolchain notices are not accepted")

def checksums(directory):
    files = sorted(p for p in directory.rglob("*") if p.is_file() and p.name != "CHECKSUMS.sha256")
    (directory / "CHECKSUMS.sha256").write_text("".join(
        f"{sha256(path)}  {path.relative_to(directory).as_posix()}\n" for path in files))

def archive(directory, destination):
    """Fixed tar metadata and gzip header; reproducibility of Rust itself is separate."""
    with destination.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for path in [directory, *sorted(directory.rglob("*"))]:
                    if path.is_symlink() or not (path.is_file() or path.is_dir()):
                        raise ValueError("only regular package files and directories are accepted")
                    info = tarfile.TarInfo(path.relative_to(directory.parent).as_posix())
                    info.uid = info.gid = info.mtime = 0
                    info.uname = info.gname = ""
                    info.mode = 0o755 if path.is_dir() or path == directory / "kiarni" else 0o644
                    if path.is_dir():
                        info.type = tarfile.DIRTYPE
                        tar.addfile(info)
                    else:
                        info.size = path.stat().st_size
                        with path.open("rb") as file:
                            tar.addfile(info, file)

def system_libraries(binary, target):
    if target.endswith("linux-gnu"):
        text = command(["ldd", str(binary)])
        libraries = sorted({Path(line.strip().split()[0]).name for line in text.splitlines() if line.strip()})
        allowed = {"linux-vdso.so.1", "ld-linux-x86-64.so.2", "libc.so.6", "libgcc_s.so.1", "libm.so.6", "libpthread.so.0", "libdl.so.2", "librt.so.1", "libresolv.so.2"}
        if not set(libraries) <= allowed or "not found" in text:
            raise ValueError("unexpected Linux shared dependency")
    else:
        libraries = sorted(line.strip().split()[0] for line in command(["otool", "-L", str(binary)]).splitlines()[1:])
        if not all(name.startswith(("/usr/lib/", "/System/Library/Frameworks/")) for name in libraries):
            raise ValueError("unexpected macOS shared dependency")
        if not re.search(r"\bminos\s+15(?:\.0){0,2}\b", command(["otool", "-l", str(binary)])):
            raise ValueError("unexpected macOS deployment target")
    return libraries

def build(target, output, allow_dirty=False):
    required = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    compiler = command(["rustc", "-Vv"])
    fields = dict(line.split(": ", 1) for line in compiler.splitlines() if ": " in line)
    if fields.get("release") != required or fields.get("host") != target:
        raise ValueError("native host and pinned Rust toolchain are required")
    if target.endswith("linux-gnu") and platform.libc_ver() != ("glibc", "2.39"):
        raise ValueError("Linux build requires the glibc 2.39 baseline")
    if target.endswith("apple-darwin") and not platform.mac_ver()[0].startswith("15."):
        raise ValueError("macOS build requires the macOS 15 baseline")
    forbidden = [name for name in os.environ if os.environ[name] and
                 (name in {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_DIR"} or
                  name.startswith("CARGO_PROFILE_RELEASE_") or
                  (name.startswith("CARGO_TARGET_") and name.endswith(("_RUSTFLAGS", "_LINKER"))))]
    if forbidden:
        raise ValueError("external build/profile flags are not accepted")
    dirty = bool(command(["git", "status", "--porcelain"]))
    if dirty and (not allow_dirty or os.environ.get("GITHUB_ACTIONS") == "true"):
        raise ValueError("clean checkout required; --allow-dirty is local QA only")
    revision = command(["git", "rev-parse", "HEAD"])
    if not re.fullmatch(r"[a-f0-9]{40}", revision):
        raise ValueError("invalid source revision")
    environment = dict(os.environ)
    if target.endswith("apple-darwin"):
        if environment.get("MACOSX_DEPLOYMENT_TARGET", "15.0") != "15.0":
            raise ValueError("external macOS deployment target is not accepted")
        environment["MACOSX_DEPLOYMENT_TARGET"] = "15.0"
    subprocess.run(["cargo", "build", "--package", "kiarni", "--release", "--locked", "--target", target], cwd=ROOT, env=environment, check=True)
    metadata = json.loads(command(["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target]))
    packages = production_packages(metadata)
    version = next(p["version"] for p in packages if p["name"] == "kiarni")
    binary = ROOT / "target" / target / "release/kiarni"
    if binary.is_symlink() or not binary.is_file():
        raise ValueError("regular product binary missing")
    if command([str(binary), "--version"]) != f"kiarni {version}":
        raise ValueError("binary version mismatch")
    repository = os.environ.get("GITHUB_REPOSITORY", "sergiomonteiro/kiarni")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("invalid source repository")
    name = f"kiarni-{version}-{revision[:12]}{'-dirty' if dirty else ''}-{target}"
    output.mkdir(parents=True, exist_ok=True)
    destination = output / f"{name}.tar.gz"
    if destination.exists() or destination.with_name(destination.name + ".sha256").exists():
        raise ValueError("output already exists")
    with tempfile.TemporaryDirectory(prefix="kiarni-package-") as temporary:
        package = Path(temporary) / name
        package.mkdir()
        shutil.copyfile(binary, package / "kiarni")
        (package / "kiarni").chmod(0o755)
        shutil.copyfile(ROOT / "LICENSE", package / "LICENSE")
        notices(packages, package)
        rust_notices(Path(command(["rustc", "--print", "sysroot"])), package)
        write_json(package / "BUILD_INFO.json", {
            "schema": 1, "channel": "development", "version": version, "target": target,
            "commit": revision, "dirty": dirty, "repository": repository,
            "rust_version": required, "rustc_verbose": compiler,
            "build_os": platform.system(), "build_os_release": platform.release(),
            "build_macos": platform.mac_ver()[0], "build_libc": list(platform.libc_ver()),
            "baseline": TARGETS[target]["baseline"], "system_libraries": system_libraries(binary, target),
            "build_command": f"cargo build --package kiarni --release --locked --target {target}",
        })
        url = f"https://github.com/{repository}/blob/{revision}/docs"
        (package / "INSTALL.md").write_text(f"""# Kiarni {version} — experimental development binary

Target: {target}. Validated baseline: {TARGETS[target]['baseline']}.
Source commit: {revision}. Dirty local build: {str(dirty).lower()}.
This is not an approved stable release or a completed MVP.

Verify the archive SHA-256 using its adjacent .sha256 file before extraction:
Linux: sha256sum -c ARCHIVE.tar.gz.sha256
macOS: shasum -a 256 -c ARCHIVE.tar.gz.sha256
Hashes detect corruption, not publisher identity. Use a trusted source.

After extraction, verify internal CHECKSUMS.sha256 in this directory with the
same checksum tool. Copy kiarni to a directory you control (for example
$HOME/.local/bin), preserving executable permissions. No Rust, Python, Node or
other runtime service is required by the binary; system libraries listed in
BUILD_INFO.json are provided by the operating system.

./kiarni --version
./kiarni serve --data-dir ./data

The data parent must exist. The server defaults to 127.0.0.1:8711 and delivers
a new credential through a private sibling directory. Ctrl+C or SIGTERM stops
it gracefully. Never place the token in URLs, command arguments or logs.

Offline operations, with server stopped:
./kiarni auth rotate --data-dir ./data
./kiarni auth revoke-all --data-dir ./data
./kiarni backup --data-dir ./data --output ./backups/first
./kiarni restore --from ./backups/first --data-dir ./restores/data

Prepare private backup/restore parent directories first. Destinations must be
new; restore issues new credentials only at the first serve. See the complete
contracts before using backups: {url}/BACKUP.md and {url}/RESTORE.md.
Remote operation uses the optional HTTPS proxy profile: {url}/REMOTE_HTTPS.md.
Installation and platform limits: {url}/DISTRIBUTION.md.

Unsigned/unnotarized development artifacts may be refused by macOS Gatekeeper.
This pipeline does not disable OS security or prove every download path. Other
architectures/OS versions and bit-for-bit rebuilds are not certified here.
Licenses: LICENSE, THIRD_PARTY_NOTICES.txt and toolchain/COPYRIGHT-library.html.
""")
        checksums(package)
        archive(package, destination)
    destination.with_name(destination.name + ".sha256").write_text(f"{sha256(destination)}  {destination.name}\n")
    print(destination.name)
    return destination

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    parser.add_argument("--allow-dirty", action="store_true")
    args = parser.parse_args()
    build(args.target, args.output, args.allow_dirty)
