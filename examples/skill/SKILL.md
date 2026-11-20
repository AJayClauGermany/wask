---
name: release-notes
description: Drafts release notes from git history. Use when preparing a release or a changelog entry.
wask:
  dir: file            # run scripts next to this file, so relative paths work
  env:
    NOTES_FORMAT: markdown
---

# Release notes

Current branch: !`git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown`

Read @reference.md for the house style before writing anything.

## Workflow

1. Run `wask changes` to see what happened since the last release.
2. Draft the notes, then check them with `wask check notes.md`.

```bash
# An example, not a command: only headings under "Commands" are runnable.
wask changes v1.2.0 --limit 5
```

## Commands

### changes [since]

> List commits since a tag (default: the latest tag)

**OPTIONS**
* limit
    * flags: -n --limit
    * type: number
    * desc: Show at most this many commits

```bash
since="${since:-$(git describe --tags --abbrev=0 2>/dev/null)}"
git log --oneline ${limit:+-n "$limit"} ${since:+"$since"..HEAD}
```

### check (file)

> Check a draft against the house style

@scripts/check.py

### style

> Print the house style

```sh
cat "$WASK_DIR/reference.md"
```
