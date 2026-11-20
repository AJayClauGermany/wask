//! The command tree produced by the markdown parser.

use crate::config::Settings;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub desc: String,
    pub args: Vec<Arg>,
    pub options: Vec<Opt>,
    pub script: Option<Script>,
    pub subs: Vec<Command>,
    /// Directory of the markdown file that defined this command.
    pub dir: PathBuf,
    /// Global config, overlaid by the root file's frontmatter, then this file's.
    pub settings: Rc<Settings>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    pub name: String,
    pub required: bool,
    pub variadic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Bool,
    String,
    Number,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Opt {
    pub name: String,
    pub short: Option<char>,
    pub long: Option<String>,
    pub kind: Kind,
    pub choices: Vec<String>,
    pub required: bool,
    pub desc: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Script {
    /// A fenced code block.
    Inline { lang: String, source: String },
    /// A file next to the markdown, linked with `@path`.
    File(PathBuf),
}

/// Environment variable name for an argument or option.
pub fn env_name(name: &str) -> String {
    name.replace('-', "_")
}
