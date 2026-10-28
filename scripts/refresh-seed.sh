#!/usr/bin/env bash
# Regenerate the compiled-in seed literals in
# crates/wcode-harness/src/catalog.rs from models.dev's opencode-go provider.
#
# Usage:
#   scripts/refresh-seed.sh                # fetch https://models.dev/api.json
#   scripts/refresh-seed.sh FILE.json      # read a saved payload
#   scripts/refresh-seed.sh --dry-run      # print the new consts, don't write
#   scripts/refresh-seed.sh --prune        # DELETE rows the payload no longer has
#
# MERGE-PRESERVE BY DEFAULT. The seed is a SUPERset of the two retired static
# tables (D043): a row that is in the current const but absent from the payload
# is KEPT and listed on stderr, because offline/first-run behaviour has to match
# pre-W038 exactly — a model quietly vanishing from the seed is how `glm-5` lost
# its 202_752 window. Pass --prune when you really mean "delete what models.dev
# dropped".
#
# Where both name an id the payload (live) value WINS.
#
# Only SEED_LIMITS and SEED_PRICES are regenerated; SEED_RESPONSES_ONLY is
# hand-maintained (models.dev carries no wire-protocol fact) and is left alone.
set -euo pipefail

DRY_RUN=0
PRUNE=0
INPUT=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --prune) PRUNE=1 ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) echo "unknown flag: $arg" >&2; exit 2 ;;
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

python3 - "$INPUT" "$CATALOG" "$DRY_RUN" "$PRUNE" << 'PY'
import json, re, sys

src_path = sys.argv[2]
dry_run = sys.argv[3] == "1"
prune = sys.argv[4] == "1"

with open(sys.argv[1]) as f:
    data = json.load(f)
models = data["opencode-go"]["models"]

def micro(x):
    # Rust f64::round() is round-half-away-from-zero.
    v = x * 1_000_000
    if v >= 0:
        return int(v + 0.5)
    return int(v - 0.5)

# --- the compiled-in consts, as they are on disk -------------------------------
def parse_const(text, name):
    m = re.search(r"const %s: &\[.*?\] = &\[\n(.*?)\n\];" % name, text, re.DOTALL)
    if not m:
        sys.exit(f"{name} block not found in {src_path}")
    rows = {}
    for line in m.group(1).splitlines():
        line = line.strip()
        if not line.startswith("("):
            continue
        parts = re.findall(r'"([^"]*)"|([0-9]+(?:_[0-9]+)*)', line)
        vals = [a or b.replace("_", "") for a, b in parts]
        rows[vals[0]] = tuple(int(v) for v in vals[1:])
    return rows

with open(src_path) as f:
    text = f.read()
const_limits = parse_const(text, "SEED_LIMITS")
const_prices = parse_const(text, "SEED_PRICES")

# --- the payload ---------------------------------------------------------------
payload_limits = {}
payload_prices = {}
for mid in sorted(models):
    m = models[mid]
    lim = m.get("limit") or {}
    ctx = lim.get("context", 0)
    out = lim.get("output", 0)
    if ctx == 0 and out == 0:
        continue  # no window at all — nothing to seed (a 0 would read as UNKNOWN)
    payload_limits[mid] = (ctx, out)
for mid in sorted(models):
    cost = models[mid].get("cost")
    if not cost:
        continue
    payload_prices[mid] = (
        micro(cost.get("input", 0.0)),
        micro(cost.get("output", 0.0)),
        micro(cost.get("cache_read", 0.0)),
        micro(cost.get("cache_write", 0.0)),
    )

# --- merge (payload wins; const-only rows are kept unless --prune) ------------
def merge(const, payload):
    kept = sorted(set(const) - set(payload))
    merged = dict(const) if not prune else {}
    merged.update(payload)
    return {k: merged[k] for k in sorted(merged)}, kept

limits, kept_limits = merge(const_limits, payload_limits)
prices, kept_prices = merge(const_prices, payload_prices)

def report(label, merged, payload, kept):
    print(f"{label}: {len(merged)} rows · payload {len(payload)}"
          f" · kept {0 if prune else len(kept)} · dropped {len(kept) if prune else 0}"
          + ("" if prune else " (merge-preserve; --prune deletes)"),
          file=sys.stderr)
    for mid in kept:
        print(f"  {'dropped' if prune else 'kept'}: {mid}", file=sys.stderr)

print(f"payload: {len(payload_limits)} limit rows, {len(payload_prices)} price rows",
      file=sys.stderr)
report("SEED_LIMITS", limits, payload_limits, kept_limits)
report("SEED_PRICES", prices, payload_prices, kept_prices)

def fmt_limits(rows):
    lines = ["const SEED_LIMITS: &[(&str, u64, u64)] = &["]
    for mid in sorted(rows):
        lines.append('    ("%s", %d, %d),' % (mid, *rows[mid]))
    lines.append("];")
    return "\n".join(lines)

def fmt_prices(rows):
    lines = ["const SEED_PRICES: &[(&str, u64, u64, u64, u64)] = &["]
    for mid in sorted(rows):
        lines.append('    ("%s", %d, %d, %d, %d),' % (mid, *rows[mid]))
    lines.append("];")
    return "\n".join(lines)

new_limits = fmt_limits(limits)
new_prices = fmt_prices(prices)
for name, new in (("SEED_LIMITS", new_limits), ("SEED_PRICES", new_prices)):
    pat = re.compile(r"const %s: &\[.*?\] = &\[\n.*?\n\];" % name, re.DOTALL)
    if not pat.search(text):
        sys.exit(f"{name} block not found in {src_path}")
    text = pat.sub(lambda _m, blk=new: blk, text, count=1)

if dry_run:
    print(new_limits)
    print()
    print(new_prices)
    print(f"\n# {len(limits)} limits, {len(prices)} prices "
          f"(kept {len(kept_limits)} limits / {len(kept_prices)} prices)",
          file=sys.stderr)
    sys.exit(0)

with open(src_path, "w") as f:
    f.write(text)
print(f"updated {src_path}: {len(limits)} limits, {len(prices)} prices "
      f"(kept {len(kept_limits)} limits / {len(kept_prices)} prices)", file=sys.stderr)
PY
