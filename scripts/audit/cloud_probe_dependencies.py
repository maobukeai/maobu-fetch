"""Select one coherent dependency graph from the last real application test build."""
import json
from pathlib import Path
import sys

deps = Path(sys.argv[1]) / "src-tauri/target/debug/deps"
fingerprints = deps.parent / ".fingerprint"
build = max(fingerprints.glob("maobu-fetch-*/test-lib-maobu_fetch_lib.json"), key=lambda p: p.stat().st_mtime)
expected = {name: value for _, name, _, value in json.loads(build.read_text())["deps"]}
selected = {}
for name in sys.argv[2:]:
    for value_path in fingerprints.glob(f"*/lib-{name}"):
        fingerprint = int.from_bytes(bytes.fromhex(value_path.read_text().strip()), "little")
        if fingerprint != expected[name]:
            continue
        suffix = value_path.parent.name.rsplit("-", 1)[1]
        library = deps / f"lib{name}-{suffix}.rlib"
        if library.is_file():
            selected[name] = str(library)
            break
    if name not in selected:
        raise RuntimeError(f"Missing coherent compiled dependency {name}; run project tests first")
print(json.dumps(selected))
