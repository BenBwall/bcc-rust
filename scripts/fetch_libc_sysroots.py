#!/usr/bin/env python3
"""Download pinned libc binary packages and extract their headers as sysroots.

Run from the repository root. Each sysroot lands in target/sysroots/<name>/
with its headers under usr/include/ and a sysroot.json manifest. Packages are
official distribution builds, so their headers are generated and installed;
nothing from a package is executed. Downloads are checked against the pinned
SHA-256 before extraction and kept in target/sysroots/downloads/.
"""

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import shutil
import tarfile
import urllib.parse
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
INCLUDE_PREFIX = "usr/include/"

# Debian 13 (trixie) through snapshot.debian.org, whose timestamped archive
# URLs never change. The hashes match the trixie and trixie-security Packages
# indexes. linux-libc-dev is Architecture: all from Linux 6.5 on: each
# multiarch asm/ directory holds symlinks into usr/lib/linux/uapi/<arch>/.
# Alpine 3.20 reached end of life on 2026-04-01, so its repository is frozen
# and these package URLs stay fixed.
SYSROOTS = {
    "glibc-x86_64-linux": {
        "distribution": "Debian 13 (trixie), amd64",
        "packages": [
            {
                "name": "libc6-dev", "version": "2.41-12+deb13u4", "format": "deb",
                "url": "https://snapshot.debian.org/archive/debian/20260711T202405Z/"
                       "pool/main/g/glibc/libc6-dev_2.41-12%2Bdeb13u4_amd64.deb",
                "size": 1994660,
                "sha256": "1fda734dabcd80b77266a09ab62b0f1e3e16d8091db62899890b2200745632a2",
            },
            {
                "name": "linux-libc-dev", "version": "6.12.111-1", "format": "deb",
                "url": "https://snapshot.debian.org/archive/debian-security/20260929T095154Z/"
                       "pool/updates/main/l/linux/linux-libc-dev_6.12.111-1_all.deb",
                "size": 2988444,
                "sha256": "b9055566f71a6fa2e818174dcc3622aa0fb5d16cdff1bc5346e52492ba27b57c",
            },
        ],
    },
    "musl-x86_64-linux": {
        "distribution": "Alpine Linux 3.20, x86_64",
        "packages": [
            {
                "name": "musl-dev", "version": "1.2.5-r3", "format": "apk",
                "url": "https://dl-cdn.alpinelinux.org/alpine/v3.20/main/x86_64/musl-dev-1.2.5-r3.apk",
                "size": 3437187,
                "sha256": "36abcf8a199826080b9b2a45f86782afae4ae5c8b8331909e5113911e2bdcad1",
            },
            {
                "name": "linux-headers", "version": "6.6-r0", "format": "apk",
                "url": "https://dl-cdn.alpinelinux.org/alpine/v3.20/main/x86_64/linux-headers-6.6-r0.apk",
                "size": 1633580,
                "sha256": "eac03f9d5ac52a86734f73bb444272aebf323e6dc03e9b69f87545f9da122f8a",
            },
        ],
    },
}
WINDOWS_DEVICE_NAMES = {"con", "prn", "aux", "nul", *(f"com{n}" for n in range(1, 10)),
                        *(f"lpt{n}" for n in range(1, 10))}


