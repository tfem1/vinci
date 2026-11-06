#!/usr/bin/env bash
# Set the version everywhere: Cargo workspace, package.json, tauri.conf.json.
#   scripts/bump-version.sh 0.2.0          (stable)
#   scripts/bump-version.sh 0.3.0-beta.1   (beta — published as a pre-release)
set -euo pipefail
cd "$(dirname "$0")/.."
V="${1:?usage: $0 <semver>}"
[[ "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "not a semver: $V" >&2; exit 1; }
python3 - "$V" <<'PY'
import json, re, sys
v = sys.argv[1]
p = "Cargo.toml"; s = open(p).read()
s = re.sub(r'(\[workspace\.package\][^\[]*?\nversion = ")[^"]+(")', lambda m: m.group(1) + v + m.group(2), s, count=1, flags=re.S)
open(p, "w").write(s)
for p in ("package.json", "src-tauri/tauri.conf.json"):
    d = json.load(open(p)); d["version"] = v
    open(p, "w").write(json.dumps(d, indent=2, ensure_ascii=False) + "\n")
PY
grep -n '^version' Cargo.toml | head -1
echo "version set to $V — commit, then: git tag v$V && git push --tags"
