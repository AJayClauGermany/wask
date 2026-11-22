//! Settings: built-in defaults, overlaid by the global config file, then by
//! the `wask:` frontmatter of the root file, then of each imported file.

use crate::yaml;
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DirMode {
    /// Run scripts where wask was invoked (mask's behaviour).
    Cwd,
    /// Run scripts in the directory of the file that defines them.
    File,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContextMode {
    Ask,
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub struct Settings {
    /// Language -> runner template, e.g. `python3 -u -c {script}`.
    pub runners: BTreeMap<String, String>,
    pub env: Vec<(String, String)>,
    pub dir: DirMode,
    /// Also export `$MASK` and `$MASKFILE_DIR`.
    pub mask_compat: bool,
    /// Policy for `!` dynamic context injection. Global config only.
    pub context: ContextMode,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            runners: BTreeMap::new(),
            env: Vec::new(),
            dir: DirMode::Cwd,
            mask_compat: false,
            context: ContextMode::Ask,
        }
    }
}

impl Settings {
    /// Applies one flattened `key: value` setting. `trusted` is true only for
    /// the global config: a markdown file must not grant itself permissions.
    pub fn apply(&mut self, key: &str, value: &str, trusted: bool) -> Result<(), String> {
        if let Some(lang) = key.strip_prefix("runners.") {
            self.runners.insert(canonical_lang(lang), value.to_string());
        } else if let Some(name) = key.strip_prefix("env.") {
            self.env.retain(|(k, _)| k != name);
            self.env.push((name.to_string(), value.to_string()));
        } else if key == "dir" {
            self.dir = match value {
                "cwd" => DirMode::Cwd,
                "file" => DirMode::File,
                _ => return Err(format!("`dir` must be `cwd` or `file`, got `{value}`")),
            };
        } else if key == "mask-compat" {
            self.mask_compat = yaml::parse_bool(value)
                .ok_or_else(|| format!("`mask-compat` must be true or false, got `{value}`"))?;
        } else if key == "context" {
            if !trusted {
                return Err("`context` can only be set in the global config".into());
            }
            self.context = match value {
                "ask" => ContextMode::Ask,
                "allow" => ContextMode::Allow,
                "deny" => ContextMode::Deny,
                _ => {
                    return Err(format!(
                        "`context` must be ask, allow or deny, got `{value}`"
                    ));
                }
            };
        } else if key == "prefix" || key == "section" {
            return Err(format!(
                "`{key}` says how a file is written, so it belongs in that file's \
                 frontmatter, not here"
            ));
        } else {
            return Err(format!("unknown setting `{key}`"));
        }
        Ok(())
    }

    /// The runner template for a code block language, if one is known.
    pub fn runner(&self, lang: &str) -> Option<String> {
        let lang = canonical_lang(lang);
        self.runners
            .get(&lang)
            .cloned()
            .or_else(|| builtin_runner(&lang))
    }
}

pub fn canonical_lang(lang: &str) -> String {
    match lang.to_ascii_lowercase().as_str() {
        "javascript" => "js".into(),
        "python" => "py".into(),
        "ruby" => "rb".into(),
        "pwsh" | "ps1" => "powershell".into(),
        "batch" | "bat" => "cmd".into(),
        other => other.to_string(),
    }
}

pub fn is_windows_lang(lang: &str) -> bool {
    matches!(canonical_lang(lang).as_str(), "powershell" | "cmd")
}

fn builtin_runner(lang: &str) -> Option<String> {
    let template = match lang {
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" => {
            return Some(format!("{lang} -c {{script}}"));
        }
        "js" => "node -e {script}",
        "py" if cfg!(windows) => "python -c {script}",
        "py" => "python3 -c {script}",
        "rb" => "ruby -e {script}",
        "php" => "php -r {script}",
        "lua" => "lua -e {script}",
        "powershell" if cfg!(windows) => "powershell -c {script}",
        "powershell" => "pwsh -c {script}",
        "cmd" => "cmd /c {script}",
        "swift" => "swift {file}",
        _ => return None,
    };
    Some(template.to_string())
}

/// `$WASK_CONFIG`, else `$XDG_CONFIG_HOME/wask/config.yaml`, else
/// `~/.config/wask/config.yaml`.
pub fn config_path() -> Option<PathBuf> {
    let var = |name: &str| {
        env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(path) = var("WASK_CONFIG") {
        return Some(path);
    }
    let base = var("XDG_CONFIG_HOME").or_else(|| home_dir().map(|h| h.join(".config")))?;
    Some(base.join("wask").join("config.yaml"))
}

pub fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|name| env::var_os(name).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

/// Approved `!` commands live next to the config file.
pub fn permissions_path() -> Option<PathBuf> {
    Some(config_path()?.with_file_name("permissions"))
}

pub fn load_global() -> Result<Settings, String> {
    let mut settings = Settings::default();
    let Some(path) = config_path() else {
        return Ok(settings);
    };
    let Ok(src) = std::fs::read_to_string(&path) else {
        return Ok(settings);
    };
    let (front, body) = yaml::split_frontmatter(&src);
    let pairs =
        yaml::parse(front.unwrap_or(body)).map_err(|e| format!("{}: {e}", path.display()))?;
    for (key, value) in pairs {
        settings
            .apply(&key, &value, true)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runners_resolve_through_aliases() {
        let mut s = Settings::default();
        assert_eq!(s.runner("bash").unwrap(), "bash -c {script}");
        assert_eq!(s.runner("JavaScript").unwrap(), "node -e {script}");
        assert!(s.runner("mermaid").is_none());
        s.apply("runners.python", "uv run python -c {script}", false)
            .unwrap();
        assert_eq!(s.runner("py").unwrap(), "uv run python -c {script}");
    }

    #[test]
    fn files_cannot_grant_themselves_context() {
        let mut s = Settings::default();
        assert!(s.apply("context", "allow", false).is_err());
        assert_eq!(s.context, ContextMode::Ask);
        s.apply("context", "allow", true).unwrap();
        assert_eq!(s.context, ContextMode::Allow);
        assert!(s.apply("nope", "1", true).is_err());
    }
}