def sha256(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def download(package, downloads):
    """Return the verified package file, downloading it into its own directory."""
    name = urllib.parse.unquote(package["url"].rsplit("/", 1)[1])
    directory = downloads / f"{package['name']}_{package['version']}".replace("+", "_")
    archive = directory / name
    if directory.exists():
        if sorted(item.name for item in directory.iterdir()) != [name]:
            raise RuntimeError(f"{directory} holds unexpected files; remove it and retry")
    else:
        directory.mkdir(parents=True)
        temporary = directory / (name + ".download")
        print(f"Downloading {package['url']}", flush=True)
        with urllib.request.urlopen(package["url"], timeout=120) as response:
            with temporary.open("xb") as output:
                shutil.copyfileobj(response, output)
        temporary.replace(archive)
    digest = sha256(archive)
    if digest != package["sha256"]:
        raise RuntimeError(f"Checksum mismatch for {archive}: expected {package['sha256']}, "
                           f"found {digest}; remove {directory} and retry")
    return archive


def ar_member(data, prefix):
    """Return the name and contents of the first `ar` member starting with prefix."""
    if data[:8] != b"!<arch>\n":
        raise RuntimeError("not an ar archive")
    position = 8
    while position + 60 <= len(data):
        header = data[position:position + 60]
        if header[58:60] != b"`\n":
            raise RuntimeError("malformed ar member header")
        name = header[:16].decode("ascii").strip().rstrip("/")
        size = int(header[48:58].decode("ascii"))
        start = position + 60
        if name.startswith(prefix):
            return name, data[start:start + size]
        position = start + size + (size & 1)
    raise RuntimeError(f"no {prefix}* member")


def open_data(archive, package_format):
    """Open the package's file tree as a tar archive; `.deb` data may be xz or zstd."""
    if package_format == "deb":
        member, data = ar_member(archive.read_bytes(), "data.tar")
        # Python 3.14 reads .zst through compression.zstd; older versions need xz data.
        return tarfile.open(fileobj=io.BytesIO(data), mode="r:*"), member
    # An .apk concatenates gzip streams (signature, control, data) whose tar
    # segments omit end-of-archive blocks, so they read as one tar stream.
    return tarfile.open(fileobj=gzip.open(archive), mode="r|", ignore_zeros=True), "gzip tar"


def safe_name(name):
    """Normalize an archive member name, rejecting anything that could escape."""
    if name.startswith("/") or "\\" in name or ":" in name:
        raise RuntimeError(f"Unsafe archive member: {name!r}")
    path = PurePosixPath(name)
    if ".." in path.parts:
        raise RuntimeError(f"Unsafe archive member: {name!r}")
    normalized = path.as_posix()
    return "" if normalized == "." else normalized.removeprefix("./")


def resolve(name, entries, depth=0):
    """Follow symlinks in any component of name, staying within the archive root."""
    if depth > 40:
        raise RuntimeError(f"Symlink loop at {name}")
    parts = name.split("/")
    for index in range(1, len(parts) + 1):
        prefix = "/".join(parts[:index])
        entry = entries.get(prefix)
        if entry is None or not (entry.issym() or entry.islnk()):
            continue
        target = entry.linkname
        if target.startswith("/") or "\\" in target or ":" in target:
            raise RuntimeError(f"Link {prefix} -> {target} leaves the sysroot")
        # A symlink is relative to its directory; a hard link names a member.
        if entry.issym():
            target = posixpath.join(posixpath.dirname(prefix), target)
        else:
            target = target.removeprefix("./")
        joined = posixpath.normpath(posixpath.join(target, *parts[index:]))
        if joined == ".." or joined.startswith("../") or joined.startswith("/"):
            raise RuntimeError(f"Link {prefix} -> {entry.linkname} leaves the sysroot")
        return resolve(joined, entries, depth + 1)
    return name


def plan(members):
    """Map each needed regular file in the archive to the header paths it supplies.

    Headers are regular files under usr/include/. Links there become copies
    of their targets, which may lie elsewhere in the package (Debian points
    asm/ headers into usr/lib/linux/uapi/); no link is created on disk.
    """
    entries = {safe_name(member.name): member for member in members}
    destinations = {}
    for name, member in entries.items():
        if not name.startswith(INCLUDE_PREFIX) or member.isdir():
            continue
        if not (member.isfile() or member.issym() or member.islnk()):
            continue
        target = resolve(name, entries)
        entry = entries.get(target)
        if entry is not None and entry.isfile():
            destinations.setdefault(target, []).append(name)
        elif entry is not None and entry.isdir():
            # A directory link supplies every file beneath its target.
            for inner, inner_member in entries.items():
                if inner.startswith(target + "/") and not inner_member.isdir():
                    final = resolve(inner, entries)
                    if final in entries and entries[final].isfile():
                        destinations.setdefault(final, []).append(name + inner[len(target):])
        else:
            raise RuntimeError(f"Link {name} has no regular file target in the package")
    return destinations


def write_header(staging, relative, content, skipped):
    parts = relative.split("/")
    if any(part.split(".")[0].lower() in WINDOWS_DEVICE_NAMES for part in parts):
        skipped.append({"path": relative, "reason": "reserved Windows device name"})
        return False
    destination = staging.joinpath(*parts)
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        with destination.open("xb") as output:
            output.write(content)
    except FileExistsError:
        # Linux headers such as xt_MARK.h and xt_mark.h differ only in case.
        if destination.name in os.listdir(destination.parent):
            raise RuntimeError(f"Two packages both supply {relative}") from None
        skipped.append({"path": relative, "reason": "case-insensitive name collision"})
        return False
    return True


def extract(archive, package, staging, skipped):
    """Copy package headers into staging and return how many were written."""
    tar, member = open_data(archive, package["format"])
    with tar:
        members = [item for item in tar]
    destinations = plan(members)
    # Streaming archives cannot seek, so read the contents in a second pass.
    tar, _ = open_data(archive, package["format"])
    written = 0
    with tar:
        for item in tar:
            name = safe_name(item.name)
            if item.isfile() and name in destinations:
                content = tar.extractfile(item).read()
                for relative in destinations.pop(name):
                    written += write_header(staging, relative, content, skipped)
    if destinations:
        raise RuntimeError(f"{archive.name}: missing link targets {sorted(destinations)[:5]}")
    return written, member


def build(name, spec, sysroots):
    destination = sysroots / name
    manifest_path = destination / "sysroot.json"
    expected = [{key: package[key] for key in ("name", "version", "url", "size", "sha256")}
                for package in spec["packages"]]
    if destination.exists():
        if manifest_path.exists():
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            if manifest.get("packages") == expected:
                print(f"{name}: up to date in {destination}", flush=True)
                return manifest
        raise RuntimeError(f"{destination} exists with other contents; remove it and retry")
    archives = [download(package, sysroots / "downloads") for package in spec["packages"]]
    staging = sysroots / f"{name}.partial"
    if staging.exists():
        raise RuntimeError(f"{staging} remains from an interrupted run; remove it and retry")
    staging.mkdir(parents=True)
    skipped = []
    try:
        counts = []
        for archive, package in zip(archives, spec["packages"]):
            written, member = extract(archive, package, staging, skipped)
            counts.append({"package": package["name"], "data": member, "headers": written})
            print(f"{name}: {package['name']} {package['version']}: {written} headers from {member}",
                  flush=True)
        manifest = {"name": name, "distribution": spec["distribution"], "packages": expected,
                    "extracted": counts, "skipped": skipped}
        (staging / "sysroot.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    except BaseException:
        # Only this run's freshly created staging directory is removed.
        shutil.rmtree(staging)
        raise
    staging.rename(destination)
    for item in skipped:
        print(f"{name}: skipped {item['path']} ({item['reason']})", flush=True)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sysroots", type=Path, default=ROOT / "target/sysroots",
                        help="Directory to hold sysroots and downloads")
    parser.add_argument("--only", action="append", choices=sorted(SYSROOTS),
                        help="Fetch only this sysroot (repeatable)")
    parser.add_argument("--list", action="store_true", help="Print the pinned packages and exit")
    args = parser.parse_args()
    names = args.only or sorted(SYSROOTS)
    if args.list:
        for name in names:
            for package in SYSROOTS[name]["packages"]:
                print(f"{name}\t{package['name']}\t{package['version']}\t{package['size']}\t"
                      f"{package['sha256']}\t{package['url']}")
        return 0
    sysroots = args.sysroots.resolve()
    total = sum(package["size"] for name in names for package in SYSROOTS[name]["packages"])
    print(f"Pinned packages total {total} bytes ({total / 1e6:.1f} MB)", flush=True)
    for name in names:
        build(name, SYSROOTS[name], sysroots)
        print(f"{name}: {sysroots / name}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
