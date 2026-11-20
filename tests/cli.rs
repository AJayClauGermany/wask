//! End-to-end tests: real files, the real binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Sandbox(PathBuf);

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Sandbox(dir.canonicalize().unwrap())
    }

    fn write(&self, path: &str, contents: &str) -> &Sandbox {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        self
    }

    fn run_in(&self, dir: &str, args: &[&str]) -> Output {
        let out = Command::new(env!("CARGO_BIN_EXE_wask"))
            .args(args)
            .current_dir(self.0.join(dir))
            .env("WASK_CONFIG", self.0.join("config/config.yaml"))
            .output()
            .unwrap();
        Output {
            code: out.status.code().unwrap(),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_in(".", args)
    }
}

const MASKFILE: &str = r#"# Tasks

## build

> Builds the project

~~~sh
echo "building in $(basename "$MASKFILE_DIR")"
~~~

## test (file) [test_case]

~~~bash
echo "file=$file case=${test_case:-none} verbose=${verbose:-false}"
~~~

## serve

**OPTIONS**
* port
    * flags: -p --port
    * type: number
    * required
* dry-run
    * flags: --dry-run

~~~bash
echo "port=$port dry=${dry_run:-false}"
~~~

## services

> Manage services

### services start (name)

~~~bash
echo "start $name"
~~~

### stop (name)

~~~powershell
Write-Output "never on unix"
~~~

~~~sh
echo "stop $name"; exit 7
~~~

## chain

~~~sh
$MASK services start "from-chain"
~~~

## Notes

Just prose, with no code.
"#;

#[test]
fn runs_an_existing_maskfile() {
    let sb = Sandbox::new("maskfile");
    sb.write("maskfile.md", MASKFILE).write("sub/.keep", "");

    let help = sb.run(&[]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("build") && help.stdout.contains("Builds the project"));
    assert!(!help.stdout.contains("Notes"));

    assert!(
        sb.run(&["build"])
            .stdout
            .starts_with("building in maskfile")
    );
    // Found from a subdirectory too.
    assert_eq!(sb.run_in("sub", &["build"]).code, 0);
    assert_eq!(
        sb.run(&["test", "a.rs", "-v"]).stdout,
        "file=a.rs case=none verbose=true\n"
    );
    assert_eq!(
        sb.run(&["serve", "--dry-run", "-p", "80"]).stdout,
        "port=80 dry=true\n"
    );
    assert_eq!(sb.run(&["services", "start", "db"]).stdout, "start db\n");
    assert_eq!(sb.run(&["chain"]).stdout, "start from-chain\n");

    let stop = sb.run(&["services", "stop", "db"]);
    assert_eq!((stop.code, stop.stdout.as_str()), (7, "stop db\n"));

    let bad = sb.run(&["serve", "-p", "eighty"]);
    assert_eq!(bad.code, 2);
    assert!(bad.stderr.contains("expects a number"));
    assert_eq!(sb.run(&["test"]).code, 2);
    assert_eq!(sb.run(&["nope"]).code, 2);

    let sub_help = sb.run(&["help", "services", "start"]);
    assert!(sub_help.stdout.contains("wask services start <name>"));
    assert_eq!(
        sb.run(&["services", "start", "--help"]).stdout,
        sub_help.stdout
    );
}

const SKILL: &str = r#"---
name: greeter
description: >
  Greets people. Use when
  someone needs a greeting.
wask:
  dir: file
  env:
    GREETING: Hello
  runners:
    shout: sh {file}
---

# Greeter

See @notes.txt for background.

## Quick start

```sh
echo "an example, not a command"
```

## Commands

### greet (name) [rest...]

> Say hello

```sh
echo "$GREETING $name ${rest:-} from $(basename "$PWD")"
```

### linked

@scripts/linked.sh

### loud

```shout
echo "LOUD from $(basename "$WASK_DIR") root $(basename "$WASK_ROOT")"
```

### nested

@more/extra.md
"#;

fn skill_sandbox(name: &str) -> Sandbox {
    let sb = Sandbox::new(name);
    sb.write("skills/greeter/SKILL.md", SKILL)
        .write("skills/greeter/notes.txt", "notes")
        .write(
            "skills/greeter/scripts/linked.sh",
            "echo \"linked $(cat \"$WASK_DIR/notes.txt\")\"\n",
        )
        .write(
            "skills/greeter/more/extra.md",
            "## Commands\n\n### deep\n\n```sh\necho deep\n```\n",
        )
        .write(
            "WASK.md",
            "# Project\n\n@skills/greeter/SKILL.md\n\n## local\n\n```sh\necho local\n```\n",
        );
    sb
}

