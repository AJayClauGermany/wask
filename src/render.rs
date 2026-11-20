//! `--render`: print the markdown as an agent should read it, with imports
//! inlined and dynamic context (`` !`command` `` and ```` ```! ```` blocks)
//! replaced by command output. Those commands run just by reading the file,
//! so each one needs approval, remembered in the global permissions file.

use crate::config::{self, ContextMode, Settings};
use crate::markdown::{Fence, at_links, warn};
use crate::yaml;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process;

type Injection = (PathBuf, String);
type Handler<'a> = dyn FnMut(&Path, &str) -> Result<String, String> + 'a;

/// Walks `file` and its `@` imports, passing every context command to `f` and
/// substituting what it returns.
fn walk(
    file: &Path,
    seen: &mut Vec<PathBuf>,
    f: &mut Handler,
    out: &mut String,
) -> Result<(), String> {
    let file = file
        .canonicalize()
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let src = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    seen.push(file.clone());
    let dir = file.parent().unwrap_or(Path::new("."));
    let (_, body) = yaml::split_frontmatter(&src);
    let mut fence: Option<(Fence, Vec<&str>)> = None;

    for line in body.lines() {
        if let Some((f_open, buf)) = &mut fence {
            let closing = f_open.closes(line);
            if f_open.lang != "!" {
                out.push_str(line);
                out.push('\n');
            } else if closing {
                let output = f(&file, buf.join("\n").trim())?;
                out.push_str(output.trim_end());
                out.push('\n');
            } else {
                buf.push(f_open.strip(line));
            }
            if closing {
                fence = None;
            }
            continue;
        }
        if let Some(opened) = Fence::open(line) {
            if opened.lang != "!" {
                out.push_str(line);
                out.push('\n');
            }
            fence = Some((opened, Vec::new()));
            continue;
        }
        // A line that is only `@other.md` is replaced by that file.
        let text = line.trim();
        let import = at_links(text)
            .next()
            .filter(|link| text == format!("@{link}") && link.to_lowercase().ends_with(".md"))
            .and_then(|link| dir.join(link).canonicalize().ok());
        if let Some(target) = import {
            if !seen.contains(&target) {
                walk(&target, seen, f, out)?;
            }
            continue;
        }
        out.push_str(&expand_inline(line, &mut |cmd| f(&file, cmd))?);
        out.push('\n');
    }
    Ok(())
}

