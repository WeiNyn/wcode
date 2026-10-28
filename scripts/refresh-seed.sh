#!/usr/bin/env bash
# Regenerate the compiled-in seed literals in
# crates/wcode-harness/src/catalog.rs from models.dev's opencode-go provider.
#
# Usage:
#   scripts/refresh-seed.sh            # fetch https://models.dev/api.json
#   scripts/refresh-seed.sh FILE.json  # read a saved payload
#   scripts/refresh-seed.sh --dry-run  # print the new consts, don't write
#
# Only SEED_LIMITS and SEED_PRICES are regenerated; SEED_RESPONSES_ONLY is
# hand-maintained (models.dev carries no wire-protocol fact) and is left alone.
set -euo pipefail

DRY_RUN=0
INPUT=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    *) INPUT="$arg" ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CATALOG="$ROOT/crates/wcode-harness/src/catalog.rs"

if [ -z "$INPUT" ]; then
  TMP="$(mktemp)"
  trap 'rm -f "$TMP"' EXIT
  echo "fetching models.dev..." >&2
  curl -sSf https://models.dev/api.json -o "$TMP"
  INPUT="$TMP"
fi

python3 - "$INPUT" "$CATALOG" "$DRY_RUN" << 'PY'
import json, re, sys

src_path = sys.argv[2]
dry_run = sys.argv[3] == "1"
with open(sys.argv[1]) as f:
    data = json.load(f)
prov = data["opencode-go"]
models = prov["models"]

def micro(x):
    # Rust f64::round() is round-half-away-from-zero.
    v = x * 1_000_000
    if v >= 0:
        return int(v + 0.5)
    return int(v - 0.5)

# SEED_LIMITS: every model with a context window.
limits = []
for mid in sorted(models):
    m = models[mid]
    lim = m.get("limit") or {}
    ctx = lim.get("context", 0)
    out = lim.get("output", 0)
    if ctx == 0 and out == 0:
        continue
    limits.append((mid, ctx, out))

# SEED_PRICES: every model with a cost block.
prices = []
for mid in sorted(models):
    m = models[mid]
    cost = m.get("cost")
    if not cost:
        continue
    prices.append((mid,
                   micro(cost.get("input", 0.0)),
                   micro(cost.get("output", 0.0)),
                   micro(cost.get("cache_read", 0.0)),
                   micro(cost.get("cache_write", 0.0))))

def fmt_limits(rows):
    lines = ["const SEED_LIMITS: &[(&str, u64, u64)] = &["]
    for mid, ctx, out in rows:
        lines.append(f'    ("{mid}", {ctx}, {out}),')
    lines.append("];")
    return "\n".join(lines)

def fmt_prices(rows):
    lines = ["const SEED_PRICES: &[(&str, u64, u64, u64, u64)] = &["]
    for mid, i, o, cr, cw in rows:
        lines.append(f'    ("{mid}", {i}, {o}, {cr}, {cw}),')
    lines.append("];")
    return "\n".join(lines)

with open(src_path) as f:
    text = f.read()

# Replace the SEED_LIMITS const block.
pat_limits = re.compile(
    r"const SEED_LIMITS: &\[.*?\] = &\[\n.*?\n\];",
    re.DOTALL,
)
new_limits = fmt_limits(limits)
assert pat_limits.search(text), "SEED_LIMITS block not found"
text2 = pat_limits.sub(new_limits, text, count=1)

pat_prices = re.compile(
    r"const SEED_PRICES: &\[.*?\] = &\[\n.*?\n\];",
    re.DOTALL,
)
new_prices = fmt_prices(prices)
assert pat_prices.search(text2), "SEED_PRICES block not found"
text2 = pat_prices.sub(new_prices, text2, count=1)

if dry_run:
    print(new_limits)
    print()
    print(new_prices)
    print(f"\n# {len(limits)} limits, {len(prices)} prices", file=sys.stderr)
    sys.exit(0)

with open(src_path, "w") as f:
    f.write(text2)
print(f"updated {src_path}: {len(limits)} limits, {len(prices)} prices", file=sys.stderr)
PY
