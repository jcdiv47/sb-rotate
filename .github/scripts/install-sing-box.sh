#!/usr/bin/env bash
# Usage: install-sing-box.sh TAG [SHA256]
# Downloads the linux-amd64 sing-box release TAG, verifies it against SHA256
# (or the digest GitHub records for the asset), and exports SING_BOX.
set -euo pipefail

tag="$1"
expected="${2:-}"
name="sing-box-${tag#v}-linux-amd64.tar.gz"
archive="$RUNNER_TEMP/sing-box.tar.gz"

if [[ -z "$expected" ]]; then
  expected="$(gh api "repos/SagerNet/sing-box/releases/tags/$tag" \
    --jq ".assets[] | select(.name == \"$name\") | .digest")"
  expected="${expected#sha256:}"
  [[ -n "$expected" ]] || { echo "No digest published for $name" >&2; exit 1; }
fi

curl --fail --location --retry 3 \
  "https://github.com/SagerNet/sing-box/releases/download/$tag/$name" \
  --output "$archive"
echo "$expected  $archive" | sha256sum --check --strict

python3 - <<'PY'
import os
from pathlib import Path
import shutil
import tarfile

root = Path(os.environ['RUNNER_TEMP'])
with tarfile.open(root / 'sing-box.tar.gz', 'r:gz') as archive:
    binaries = [entry for entry in archive.getmembers()
                if entry.isfile() and Path(entry.name).name == 'sing-box']
    if len(binaries) != 1:
        raise SystemExit('Expected exactly one sing-box binary')
    with archive.extractfile(binaries[0]) as source, (root / 'sing-box').open('wb') as output:
        shutil.copyfileobj(source, output)
(root / 'sing-box').chmod(0o755)
PY

"$RUNNER_TEMP/sing-box" version
echo "SING_BOX=$RUNNER_TEMP/sing-box" >> "$GITHUB_ENV"
