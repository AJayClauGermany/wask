//! Matching command-line tokens against the command tree, and help output.

use crate::model::{Arg, Command, Kind, Opt, env_name};
use std::collections::BTreeMap;
use std::fmt::Write;

pub enum Resolved<'a> {
    /// A command plus the tokens left for its arguments and options.
    Invoke(Vec<&'a Command>, &'a [String]),
    /// Help for a command, or for the root when the path is empty.
    Help(Vec<&'a Command>),
}

pub enum Parsed {
    Run(Vec<(String, String)>),
    Help,
}

/// Walks subcommand names. `help` is accepted at any level.
pub fn resolve<'a>(top: &'a [Command], tokens: &'a [String]) -> Result<Resolved<'a>, String> {
    let mut path: Vec<&Command> = Vec::new();
    let mut subs = top;
    let mut i = 0;
    let mut help = false;
    while let Some(token) = tokens.get(i) {
        if let Some(cmd) = subs.iter().find(|c| c.name == *token) {
            path.push(cmd);
            subs = &cmd.subs;
        } else if token == "help" && !help {
            help = true;
        } else {
            break;
        }
        i += 1;
    }
    let rest = &tokens[i..];
    let stray = rest.first().filter(|t| !t.starts_with('-'));
    let runnable = path.last().is_some_and(|c| c.script.is_some());
    if let Some(stray) = stray.filter(|_| help || !runnable) {
        let names: Vec<&str> = subs.iter().map(|c| c.name.as_str()).collect();
        return Err(format!(
            "unknown command `{stray}` (available: {})",
            names.join(", ")
        ));
    }
    if help || !runnable {
        if let Some(flag) = rest.iter().find(|t| !matches!(t.as_str(), "-h" | "--help")) {
            return Err(format!("unknown option `{flag}`"));
        }
        return Ok(Resolved::Help(path));
    }
    Ok(Resolved::Invoke(path, rest))
}

/// Every command gets `-v/--verbose` unless it defines those itself.
fn effective_options(cmd: &Command) -> Vec<Opt> {
    let mut opts = cmd.options.clone();
    let taken = |o: &Opt| o.name == "verbose" || o.long.as_deref() == Some("verbose");
    if !opts.iter().any(taken) {
        opts.push(Opt {
            name: "verbose".into(),
            short: (!opts.iter().any(|o| o.short == Some('v'))).then_some('v'),
            long: Some("verbose".into()),
            kind: Kind::Bool,
            choices: Vec::new(),
            required: false,
            desc: "Sets $verbose to \"true\"".into(),
        });
    }
    opts
}

fn flag_label(opt: &Opt) -> String {
    match (&opt.long, opt.short) {
        (Some(long), _) => format!("--{long}"),
        (None, Some(short)) => format!("-{short}"),
        (None, None) => opt.name.clone(),
    }
}

