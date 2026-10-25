#!/usr/bin/env bash
# Quality gate: no stubs and no hand-typed metrics in AegisFlow sources.
#
#   .claude/hooks/verify_no_stubs.sh          scan crates/ harness/ models/ policies/
#   .claude/hooks/verify_no_stubs.sh --hook   Claude Code PostToolUse mode: read the hook JSON on
#                                             stdin and scan only the file that was just written
#
# Flags: TODO / FIXME / XXX / HACK markers, todo!() / unimplemented!(), NotImplementedError,
# and numeric literals assigned to telemetry-looking keys (`"clonefile_ms": 0.3`, `p50_us: 1.2`).
# Keys that describe limits rather than measurements (target/timeout/budget/max/min/...) are allowed.
# Exit 2 on violations (Claude Code feeds stderr back to the agent), 0 when clean.
set -uo pipefail
ROOT="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT" || exit 0

payload=""
if [ "${1:-}" = "--hook" ]; then
  payload="$(cat || true)"
fi

AEGIS_HOOK_PAYLOAD="$payload" python3 - "$ROOT" <<'PY'
import json, os, re, sys
from pathlib import Path

root = Path(sys.argv[1])
scan_dirs = ["crates", "harness", "models", "policies"]
exts = {".rs", ".swift", ".py", ".sh", ".cedar", ".cedarschema", ".toml"}
skip_parts = {"target", ".venv", "__pycache__", ".build"}

stub = re.compile(r"\b(TODO|FIXME|XXX|HACK)\b|\bunimplemented!\s*\(|\btodo!\s*\(|raise\s+NotImplementedError")
fake = re.compile(
    r'"([A-Za-z0-9_]*(?:latency|_ms|_us|_ns)[A-Za-z0-9_]*)"\s*:\s*-?\d+(?:\.\d+)?'
    r"|\b([a-z0-9_]*(?:latency|_ms|_us|_ns))\s*:\s*-?\d+\.\d+"
)
allowed_key = re.compile(r"target|timeout|interval|deadline|budget|limit|max_|min_|cap|warmup|iterations", re.I)

def candidates():
    payload = os.environ.get("AEGIS_HOOK_PAYLOAD", "")
    if payload.strip():
        try:
            path = (json.loads(payload).get("tool_input") or {}).get("file_path", "")
        except json.JSONDecodeError:
            path = ""
        if not path:
            return []
        p = Path(path)
        p = p if p.is_absolute() else root / p
        try:
            rel = p.resolve().relative_to(root.resolve())
        except ValueError:
            return []
        if rel.parts and rel.parts[0] in scan_dirs and p.suffix in exts and p.is_file():
            return [p]
        return []
    files = []
    for d in scan_dirs:
        base = root / d
        if base.exists():
            files += [p for p in base.rglob("*") if p.is_file() and p.suffix in exts and not (set(p.parts) & skip_parts)]
    return sorted(files)

violations = []
for path in candidates():
    try:
        lines = path.read_text(errors="replace").splitlines()
    except OSError:
        continue
    for no, line in enumerate(lines, 1):
        if stub.search(line):
            violations.append(f"{path.relative_to(root)}:{no}: stub marker: {line.strip()[:120]}")
            continue
        for m in fake.finditer(line):
            key = m.group(1) or m.group(2) or ""
            if not allowed_key.search(key):
                violations.append(f"{path.relative_to(root)}:{no}: hardcoded metric `{key}`: {line.strip()[:120]}")

if violations:
    sys.stderr.write("verify_no_stubs: %d violation(s)\n" % len(violations))
    for v in violations[:50]:
        sys.stderr.write("  " + v + "\n")
    sys.exit(2)
print("verify_no_stubs: clean")
PY