/// Replaces each `` !`command` `` in a line.
fn expand_inline(
    line: &str,
    f: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find("!`") {
        let starts_word = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || "([{:".contains(c));
        let end = rest[at + 2..].find('`');
        match end {
            Some(end) if starts_word && end > 0 => {
                out.push_str(&rest[..at]);
                out.push_str(f(&rest[at + 2..at + 2 + end])?.trim_end());
                rest = &rest[at + 3 + end..];
            }
            _ => {
                out.push_str(&rest[..at + 2]);
                rest = &rest[at + 2..];
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn collect(root: &Path) -> Result<Vec<Injection>, String> {
    let mut found = Vec::new();
    let mut record = |file: &Path, cmd: &str| {
        found.push((file.to_path_buf(), cmd.to_string()));
        Ok(String::new())
    };
    walk(root, &mut Vec::new(), &mut record, &mut String::new())?;
    Ok(found)
}

fn execute(file: &Path, cmd: &str) -> Result<String, String> {
    let dir = file.parent().unwrap_or(Path::new("."));
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/c")
    } else {
        ("sh", "-c")
    };
    let output = process::Command::new(shell)
        .args([flag, cmd])
        .current_dir(dir)
        .env("WASK_DIR", dir)
        .stderr(process::Stdio::inherit())
        .output()
        .map_err(|e| format!("failed to run `{cmd}`: {e}"))?;
    if !output.status.success() {
        warn(&format!(
            "context command `{cmd}` exited with {}",
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// The permissions file: one `path<TAB>command` per line, backslash-escaped.
// Approval is for the exact command text in that exact file.

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

fn entry(inj: &Injection) -> String {
    format!("{}\t{}", escape(&inj.0.to_string_lossy()), escape(&inj.1))
}

fn approved() -> Vec<String> {
    config::permissions_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn approve(injections: &[Injection]) -> Result<(), String> {
    let path = config::permissions_path().ok_or("no config directory to store permissions in")?;
    let err = |e: io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(err)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(err)?;
    for inj in injections {
        writeln!(file, "{}", entry(inj)).map_err(err)?;
    }
    Ok(())
}

fn pending(root: &Path) -> Result<Vec<Injection>, String> {
    let known = approved();
    let mut pending: Vec<Injection> = Vec::new();
    for inj in collect(root)? {
        if !known.contains(&entry(&inj)) && !pending.contains(&inj) {
            pending.push(inj);
        }
    }
    Ok(pending)
}

fn list(injections: &[Injection]) -> String {
    let mut out = String::new();
    for (file, cmd) in injections {
        out.push_str(&format!(
            "    {}: {}\n",
            file.display(),
            cmd.replace('\n', "\n        ")
        ));
    }
    out
}

fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

fn prompt(question: &str) -> String {
    eprint!("{question} ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    let _ = io::stdin().lock().read_line(&mut answer);
    answer.trim().to_lowercase()
}

pub fn render(root: &Path, settings: &Settings) -> Result<String, String> {
    let all = collect(root)?;
    if !all.is_empty() {
        match settings.context {
            ContextMode::Allow => {}
            ContextMode::Deny => {
                return Err(format!(
                    "dynamic context is disabled (`context: deny`), but this file wants to run:\n{}",
                    list(&all)
                ));
            }
            ContextMode::Ask => {
                let pending = pending(root)?;
                if !pending.is_empty() {
                    if !interactive() {
                        return Err(format!(
                            "these context commands have not been approved:\n{}\
                             A person can review and approve them with `wask --trust`.",
                            list(&pending)
                        ));
                    }
                    eprintln!(
                        "Rendering this file runs commands that are not yet approved:\n{}",
                        list(&pending)
                    );
                    match prompt("Run them? [y]es once / [a]lways / [N]o:").as_str() {
                        "y" | "yes" => {}
                        "a" | "always" => approve(&pending)?,
                        _ => return Err("context commands were not approved".into()),
                    }
                }
            }
        }
    }
    let mut out = String::new();
    walk(root, &mut Vec::new(), &mut execute, &mut out)?;
    Ok(out)
}

/// `--trust`: a person reviews and approves the pending context commands.
pub fn trust(root: &Path) -> Result<(), String> {
    let pending = pending(root)?;
    if pending.is_empty() {
        eprintln!("Nothing to approve.");
        return Ok(());
    }
    if !interactive() {
        return Err(format!(
            "`--trust` needs an interactive terminal. Pending context commands:\n{}",
            list(&pending)
        ));
    }
    eprintln!("Context commands awaiting approval:\n{}", list(&pending));
    match prompt("Allow these to run whenever the file is rendered? [y/N]:").as_str() {
        "y" | "yes" => approve(&pending),
        _ => Err("nothing was approved".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_expansion() {
        let mut seen = Vec::new();
        let mut f = |cmd: &str| {
            seen.push(cmd.to_string());
            Ok(format!("<{cmd}>\n"))
        };
        let out = expand_inline(
            "Branch: !`git branch` and (!`date`), not a!`b` or !``",
            &mut f,
        );
        assert_eq!(
            out.unwrap(),
            "Branch: <git branch> and (<date>), not a!`b` or !``"
        );
        assert_eq!(seen, ["git branch", "date"]);
    }

    #[test]
    fn entries_escape() {
        let inj = (PathBuf::from("/a/b.md"), "printf 'a\tb'\nls".to_string());
        assert_eq!(entry(&inj), "/a/b.md\tprintf 'a\\tb'\\nls");
    }
}
