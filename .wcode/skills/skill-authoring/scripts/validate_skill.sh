#!/bin/sh
# validate_skill.sh — check every SKILL.md under .wcode/skills/** against the
# wcode skill standard (frontmatter, name, description, body size, layout).
#
# Usage (from the repo root, no arguments):
#   sh .wcode/skills/skill-authoring/scripts/validate_skill.sh
#
# Exits 0 when every skill passes; on failure exits 1 and prints one
#   <file>: <reason>
# line per problem (frontmatter delimiter, name rule, description cap, body
# size, README presence, nesting depth). A missing negative trigger in the
# description is a printed warning, not a failure.
#
# Discovery depth is bounded to 4 dirs below .wcode/skills, matching
# crates/wcode-cli/src/skills.rs (MAX_SCAN_DEPTH = 4).
set -eu

root=.wcode/skills
status=0

if [ ! -d "$root" ]; then
  printf '%s: no such directory (run from the repo root)\n' "$root"
  exit 1
fi

fail() {
  # $1 = file, $2 = reason
  printf '%s: %s\n' "$1" "$2"
  status=1
}

files=$(find "$root" -maxdepth 5 -name SKILL.md | sort)
if [ -z "$files" ]; then
  printf '%s: no SKILL.md found\n' "$root"
  exit 1
fi

while IFS= read -r file; do
  [ -n "$file" ] || continue

  # --- frontmatter delimiters ---------------------------------------------
  first=$(sed -n '1p' "$file")
  if [ "$first" != "---" ]; then
    fail "$file" "frontmatter must start with '---' on line 1"
    continue
  fi
  close=$(awk 'NR > 1 && $0 ~ /^---[[:space:]]*$/ { print NR; exit }' "$file")
  if [ -z "$close" ]; then
    fail "$file" "frontmatter has no closing '---' line"
    continue
  fi

  fm=$(sed -n "2,$((close - 1))p" "$file")

  # --- name: present, [a-z0-9-], 1-64, no leading/trailing/double hyphen ---
  name=$(printf '%s\n' "$fm" | sed -n 's/^name:[[:space:]]*//p' | head -n 1)
  name=$(printf '%s' "$name" | sed -e 's/[[:space:]]*$//' -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")
  if [ -z "$name" ]; then
    fail "$file" "missing required frontmatter key 'name'"
    continue
  fi
  namelen=${#name}
  if [ "$namelen" -lt 1 ] || [ "$namelen" -gt 64 ]; then
    fail "$file" "name '$name' is $namelen chars (must be 1-64)"
    continue
  fi
  case "$name" in
    -*) fail "$file" "name '$name' must not start with '-'"; continue ;;
    *-) fail "$file" "name '$name' must not end with '-'"; continue ;;
    *--*) fail "$file" "name '$name' must not contain '--'"; continue ;;
  esac
  bad=$(printf '%s' "$name" | sed 's/[a-z0-9-]//g')
  if [ -n "$bad" ]; then
    fail "$file" "name '$name' may only contain [a-z0-9-] (offending: '$bad')"
    continue
  fi

  # --- the directory name must equal `name` -------------------------------
  dir=$(dirname "$file")
  base=$(basename "$dir")
  if [ "$base" != "$name" ]; then
    fail "$file" "name '$name' must equal its directory name '$base'"
  fi

  # --- description: present, <= 1024 chars, negative trigger (warn only) ---
  desc=$(printf '%s\n' "$fm" | sed -n 's/^description:[[:space:]]*//p' | head -n 1)
  desc=$(printf '%s' "$desc" | sed -e 's/[[:space:]]*$//' -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")
  if [ -z "$desc" ]; then
    fail "$file" "missing required frontmatter key 'description'"
  else
    dlen=$(printf '%s' "$desc" | wc -m | tr -d ' ')
    if [ "$dlen" -gt 1024 ]; then
      fail "$file" "description is $dlen chars (must be <= 1024)"
    fi
    case "$desc" in
      *"not for"*|*"Not for"*|*"don't use"*|*"Don't use"*|*"not use"*|*"Not use"*) ;;
      *) printf 'warning: %s: description has no explicit negative trigger (e.g. "not for X")\n' "$file" >&2 ;;
    esac
  fi

  # --- the body must be under 500 lines ------------------------------------
  total=$(wc -l < "$file" | tr -d ' ')
  body=$((total - close))
  if [ "$body" -lt 0 ]; then
    body=0
  fi
  if [ "$body" -ge 500 ]; then
    fail "$file" "body is $body lines (must be under 500)"
  fi

  # --- no README.md inside the skill directory -----------------------------
  readme=$(find "$dir" -iname 'README.md' | head -n 1)
  if [ -n "$readme" ]; then
    fail "$file" "skills must not contain a README.md (found $readme)"
  fi

  # --- nothing nested more than one level deep -----------------------------
  # references/x.md is ok; references/a/x.md is not.
  deep=$(find "$dir" -mindepth 3 | head -n 1)
  if [ -n "$deep" ]; then
    fail "$file" "nothing may nest more than one level deep (found $deep)"
  fi
done <<EOF
$files
EOF

exit "$status"