#[test]
fn wask_file_links_a_skill() {
    let sb = skill_sandbox("skill");

    let help = sb.run(&["--help"]);
    for name in [
        "local",
        "greet <name> [rest...]",
        "linked",
        "loud",
        "nested ...",
    ] {
        assert!(
            help.stdout.contains(name),
            "missing {name}: {}",
            help.stdout
        );
    }
    assert!(!help.stdout.contains("Quick"));

    assert_eq!(sb.run(&["local"]).stdout, "local\n");
    // Frontmatter env, variadic args, and `dir: file`.
    assert_eq!(
        sb.run(&["greet", "Ada", "and", "Bob"]).stdout,
        "Hello Ada and Bob from greeter\n"
    );
    assert_eq!(sb.run(&["linked"]).stdout, "linked notes\n");
    assert_eq!(sb.run(&["loud"]).stdout, "LOUD from greeter root skill\n");
    assert_eq!(sb.run(&["nested", "deep"]).stdout, "deep\n");
}

#[test]
fn skill_file_runs_directly_and_introspects() {
    let sb = skill_sandbox("direct");
    let out = sb.run(&["--file", "skills/greeter/SKILL.md", "greet", "Ada"]);
    assert_eq!(out.stdout, "Hello Ada  from greeter\n");

    let json = sb
        .run(&["--file", "skills/greeter/SKILL.md", "--introspect"])
        .stdout;
    assert!(json.contains(r#""name":"greeter""#));
    assert!(json.contains(r#""description":"Greets people. Use when someone needs a greeting.""#));
    assert!(json.contains(r#"{"name":"rest","env":"rest","required":false,"variadic":true}"#));
}

#[test]
fn nothing_runs_without_opting_in() {
    let sb = Sandbox::new("optin");
    // `.git` marks the repository top, where the upward search stops.
    sb.write("SKILL.md", SKILL).write(".git/HEAD", "");
    let out = sb.run(&["greet", "Ada"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("no WASK.md or maskfile.md"));
}

#[test]
fn global_config_changes_runners() {
    let sb = Sandbox::new("config");
    sb.write(
        "WASK.md",
        "## hi\n\n```py\nignored by the fake runner\n```\n",
    )
    .write(
        "config/config.yaml",
        "runners:\n  python: echo configured\n",
    );
    let out = sb.run(&["hi"]);
    assert!(out.stdout.starts_with("configured "), "{}", out.stdout);
}

#[test]
fn context_injection_needs_approval() {
    let sb = Sandbox::new("context");
    sb.write(
        "WASK.md",
        "---\nwask:\n  context: allow\n---\n# Doc\n\nNow: !`echo injected`\n\n```!\necho block\n```\n\n\
         ```sh\necho !`untouched`\n```\n\n@part.md\n",
    )
    .write("part.md", "Part: !`echo imported`\n");

    // Not approved, and the file cannot approve itself via frontmatter.
    let blocked = sb.run(&["--render"]);
    assert_eq!(blocked.code, 2);
    assert_eq!(blocked.stdout, "");
    assert!(blocked.stderr.contains("echo injected") && blocked.stderr.contains("echo imported"));
    assert_eq!(sb.run(&["--trust"]).code, 2); // no terminal here

    // Approval is per file and exact command text.
    let entry = |file: &str, cmd: &str| format!("{}\t{cmd}\n", sb.0.join(file).display());
    let mut approved = entry("WASK.md", "echo injected") + &entry("WASK.md", "echo block");
    sb.write("config/permissions", &approved);
    assert_eq!(sb.run(&["--render"]).code, 2);
    approved += &entry("part.md", "echo imported");
    sb.write("config/permissions", &approved);

    let ok = sb.run(&["--render"]);
    assert_eq!(ok.code, 0, "{}", ok.stderr);
    assert_eq!(
        ok.stdout,
        "# Doc\n\nNow: injected\n\nblock\n\n```sh\necho !`untouched`\n```\n\nPart: imported\n"
    );

    // The global config can switch the gate off, or shut it.
    fs::remove_file(sb.0.join("config/permissions")).unwrap();
    sb.write("config/config.yaml", "context: allow\n");
    assert_eq!(sb.run(&["--render"]).code, 0);
    sb.write("config/config.yaml", "context: deny\n");
    assert_eq!(sb.run(&["--render"]).code, 2);
}
