---
name: changelog
description: Keeps CHANGELOG.md up to date. Use when a change is ready to merge or a release is being cut.
metadata:
  wask.prefix: "task:"
---

# Changelog

Every change a user would notice gets one line under `## Unreleased` in
CHANGELOG.md. Headings that start with `task:` are commands you can run with
`wask`; everything else is guidance, and @CHANGELOG.md is the file itself.

## Adding an entry

Pick a kind (added, changed, fixed or removed) and write one line in the
imperative mood, the way it should read in the release notes:

```bash example
wask add fixed "Handle an empty config file"
```

### task: add (kind) (text...)

> Add a line under Unreleased

```bash
file="${CHANGELOG_FILE:-CHANGELOG.md}"
case "$kind" in
  added|changed|fixed|removed) ;;
  *) echo "kind must be added, changed, fixed or removed, not \`$kind\`" >&2; exit 2 ;;
esac
grep -q '^## Unreleased' "$file" || { echo "$file has no ## Unreleased section" >&2; exit 1; }
awk -v line="- $kind: $text" '
  skip && /^$/ { next }
  skip && /^#/ { print "" }
  { skip = 0; print }
  /^## Unreleased/ && !done { print ""; print line; done = skip = 1 }
' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
echo "added to $file: $kind: $text"
```

## Cutting a release

Read what is unreleased before choosing a version number: anything under
`removed` or `changed` usually means a major or minor bump.

### task: unreleased

> Print the entries waiting for a release

```sh
awk '/^## Unreleased/ { on = 1; next } /^## / { on = 0 } on && NF' "${CHANGELOG_FILE:-CHANGELOG.md}"
```