/// Turns the remaining tokens into the environment for the script.
pub fn parse_invocation(cmd: &Command, tokens: &[String]) -> Result<Parsed, String> {
    let opts = effective_options(cmd);
    let mut values: BTreeMap<usize, String> = BTreeMap::new();
    let mut positional: Vec<&str> = Vec::new();
    let mut only_positional = false;
    let mut i = 0;

    // Stores a value for a non-boolean option, consuming a token if needed.
    let mut take = |idx: usize, inline: Option<&str>, i: &mut usize| -> Result<(), String> {
        let opt = &opts[idx];
        let value = match (opt.kind, inline) {
            (Kind::Bool, None) => "true".to_string(),
            (Kind::Bool, Some(_)) => {
                return Err(format!("option `{}` takes no value", flag_label(opt)));
            }
            (_, Some(v)) => v.to_string(),
            (_, None) => {
                let v = tokens
                    .get(*i)
                    .ok_or_else(|| format!("option `{}` needs a value", flag_label(opt)))?;
                *i += 1;
                v.clone()
            }
        };
        values.insert(idx, value);
        Ok(())
    };

    while i < tokens.len() {
        let token = tokens[i].as_str();
        i += 1;
        if only_positional {
            positional.push(token);
        } else if token == "--" {
            only_positional = true;
        } else if token == "-h" || token == "--help" {
            return Ok(Parsed::Help);
        } else if let Some(body) = token.strip_prefix("--") {
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (body, None),
            };
            let idx = opts
                .iter()
                .position(|o| o.long.as_deref() == Some(name))
                .ok_or_else(|| format!("unknown option `--{name}`"))?;
            take(idx, inline, &mut i)?;
        } else if token.len() > 1 && token.starts_with('-') {
            // `-p 80`, `-p80`, `-p=80`, or grouped booleans like `-vq`.
            let body = &token[1..];
            for (at, c) in body.char_indices() {
                let Some(idx) = opts.iter().position(|o| o.short == Some(c)) else {
                    if at == 0 && token.parse::<f64>().is_ok() {
                        positional.push(token); // a negative number
                        break;
                    }
                    return Err(format!("unknown option `-{c}`"));
                };
                if opts[idx].kind == Kind::Bool {
                    take(idx, None, &mut i)?;
                    continue;
                }
                let rest = &body[at + c.len_utf8()..];
                let inline = (!rest.is_empty()).then_some(rest.strip_prefix('=').unwrap_or(rest));
                take(idx, inline, &mut i)?;
                break;
            }
        } else {
            positional.push(token);
        }
    }

    let mut vars = Vec::new();
    let mut positional = positional.into_iter();
    for arg in &cmd.args {
        let value = if arg.variadic {
            let rest: Vec<&str> = positional.by_ref().collect();
            (!rest.is_empty()).then(|| rest.join(" "))
        } else {
            positional.next().map(str::to_string)
        };
        match value {
            Some(value) => vars.push((env_name(&arg.name), value)),
            None if arg.required => {
                return Err(format!("missing required argument <{}>", arg.name));
            }
            None => {}
        }
    }
    if let Some(extra) = positional.next() {
        return Err(format!("unexpected argument `{extra}`"));
    }
    for (idx, opt) in opts.iter().enumerate() {
        let Some(value) = values.remove(&idx) else {
            if opt.required {
                return Err(format!("missing required option `{}`", flag_label(opt)));
            }
            continue;
        };
        if opt.kind == Kind::Number && value.parse::<f64>().is_err() {
            return Err(format!(
                "option `{}` expects a number, got `{value}`",
                flag_label(opt)
            ));
        }
        if !opt.choices.is_empty() && !opt.choices.contains(&value) {
            return Err(format!(
                "option `{}` must be one of: {} (got `{value}`)",
                flag_label(opt),
                opt.choices.join(", ")
            ));
        }
        vars.push((env_name(&opt.name), value));
    }
    Ok(Parsed::Run(vars))
}

fn arg_label(arg: &Arg) -> String {
    let dots = if arg.variadic { "..." } else { "" };
    if arg.required {
        format!("<{}{dots}>", arg.name)
    } else {
        format!("[{}{dots}]", arg.name)
    }
}

pub fn usage(path: &[&Command]) -> String {
    let mut out = String::from("wask");
    for cmd in path {
        write!(out, " {}", cmd.name).unwrap();
    }
    let Some(cmd) = path.last() else {
        return out + " [OPTIONS] <COMMAND> [ARGS]";
    };
    if cmd.script.is_some() {
        for arg in &cmd.args {
            write!(out, " {}", arg_label(arg)).unwrap();
        }
        out.push_str(" [OPTIONS]");
    } else {
        out.push_str(" <COMMAND>");
    }
    out
}

fn table(out: &mut String, heading: &str, rows: &[(String, String)]) {
    if rows.is_empty() {
        return;
    }
    let width = rows
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    write!(out, "\n{heading}:\n").unwrap();
    for (left, right) in rows {
        let line = format!("    {left:width$}  {right}");
        out.push_str(line.trim_end());
        out.push('\n');
    }
}

fn command_rows(cmds: &[Command]) -> Vec<(String, String)> {
    cmds.iter()
        .map(|c| {
            let mut left = c.name.clone();
            if c.script.is_some() {
                for arg in &c.args {
                    write!(left, " {}", arg_label(arg)).unwrap();
                }
            }
            if !c.subs.is_empty() {
                left.push_str(" ...");
            }
            (left, c.desc.clone())
        })
        .collect()
}

pub const GLOBAL_OPTIONS: &[(&str, &str)] = &[
    (
        "-f, --file <path>",
        "Use this file instead of searching for WASK.md / maskfile.md",
    ),
    (
        "    --maskfile <path>",
        "Same, and treat the file as a maskfile",
    ),
    ("    --introspect", "Print the command tree as JSON"),
    (
        "    --render",
        "Print the markdown with `!` context commands expanded",
    ),
    (
        "    --trust",
        "Review and approve the file's `!` context commands",
    ),
    ("-h, --help", "Print help (also: wask help <command>)"),
    ("-V, --version", "Print version"),
];

pub fn root_help(title: Option<&str>, description: Option<&str>, cmds: &[Command]) -> String {
    let mut out = String::new();
    if let Some(title) = title {
        writeln!(out, "{title}").unwrap();
    }
    if let Some(description) = description {
        writeln!(out, "{description}").unwrap();
    }
    if !out.is_empty() {
        out.push('\n');
    }
    writeln!(out, "USAGE:\n    {}", usage(&[])).unwrap();
    table(&mut out, "COMMANDS", &command_rows(cmds));
    let globals: Vec<(String, String)> = GLOBAL_OPTIONS
        .iter()
        .map(|(l, r)| (l.to_string(), r.to_string()))
        .collect();
    table(&mut out, "OPTIONS", &globals);
    out
}

