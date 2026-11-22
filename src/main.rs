//! wask: run the commands described in a markdown file.

mod cli;
mod config;
mod introspect;
mod markdown;
mod model;
mod render;
mod run;
mod yaml;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Bad usage, as opposed to a failing script (which keeps its own exit code).
const USAGE_ERROR: u8 = 2;

enum Mode {
    Run,
    Introspect,
    Render,
    Trust,
}

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => ExitCode::from(code),
        Err(msg) => {
            eprintln!("wask: {msg}");
            ExitCode::from(USAGE_ERROR)
        }
    }
}

/// The nearest WASK.md (the opt-in file) or maskfile.md, looking upward from
/// `start` but never past the top of a git repository.
fn discover(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let found = ["WASK.md", "wask.md", "maskfile.md"]
            .iter()
            .map(|name| dir.join(name))
            .find(|p| p.is_file());
        if found.is_some() || dir.join(".git").exists() {
            return found;
        }
    }
    None
}

fn real_main() -> Result<u8, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut file: Option<PathBuf> = None;
    let mut maskfile = false;
    let mut help = false;
    let mut mode = Mode::Run;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (arg.as_str(), None),
        };
        match flag {
            "-f" | "--file" | "--maskfile" => {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i)
                            .ok_or_else(|| format!("`{flag}` needs a path"))?
                    }
                };
                file = Some(PathBuf::from(value));
                maskfile = flag == "--maskfile";
            }
            "-h" | "--help" => help = true,
            "-V" | "--version" => {
                println!("wask {}", env!("CARGO_PKG_VERSION"));
                return Ok(0);
            }
            "--introspect" => mode = Mode::Introspect,
            "--render" => mode = Mode::Render,
            "--trust" => mode = Mode::Trust,
            _ => break,
        }
        i += 1;
    }
    let rest = &args[i..];

    let cwd = env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?;
    let Some(file) = file.or_else(|| discover(&cwd)) else {
        if help {
            print!(
                "{}",
                cli::root_help(Some("wask"), Some(env!("CARGO_PKG_DESCRIPTION")), &[])
            );
            return Ok(0);
        }
        return Err(
            "no WASK.md or maskfile.md here or in a parent directory of this repository.\n\
             Create a WASK.md (it can pull in a skill with a line like `@SKILL.md`) \
             or pass --file <path>."
                .into(),
        );
    };
    let root = file
        .canonicalize()
        .map_err(|e| format!("{}: {e}", file.display()))?;

    let mut settings = config::load_global()?;
    let is_maskfile = root
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("maskfile.md"));
    settings.mask_compat |= maskfile || is_maskfile;

    match mode {
        Mode::Render => {
            print!("{}", render::render(&root, &settings)?);
            return Ok(0);
        }
        Mode::Trust => return render::trust(&root).map(|()| 0),
        Mode::Introspect | Mode::Run => {}
    }

    let doc = markdown::load(&root, &settings, maskfile, &mut Vec::new())?;
    if let Mode::Introspect = mode {
        println!("{}", introspect::document(&doc, &root));
        return Ok(0);
    }
    let title = doc.title.as_deref().or(doc.name.as_deref());
    let root_help = || cli::root_help(title, doc.description.as_deref(), &doc.commands);
    if doc.commands.is_empty() {
        let mut msg = format!(
            "{}: no commands found. Commands are headings with a code block; in files \
             other than WASK.md and maskfile.md they go under a `## Commands` (or \
             `## Tasks`) heading, or are marked with the `prefix:` set in frontmatter.",
            markdown::relative(&root)
        );
        for miss in doc.near_misses.iter().take(5) {
            msg.push_str(&format!(
                "\n\n`{}` at {}:{} is not a command: {}.",
                miss.name,
                markdown::relative(&miss.file),
                miss.line,
                miss.reason
            ));
        }
        return Err(msg);
    }
    if help && rest.is_empty() {
        print!("{}", root_help());
        return Ok(0);
    }

    let print_help = |path: &[&model::Command]| {
        if path.is_empty() {
            print!("{}", root_help());
        } else {
            print!("{}", cli::command_help(path));
        }
    };
    let (path, tokens) = match cli::resolve(&doc.commands, &doc.near_misses, rest)? {
        cli::Resolved::Help(path) => {
            print_help(&path);
            return Ok(0);
        }
        cli::Resolved::Invoke(path, tokens) => (path, tokens),
    };
    let cmd = *path.last().expect("invoke always has a command");
    if help {
        print_help(&path);
        return Ok(0);
    }
    let vars = match cli::parse_invocation(cmd, tokens) {
        Ok(cli::Parsed::Run(vars)) => vars,
        Ok(cli::Parsed::Help) => {
            print_help(&path);
            return Ok(0);
        }
        Err(msg) => {
            let names: Vec<&str> = path.iter().map(|c| c.name.as_str()).collect();
            return Err(format!(
                "{msg}\n\nUSAGE:\n    {}\n\nFor details: wask {} --help",
                cli::usage(&path),
                names.join(" ")
            ));
        }
    };
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("wask"));
    let ctx = run::Context {
        exe: &exe,
        root_file: &root,
    };
    let code = run::run(cmd, &vars, &ctx)?;
    Ok(u8::try_from(code).unwrap_or(1))
}
