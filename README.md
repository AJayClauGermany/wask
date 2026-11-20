# wask

A task runner whose tasks live in markdown that people and agents can both read.
It is an adaptation of [mask](https://github.com/jacobdeichert/mask) that also
understands agent skills (`SKILL.md`): frontmatter, files that sit next to the
skill, and dynamic context.

No dependencies beyond the Rust standard library.

```bash
cargo install --path .
```

## Quick start

`wask` only acts on a folder that opts in with a `WASK.md` (or already has a
`maskfile.md`). It looks in the current directory, then upward to the top of the
git repository.

````markdown
# My project

## build (target) [profile]

> Build one target

**OPTIONS**
* jobs
    * flags: -j --jobs
    * type: number
    * desc: Parallel jobs

```bash
echo "building $target (${profile:-debug}) with ${jobs:-1} jobs"
```
````

```bash
wask                     # list commands
wask build app -j 4      # run one
wask help build          # or: wask build --help
```

## The format

This is the maskfile format, so an existing `maskfile.md` runs unchanged.

| Markdown | Meaning |
| --- | --- |
| `## name` with a code block | A command. Deeper headings are subcommands; repeating the parent (`### services start`) is optional. |
| `(arg)`, `[arg]`, `[rest...]` in the heading | Required, optional and catch-all arguments |
| First `> quote` (or paragraph) | The description shown in help |
| `**OPTIONS**` followed by a list | Flags: `flags`, `type` (`string`, `number`, omitted = boolean), `choices`, `required`, `desc` |
| Fenced code block | The script. The language picks the runner. |
| `@scripts/run.py` under a heading with no code block | The script is that file |
| `@other.md` | Import another file's commands (see below) |

Headings without a script are documentation and are ignored.

Scripts receive arguments and flags as environment variables, with dashes turned
into underscores (`--dry-run` is `$dry_run`). Boolean flags are `"true"` or
unset. Every command also accepts `-v/--verbose` (`$verbose`). The script's exit
code becomes the exit code of `wask`; bad usage exits with 2.

| Variable | Value |
| --- | --- |
| `$WASK` | Calls back into the same file: `$WASK other-command` |
| `$WASK_DIR` | Folder of the markdown file that defines the command, where its extra files live |
| `$WASK_ROOT` | Folder of the file `wask` started from |
| `$MASK`, `$MASKFILE_DIR` | Set for `maskfile.md`, `--maskfile`, or `mask-compat: true` |

Built-in runners: `sh`, `bash`, `zsh`, `fish`, `dash`, `ksh`, `js`, `py`, `rb`,
`php`, `lua`, `swift`, `powershell`, `cmd`. When a command has several code
blocks, the first one that suits the platform runs.

## Skills

Link a skill from `WASK.md` the way `CLAUDE.md` links files:

```markdown
# Project tasks

@skills/release-notes/SKILL.md
```

A line like that at the top of the file adds the skill's commands at the top
level. Under a heading (`## notes` then `@skills/.../SKILL.md`) they become
subcommands of it. You can also run a skill directly with
`wask --file path/to/SKILL.md`.

A `SKILL.md` is mostly prose with example snippets, so in any file not named
`WASK.md` or `maskfile.md` only the headings under `## Commands` are commands.
Change the section name with `section:` in frontmatter, or use `section: "*"`
to read the whole file.

Files next to the skill are reached through `$WASK_DIR`, by linking a script
with `@scripts/check.py`, or by setting `dir: file` so scripts run in the
skill's folder. [examples/skill](examples/skill) shows all three.

### Frontmatter

`name` and `description` are used for help. Settings go under `wask:` (or, to
stay strictly within the skills spec, as `metadata: { wask.dir: file }`).

```yaml
---
name: release-notes
description: Drafts release notes from git history.
wask:
  dir: file                  # cwd (default) or file: where scripts run
  section: Commands          # which heading holds the commands; "*" for all
  mask-compat: true          # also export $MASK and $MASKFILE_DIR
  env:
    NOTES_FORMAT: markdown
  runners:
    py: uv run python -c {script}   # {script} = source, {file} = temp file
    ts: deno run                    # neither: a temp file is appended
---
```

An imported file inherits the settings of the file that imports it and can
override them for its own commands.

## Dynamic context

Skills can embed command output with `` !`command` `` or a fenced block whose
language is `!`. `wask --render` prints the document with frontmatter removed,
`@file.md` lines inlined and those commands replaced by their output.

Because that runs commands just by reading a file, each one needs approval.
Approval is stored per file and exact command text, so editing a command asks
again. In a terminal `--render` prompts; `wask --trust` reviews and approves
ahead of time. Without a terminal (an agent, CI) unapproved commands are an
error and nothing runs.

## Global config

Optional, at `$WASK_CONFIG`, else `$XDG_CONFIG_HOME/wask/config.yaml`, else
`~/.config/wask/config.yaml`. Same keys as the `wask:` frontmatter block (minus
`section`), plus `context`, which only the global config can set:

```yaml
context: ask        # ask (default), allow, or deny dynamic context commands
mask-compat: true
runners:
  py: python3.12 -c {script}
```

Approved context commands are kept in a `permissions` file beside it.

## For agents

`wask --introspect` prints the command tree as JSON: names, descriptions,
arguments, options, the environment variable each one sets, and the script.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Or `wask check`, using this repository's own [WASK.md](WASK.md).
