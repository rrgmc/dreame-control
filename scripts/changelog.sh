#!/usr/bin/env bash
# Maintains CHANGELOG.md from the fragments in changelog/unreleased/.
#
#   scripts/changelog.sh release <version> [<date>]
#       Merges every fragment into a new "## [<version>] - <date>" section at the top of
#       CHANGELOG.md and deletes the fragments. The date defaults to today (UTC).
#   scripts/changelog.sh notes <version>
#       Prints the body of the "## [<version>]" section, for the GitHub release notes.
#       Exits with status 1 when CHANGELOG.md has no such section.
#   scripts/changelog.sh check
#       Fails when a fragment uses a heading other than the six Keep a Changelog sections.
#
# A fragment is a Markdown file of "### <Section>" headings, each followed by list items. See
# changelog/README.md.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
changelog="$root/CHANGELOG.md"
fragments="$root/changelog/unreleased"
sections=(Added Changed Deprecated Removed Fixed Security)

die() {
  echo "changelog: $*" >&2
  exit 1
}

fragment_files() {
  find "$fragments" -maxdepth 1 -type f -name '*.md' | LC_ALL=C sort
}

# Prints the items under "### <section>" in one fragment, without the heading and the blank lines
# around them.
section_items() {
  awk -v want="$2" '
    /^### / { name = $0; sub(/^### +/, "", name); sub(/[ \t]+$/, "", name); inside = (name == want); next }
    /^#/ { inside = 0; next }
    inside { print }
  ' "$1" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}'
}

check() {
  local file status=0
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    # Every heading must be "### " and one of the six sections.
    while IFS= read -r heading; do
      local known=0
      for s in "${sections[@]}"; do [ "$heading" = "### $s" ] && known=1; done
      if [ "$known" = 0 ]; then
        echo "changelog: ${file#"$root/"}: \"$heading\" is not a section heading; use \"### <Section>\" with one of: ${sections[*]}" >&2
        status=1
      fi
    done < <(grep -E '^#' "$file" | sed 's/[[:space:]]*$//' || true)
    if ! grep -q -E '^### ' "$file"; then
      echo "changelog: ${file#"$root/"}: no \"### <Section>\" heading" >&2
      status=1
    fi
  done < <(fragment_files)
  return "$status"
}

release() {
  local version="${1:-}" date="${2:-$(date -u +%Y-%m-%d)}"
  [ -n "$version" ] || die "usage: changelog.sh release <version> [<date>]"
  version="${version#v}"
  [ -f "$changelog" ] || die "CHANGELOG.md is missing"
  if grep -q -E "^## \[$version\]" "$changelog"; then
    die "CHANGELOG.md already has a section for $version"
  fi
  check

  local files body="" file items
  files="$(fragment_files)"
  [ -n "$files" ] || die "changelog/unreleased/ has no fragments"

  for s in "${sections[@]}"; do
    local collected=""
    while IFS= read -r file; do
      items="$(section_items "$file" "$s")"
      [ -n "$items" ] && collected+="$items"$'\n'
    done <<< "$files"
    [ -n "$collected" ] && body+=$'\n'"### $s"$'\n\n'"$collected"
  done

  local section="## [$version] - $date"$'\n'"$body"
  local tmp
  tmp="$(mktemp)"
  # The new section goes before the first "## [" heading, or at the end of a changelog that has
  # none yet.
  awk -v section="$section" '
    !done && /^## \[/ { print section; done = 1 }
    { print }
    END { if (!done) { print ""; printf "%s", section } }
  ' "$changelog" > "$tmp"
  mv "$tmp" "$changelog"

  while IFS= read -r file; do rm -- "$file"; done <<< "$files"
  echo "Added the $version section to CHANGELOG.md from $(wc -l <<< "$files" | tr -d ' ') fragment(s)."
}

notes() {
  local version="${1:-}"
  [ -n "$version" ] || die "usage: changelog.sh notes <version>"
  version="${version#v}"
  local out
  out="$(awk -v want="$version" '
    /^## \[/ {
      if (inside) exit
      v = $0; sub(/^## \[/, "", v); sub(/\].*/, "", v)
      inside = (v == want); found = found || inside; next
    }
    inside { print }
    END { if (!found) exit 1 }
  ' "$changelog")" || return 1
  printf '%s\n' "$out" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}'
}

case "${1:-}" in
  release) shift; release "$@" ;;
  notes) shift; notes "$@" ;;
  check) check ;;
  *) die "usage: changelog.sh release <version> [<date>] | notes <version> | check" ;;
esac
