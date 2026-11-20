//! Running a command's script with its arguments in the environment.

use crate::config::{DirMode, canonical_lang};
use crate::model::{Command, Script};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

pub struct Context<'a> {
    pub exe: &'a Path,
    pub root_file: &'a Path,
}

/// Runs the command and returns its exit code.
pub fn run(cmd: &Command, vars: &[(String, String)], ctx: &Context) -> Result<i32, String> {
    let script = cmd.script.as_ref().ok_or("command has no script")?;
    let mut temp: Option<PathBuf> = None;
    let argv = match script {
        Script::Inline { lang, source } => {
            let template = cmd
                .settings
                .runner(lang)
                .ok_or_else(|| format!("no runner for `{lang}` code blocks"))?;
            inline_argv(&template, source, &canonical_lang(lang), &mut temp)?
        }
        Script::File(path) => file_argv(cmd, path)?,
    };

    let mut child = process::Command::new(&argv[0]);
    child.args(&argv[1..]);
    child.envs(cmd.settings.env.iter().map(|(k, v)| (k, v)));
    let root_dir = ctx.root_file.parent().unwrap_or(Path::new("."));
    let alias = format!("{} --file {}", ctx.exe.display(), ctx.root_file.display());
    child.env("WASK", &alias);
    child.env("WASK_DIR", &cmd.dir);
    child.env("WASK_ROOT", root_dir);
    if cmd.settings.mask_compat {
        child.env("MASK", &alias);
        child.env("MASKFILE_DIR", root_dir);
    }
    child.envs(vars.iter().map(|(k, v)| (k, v)));
    if cmd.settings.dir == DirMode::File {
        child.current_dir(&cmd.dir);
    }

    let status = child.status();
    if let Some(temp) = temp {
        let _ = fs::remove_file(temp);
    }
    match status {
        Ok(status) => Ok(status.code().unwrap_or(1)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "`{}` is not installed or not on PATH (set a different runner under `runners:`)",
            argv[0]
        )),
        Err(e) => Err(format!("failed to run `{}`: {e}", argv[0])),
    }
}

/// Fills a runner template. `{script}` is the source itself, `{file}` a
/// temporary file holding it; with neither, the file is appended.
fn inline_argv(
    template: &str,
    source: &str,
    ext: &str,
    temp: &mut Option<PathBuf>,
) -> Result<Vec<String>, String> {
    let mut argv: Vec<String> = template.split_whitespace().map(str::to_string).collect();
    if argv.is_empty() {
        return Err("empty runner".into());
    }
    if !argv
        .iter()
        .any(|a| a.contains("{script}") || a.contains("{file}"))
    {
        argv.push("{file}".into());
    }
    if argv.iter().any(|a| a.contains("{file}")) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!("wask-{}-{nanos}.{ext}", process::id()));
        fs::write(&path, source).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.to_string_lossy().into_owned();
        for arg in &mut argv {
            *arg = arg.replace("{file}", &name);
        }
        *temp = Some(path);
    }
    for arg in &mut argv {
        if arg.contains("{script}") {
            *arg = arg.replace("{script}", source);
        }
    }
    Ok(argv)
}

/// A linked script file runs directly when executable, otherwise through the
/// runner for its extension.
fn file_argv(cmd: &Command, path: &Path) -> Result<Vec<String>, String> {
    let name = path.to_string_lossy().into_owned();
    if is_executable(path) {
        return Ok(vec![name]);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let lang = match ext.as_deref() {
        Some("mjs" | "cjs") => "js",
        Some(ext) => ext,
        None => "",
    };
    let template = cmd.settings.runner(lang).ok_or_else(|| {
        format!("{name}: not executable and no runner is known for its extension")
    })?;
    // Drop the inline-source part of the template (`-c {script}`).
    let mut argv: Vec<String> = Vec::new();
    for part in template.split_whitespace() {
        if part.contains("{script}") {
            if argv.len() > 1 && argv.last().is_some_and(|a| a.starts_with(['-', '/'])) {
                argv.pop();
            }
        } else if !part.contains("{file}") {
            argv.push(part.to_string());
        }
    }
    argv.push(name);
    Ok(argv)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        let mut temp = None;
        let argv = inline_argv("python3 -u -c {script}", "print(1)", "py", &mut temp).unwrap();
        assert_eq!(argv, ["python3", "-u", "-c", "print(1)"]);
        assert!(temp.is_none());

        let argv = inline_argv("deno run", "1", "ts", &mut temp).unwrap();
        assert_eq!(argv.len(), 3);
        assert!(argv[2].ends_with(".ts"));
        assert_eq!(fs::read_to_string(temp.as_ref().unwrap()).unwrap(), "1");
        fs::remove_file(temp.unwrap()).unwrap();
    }
}
