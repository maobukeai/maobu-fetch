"""Verify vendored production files against the checked official crate archive."""
import argparse
import hashlib
import pathlib
import tarfile

EXPECTED = "10003b9b157ef1d8352b4ff2e2025fa1544453d40539741779ee918028aba75b"
PATCHED = {
    "src/limits.rs", "src/session.rs", "src/torrent_state/mod.rs",
    "src/torrent_state/live/mod.rs",
    "src/bitv.rs", "src/session_persistence/json.rs",
    "src/torrent_state/initializing.rs",
    "src/create_torrent_file.rs",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crate", type=pathlib.Path)
    args = parser.parse_args()
    archive = args.crate
    if archive is None:
        archive = next((pathlib.Path.home() / ".cargo/registry/cache").glob("*/librqbit-9.0.0.crate"), None)
    if archive is None:
        parser.error("Supply an already cached official archive with --crate")
    if hashlib.sha256(archive.read_bytes()).hexdigest() != EXPECTED:
        raise SystemExit("Official crate SHA-256 mismatch")
    root = pathlib.Path(__file__).resolve().parents[2] / "src-tauri/vendor/librqbit"
    unchanged = 0
    modified = set()
    with tarfile.open(archive) as source:
        for member in source.getmembers():
            if not member.isfile():
                continue
            name = str(pathlib.PurePosixPath(member.name).relative_to("librqbit-9.0.0"))
            target = root / name
            if not target.exists():
                continue
            with source.extractfile(member) as original:
                same = original.read() == target.read_bytes()
            if same:
                unchanged += 1
            elif name in PATCHED:
                modified.add(name)
            else:
                raise SystemExit("Unapproved upstream production change: " + name)
    if modified != PATCHED:
        raise SystemExit("Expected patch set differs: " + repr(modified))
    if not (root / "LICENSE").is_file():
        raise SystemExit("Pinned upstream LICENSE is missing")
    print(f"Verified official SHA-256, {unchanged} unchanged files, {len(modified)} approved patches and LICENSE.")


if __name__ == "__main__":
    main()
