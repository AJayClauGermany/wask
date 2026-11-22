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
| ```` ```bash example ```` | An example: shown to readers, never run (see below) |
| `@scripts/run.py` under a heading with no code block | The script is that file |
| `@other.md` | Import another file's commands (see below) |

Headings without a script are documentation. The next section covers how to
keep documentation that *does* have code blocks from turning into commands.

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
`php`, `lua`, `swift`, `powershell`, `cmd`. A command can offer a `powershell`
or `cmd` block next to its `bash` one; the block for the current platform runs.

## Commands or documentation?

Markdown that people read is full of headings and code blocks that are not
meant to run. wask decides what is a command from the file itself, so a file
reads the same for every person, agent and machine.

**Which headings.** By default this depends on the file:

| File | Commands are |
| --- | --- |
| `WASK.md`, `maskfile.md` | Every heading with a code block (mask's rule) |
| Anything else: a `SKILL.md`, an import, `--file README.md` | Headings under a `## Commands` or `## Tasks` heading (the second is [xc]'s convention) |

Frontmatter can change that with one of two settings:

```yaml
wask:
  section: Recipes   # only headings under "Recipes"; "*" for the whole file
  prefix: "task:"    # only headings that start with "task:", anywhere in the file
```

A prefix lets commands sit next to the prose that explains them:

````markdown
## Deploying

Deploys go to staging first. To try it locally, run `wask deploy dev`:

```bash
wask deploy dev
```

### task: deploy (env)

> Deploy to an environment

```bash
./scripts/deploy.sh "$env"
```
````

`Deploying` is documentation, even though it has a code block. `deploy` is
the command: `task:` is stripped, and the rest is an ordinary command heading.
A heading marked with the prefix that cannot be a command, such as
`### task: Why this?` or one with no runnable block, is reported as a warning
with its line number, because you said you meant it. Unmarked headings are
transparent: marked headings under them are still commands, and to group
commands, mark the group's heading too (`## task: db` then `### task: migrate`).
`` prefix: "`" `` makes code-formatted headings (`` ### `deploy (env)` ``) the
commands.

In a skill that uses a prefix, an `@file.md` mention in the prose is a
reference for the reader, not an import. Only marked headings import.

**Which code block.** Under a command heading, a block whose info string says
`example` is never the script. rustdoc's `ignore` and `no_run`, and Runme's
`{ ignore=true }`, mean the same. Blocks in languages wask has no runner for
(`console`, `text`, `json` and so on) are never scripts either.

````markdown
### task: release (version)

```bash example
wask release 1.4.0
```

```bash
git tag "v$version" && git push --tags
```
````

If more than one block could be the script, wask will not guess. The command
still shows up in help, but running it fails with exit code 2 and says which
blocks to mark. `maskfile.md` keeps mask's behaviour, where the last block runs.

**Finding out why.** Asking for something that is not a command explains
itself:

```text
$ wask lint
wask: unknown command `lint` (available: build, test)

The heading `lint` at WASK.md:7 is not a command: its code block is `shell`, and
wask has no runner for that: use a language such as `sh`, `bash` or `python`, or
add one under `runners:`.
```

A typo gets a suggestion instead: ``did you mean `build`?``.

`section` and `prefix` describe how a file is written, so they are only read
from that file's own frontmatter. Imports do not inherit them, and the global
config cannot set them.

[xc]: https://xcfile.dev

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

A `SKILL.md` is mostly prose with example snippets, so only the headings under
its `## Commands` (or `## Tasks`) heading are commands. To keep each command
next to its documentation instead, mark commands with a `prefix:` (see
[Commands or documentation?](#commands-or-documentation)).
[examples/marked](examples/marked) is a skill written that way.

Files next to the skill are reached through `$WASK_DIR`, by linking a script
with `@scripts/check.py`, or by setting `dir: file` so scripts run in the
skill's folder. [examples/skill](examples/skill) shows all three.

### Frontmatter

`name` and `description` are used for help. Settings go under `wask:` (or, to
stay strictly within the skills spec, as `metadata: { wask.prefix: "task:" }`).

```yaml
---
name: release-notes
description: Drafts release notes from git history.
wask:
  dir: file                  # cwd (default) or file: where scripts run
  section: Commands          # which heading holds the commands; "*" for all
  prefix: "task:"            # or: only headings starting with this are commands
  mask-compat: true          # also export $MASK and $MASKFILE_DIR
  env:
    NOTES_FORMAT: markdown
  runners:
    py: uv run python -c {script}   # {script} = source, {file} = temp file
    ts: deno run                    # neither: a temp file is appended
---
```

An imported file inherits the settings of the file that imports it (except
`section` and `prefix`) and can override them for its own commands.

## Dynamic context

Skills can embed command output with `` !`command` `` (at the start of a line
or after a space, as in Claude Code) or a fenced block whose language is `!`. `wask --render` prints the document with frontmatter removed,
`@file.md` lines inlined and those commands replaced by their output.

Because that runs commands just by reading a file, each one needs approval.
Approval is stored per file and exact command text, so editing a command asks
again. In a terminal `--render` prompts; `wask --trust` reviews and approves
ahead of time. Without a terminal (an agent, CI) unapproved commands are an
error and nothing runs.

## Global config

Optional, at `$WASK_CONFIG`, else `$XDG_CONFIG_HOME/wask/config.yaml`, else
`~/.config/wask/config.yaml`. Same keys as the `wask:` frontmatter block (minus
`section` and `prefix`), plus `context`, which only the global config can set:

```yaml
context: ask        # ask (default), allow, or deny dynamic context commands
mask-compat: true
runners:
  py: python3.12 -c {script}
```

Approved context commands are kept in a `permissions` file beside it.

## For agents

`wask --introspect` prints the command tree as JSON. For each command it has
the name, description, a ready-to-run `usage` line, the `file` and `line` of its
heading, its arguments and options with the environment variable each one
sets, the script, and `error`: why it cannot run (an ambiguous script), or
`null`.

Errors go to stderr. Bad usage, including an unknown or ambiguous command,
exits with 2. Otherwise the exit code is the script's own. Nothing prompts
unless stdin and stderr are a terminal.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Or `wask check`, using this repository's own [WASK.md](WASK.md).