pub fn command_help(path: &[&Command]) -> String {
    let cmd = path.last().expect("command help needs a command");
    let mut out = String::new();
    if !cmd.desc.is_empty() {
        writeln!(out, "{}\n", cmd.desc).unwrap();
    }
    writeln!(out, "USAGE:\n    {}", usage(path)).unwrap();
    if cmd.script.is_some() {
        let args: Vec<(String, String)> = cmd
            .args
            .iter()
            .map(|a| {
                let note = if a.required { "required" } else { "optional" };
                (arg_label(a), format!("{note}, sets ${}", env_name(&a.name)))
            })
            .collect();
        table(&mut out, "ARGS", &args);
        let mut opts: Vec<(String, String)> = effective_options(cmd)
            .iter()
            .map(|o| {
                let short = o.short.map_or("    ".to_string(), |s| {
                    format!("-{s}{}", if o.long.is_some() { ", " } else { "  " })
                });
                let long = o.long.as_ref().map_or(String::new(), |l| format!("--{l}"));
                let value = match o.kind {
                    Kind::Bool => "",
                    Kind::String => " <string>",
                    Kind::Number => " <number>",
                };
                let mut desc = o.desc.clone();
                if !o.choices.is_empty() {
                    write!(desc, " [choices: {}]", o.choices.join(", ")).unwrap();
                }
                if o.required {
                    desc.push_str(" (required)");
                }
                (format!("{short}{long}{value}"), desc.trim().to_string())
            })
            .collect();
        opts.push(("-h, --help".into(), "Print help".into()));
        table(&mut out, "OPTIONS", &opts);
    }
    table(&mut out, "COMMANDS", &command_rows(&cmd.subs));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::model::Script;
    use std::rc::Rc;

    fn cmd() -> Command {
        let opt = |name: &str, short, kind, required| Opt {
            name: name.into(),
            short,
            long: Some(name.into()),
            kind,
            choices: Vec::new(),
            required,
            desc: String::new(),
        };
        let mut color = opt("color", Some('c'), Kind::String, false);
        color.choices = vec!["red".into(), "blue".into()];
        Command {
            name: "serve".into(),
            desc: String::new(),
            args: vec![
                Arg {
                    name: "dir".into(),
                    required: true,
                    variadic: false,
                },
                Arg {
                    name: "more".into(),
                    required: false,
                    variadic: true,
                },
            ],
            options: vec![
                opt("port", Some('p'), Kind::Number, true),
                opt("dry-run", Some('n'), Kind::Bool, false),
                color,
            ],
            script: Some(Script::Inline {
                lang: "sh".into(),
                source: String::new(),
            }),
            subs: Vec::new(),
            dir: ".".into(),
            settings: Rc::new(Settings::default()),
        }
    }

    fn run(tokens: &[&str]) -> Result<Vec<(String, String)>, String> {
        let tokens: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
        match parse_invocation(&cmd(), &tokens)? {
            Parsed::Run(vars) => Ok(vars),
            Parsed::Help => Err("help".into()),
        }
    }

    #[test]
    fn parses_args_and_options() {
        let vars = run(&["site", "-nv", "-p", "80", "a", "--color=red", "--", "-b"]).unwrap();
        let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("dir"), Some("site"));
        assert_eq!(get("more"), Some("a -b"));
        assert_eq!(get("port"), Some("80"));
        assert_eq!(get("dry_run"), Some("true"));
        assert_eq!(get("verbose"), Some("true"));
        assert_eq!(get("color"), Some("red"));
        assert_eq!(run(&["site", "-p8080"]).unwrap()[1].1, "8080");
        assert_eq!(run(&["-5", "-p", "1"]).unwrap()[0].1, "-5");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(run(&["-p", "80"]).unwrap_err().contains("<dir>"));
        assert!(run(&["site"]).unwrap_err().contains("--port"));
        assert!(run(&["site", "-p", "x"]).unwrap_err().contains("number"));
        assert!(
            run(&["site", "-p", "1", "-c", "green"])
                .unwrap_err()
                .contains("one of")
        );
        assert!(
            run(&["site", "-p", "1", "--nope"])
                .unwrap_err()
                .contains("--nope")
        );
        assert!(run(&["site", "-p"]).unwrap_err().contains("needs a value"));
        assert_eq!(run(&["site", "--help"]).unwrap_err(), "help");
    }
}
