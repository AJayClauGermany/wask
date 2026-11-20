//! Turns a markdown file into a command tree.
//!
//! Headings define commands, the first blockquote (or paragraph) describes
//! them, `**OPTIONS**` lists define flags and fenced code blocks are the
//! scripts: the maskfile format. On top of that, `@path` links import other
//! markdown files (such as a `SKILL.md`) or point a command at a script file,
//! and frontmatter configures how the file is read and run.

use crate::config::{self, Settings};
use crate::model::{Arg, Command, Kind, Opt, Script};
use crate::yaml;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Imported files beyond this are almost certainly a mistake.
const MAX_FILES: usize = 64;

#[derive(Debug, Default)]
pub struct Doc {
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub commands: Vec<Command>,
}

pub fn warn(msg: &str) {
    eprintln!("wask: warning: {msg}");
}

/// Loads `path` and everything it imports. `whole` forces maskfile semantics
/// (every heading can be a command) regardless of the file's name.
pub fn load(
    path: &Path,
    base: &Settings,
    whole: bool,
    seen: &mut Vec<PathBuf>,
) -> Result<Doc, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let src = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    seen.push(path.clone());
    if seen.len() > MAX_FILES {
        return Err(format!("more than {MAX_FILES} files imported; giving up"));
    }
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut doc = Doc::default();
    let mut settings = base.clone();
    let mut section = None;
    let (front, body) = yaml::split_frontmatter(&src);
    if let Some(front) = front {
        let pairs =
            yaml::parse(front).map_err(|e| format!("{}: frontmatter: {e}", path.display()))?;
        for (key, value) in pairs {
            // The skills spec keeps extensions under `metadata`; accept
            // `metadata: { wask.dir: file }` as well as a `wask:` block.
            let key = key
                .strip_prefix("metadata.")
                .filter(|rest| rest.starts_with("wask."))
                .unwrap_or(&key);
            match key {
                "name" => doc.name = Some(value),
                "description" => doc.description = Some(value),
                "wask.section" => section = Some(value),
                _ => {
                    if let Some(key) = key.strip_prefix("wask.")
                        && let Err(e) = settings.apply(key, &value, false)
                    {
                        warn(&format!("{}: {e}", path.display()));
                    }
                }
            }
        }
    }
    let settings = Rc::new(settings);

    // WASK.md and maskfile.md are task files through and through. Any other
    // file (a SKILL.md, say) is mostly prose with example snippets, so only
    // its "Commands" section is read unless frontmatter says otherwise.
    let file_name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
    let task_file = whole || matches!(file_name.as_deref(), Some("wask.md" | "maskfile.md"));
    let section = match section {
        Some(s) if s == "*" => None,
        Some(s) => Some(s),
        None if task_file => None,
        None => Some("Commands".to_string()),
    };

    let sections = scan(body);
    let h1 = sections.iter().find(|s| s.level == 1);
    doc.title = h1.map(|s| s.title.clone());
    if doc.description.is_none() {
        doc.description = h1.and_then(|s| s.quote.clone().or_else(|| s.para.clone()));
    }

    let (root_secs, cmd_secs): (Vec<&Section>, Vec<&Section>) = match &section {
        None => sections.iter().partition(|s| s.level <= 1),
        Some(name) => {
            let found = sections
                .iter()
                .position(|s| s.level > 0 && s.title.eq_ignore_ascii_case(name));
            match found {
                None => (Vec::new(), Vec::new()),
                Some(i) => {
                    let level = sections[i].level;
                    let end = sections[i + 1..]
                        .iter()
                        .position(|s| s.level <= level)
                        .map_or(sections.len(), |p| i + 1 + p);
                    (vec![&sections[i]], sections[i + 1..end].iter().collect())
                }
            }
        }
    };

    struct Node {
        cmd: Command,
        level: usize,
        parent: Option<usize>,
        imports: Vec<PathBuf>,
    }
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut top_imports: Vec<PathBuf> = Vec::new();
    let is_md = |link: &str| link.to_ascii_lowercase().ends_with(".md");

    for s in root_secs {
        top_imports.extend(
            s.links
                .iter()
                .filter(|l| is_md(l))
                .map(|l| resolve(&dir, l)),
        );
    }
    for s in cmd_secs {
        while stack.last().is_some_and(|&i| nodes[i].level >= s.level) {
            stack.pop();
        }
        let parent = stack.last().copied();
        let parsed = parse_title(&s.title).and_then(|(mut names, args)| {
            // `### services start` under `## services` may repeat its parents.
            let mut prefix = Vec::new();
            let mut at = parent;
            while let Some(i) = at {
                prefix.insert(0, nodes[i].cmd.name.clone());
                at = nodes[i].parent;
            }
            if names.len() > prefix.len() && names[..prefix.len()] == prefix[..] {
                names.drain(..prefix.len());
            }
            (names.len() == 1).then(|| (names.remove(0), args))
        });
        let md_links = s
            .links
            .iter()
            .filter(|l| is_md(l))
            .map(|l| resolve(&dir, l));
        let Some((name, args)) = parsed else {
            // A prose heading: transparent, its imports land one level up.
            match parent {
                Some(p) => nodes[p].imports.extend(md_links),
                None => top_imports.extend(md_links),
            }
            continue;
        };
        let imports: Vec<PathBuf> = md_links.collect();
        let script = pick_block(&s.blocks, &settings).or_else(|| {
            s.links
                .iter()
                .filter(|l| !is_md(l))
                .map(|l| resolve(&dir, l))
                .find(|p| p.is_file())
                .map(Script::File)
        });
        nodes.push(Node {
            cmd: Command {
                name,
                desc: s
                    .quote
                    .clone()
                    .or_else(|| s.para.clone())
                    .unwrap_or_default(),
                args,
                options: s.opts.clone(),
                script,
                subs: Vec::new(),
                dir: dir.clone(),
                settings: settings.clone(),
            },
            level: s.level,
            parent,
            imports,
        });
        stack.push(nodes.len() - 1);
    }

    let mut import = |target: &Path, doc: Option<&mut Doc>| -> Result<Vec<Command>, String> {
        let Ok(target) = target.canonicalize() else {
            warn(&format!(
                "{}: import not found: {}",
                path.display(),
                target.display()
            ));
            return Ok(Vec::new());
        };
        if seen.contains(&target) {
            return Ok(Vec::new());
        }
        let imported = load(&target, &settings, false, seen)?;
        if let Some(doc) = doc {
            doc.name = doc.name.take().or(imported.name);
            doc.title = doc.title.take().or(imported.title);
            doc.description = doc.description.take().or(imported.description);
        }
        Ok(imported.commands)
    };

    // Children always follow their parent, so assemble back to front.
    let mut children: Vec<Vec<Command>> = nodes.iter().map(|_| Vec::new()).collect();
    let mut top: Vec<Command> = Vec::new();
    for i in (0..nodes.len()).rev() {
        let node = nodes.pop().expect("one node per index");
        let mut cmd = node.cmd;
        cmd.subs = std::mem::take(&mut children[i]);
        cmd.subs.reverse();
        for target in &node.imports {
            cmd.subs.extend(import(target, None)?);
        }
        dedup(&mut cmd.subs, &path);
        if cmd.script.is_none() && cmd.subs.is_empty() {
            continue; // a heading that is only documentation
        }
        match node.parent {
            Some(p) => children[p].push(cmd),
            None => top.push(cmd),
        }
    }
    top.reverse();
    for target in &top_imports {
        top.extend(import(target, Some(&mut doc))?);
    }
    dedup(&mut top, &path);
    doc.commands = top;
    Ok(doc)
}

fn dedup(cmds: &mut Vec<Command>, path: &Path) {
    let mut names: Vec<String> = Vec::new();
    cmds.retain(|c| {
        if names.contains(&c.name) {
            warn(&format!(
                "{}: duplicate command `{}` ignored",
                path.display(),
                c.name
            ));
            return false;
        }
        names.push(c.name.clone());
        true
    });
}

fn resolve(dir: &Path, link: &str) -> PathBuf {
    if let Some(rest) = link.strip_prefix("~/")
        && let Some(home) = config::home_dir()
    {
        return home.join(rest);
    }
    dir.join(link) // `join` keeps absolute links as they are
}

/// The first block that can run here: a platform-specific one if present.
fn pick_block(blocks: &[(String, String)], settings: &Settings) -> Option<Script> {
    let known = |lang: &str| settings.runner(lang).is_some();
    blocks
        .iter()
        .find(|(l, _)| known(l) && config::is_windows_lang(l) == cfg!(windows))
        .or_else(|| blocks.iter().find(|(l, _)| known(l)))
        .map(|(lang, source)| Script::Inline {
            lang: lang.clone(),
            source: source.clone(),
        })
}

/// `name (required) [optional] [rest...]`, optionally wrapped in backticks.
fn parse_title(title: &str) -> Option<(Vec<String>, Vec<Arg>)> {
    let title = title.trim().trim_matches('`');
    let mut names = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    for token in title.split_whitespace() {
        let inner = token
            .strip_prefix('(')
            .and_then(|t| t.strip_suffix(')'))
            .map(|t| (t, true))
            .or_else(|| {
                token
                    .strip_prefix('[')
                    .and_then(|t| t.strip_suffix(']'))
                    .map(|t| (t, false))
            });
        match inner {
            Some((inner, required)) => {
                let (name, variadic) = match inner.strip_suffix("...") {
                    Some(name) => (name, true),
                    None => (inner, false),
                };
                let valid = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
                // Nothing may follow a variadic argument.
                if !valid || args.last().is_some_and(|a| a.variadic) {
                    return None;
                }
                args.push(Arg {
                    name: name.to_string(),
                    required,
                    variadic,
                });
            }
            None => {
                let valid = args.is_empty()
                    && token.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                    && token
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_-.:".contains(c));
                if !valid {
                    return None;
                }
                names.push(token.to_string());
            }
        }
    }
    (!names.is_empty()).then_some((names, args))
}

#[derive(Debug, Default)]
struct Section {
    level: usize,
    title: String,
    quote: Option<String>,
    para: Option<String>,
    blocks: Vec<(String, String)>,
    opts: Vec<Opt>,
    links: Vec<String>,
}

/// An open code fence: marker char, marker length, indent, language, lines.
pub struct Fence {
    ch: char,
    len: usize,
    indent: usize,
    pub lang: String,
}

impl Fence {
    pub fn open(line: &str) -> Option<Fence> {
        let text = line.trim_start();
        let ch = text.chars().next().filter(|c| *c == '`' || *c == '~')?;
        let len = text.chars().take_while(|c| *c == ch).count();
        let info = text[len..].trim();
        if len < 3 || (ch == '`' && info.contains('`')) {
            return None;
        }
        let lang = info
            .split(|c: char| c.is_whitespace() || c == '{' || c == ',')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        Some(Fence {
            ch,
            len,
            indent: line.len() - text.len(),
            lang,
        })
    }

    pub fn closes(&self, line: &str) -> bool {
        let text = line.trim();
        text.chars().count() >= self.len && text.chars().all(|c| c == self.ch)
    }

    /// A content line with the fence's own indentation removed.
    pub fn strip<'a>(&self, line: &'a str) -> &'a str {
        let lead = line.len() - line.trim_start().len();
        &line[lead.min(self.indent)..]
    }
}

/// Words like `@SKILL.md` or `@scripts/run.sh`: an `@` at the start of a word
/// followed by something that looks like a path.
pub fn at_links(line: &str) -> impl Iterator<Item = &str> {
    line.split_whitespace().filter_map(|word| {
        let word = word.trim_start_matches(['(', '[', '"', '\'', '`', '*']);
        let path = word
            .strip_prefix('@')?
            .trim_end_matches(['.', ',', ';', ':', ')', ']', '"', '\'', '`', '*']);
        (path.contains(['.', '/']) && !path.contains('@')).then_some(path)
    })
}

fn bullet(line: &str) -> Option<(usize, &str)> {
    let text = line.trim_start();
    let rest = ["* ", "- ", "+ "]
        .iter()
        .find_map(|b| text.strip_prefix(b))?;
    Some((line.len() - text.len(), rest.trim()))
}

fn scan(body: &str) -> Vec<Section> {
    let mut secs = vec![Section::default()];
    let mut fence: Option<(Fence, Vec<&str>)> = None;
    // Inside an OPTIONS list: the indent of its top-level bullets, once known.
    let mut opts: Option<Option<usize>> = None;

    for line in body.lines() {
        let cur = secs.last_mut().expect("starts non-empty");
        if let Some((f, buf)) = &mut fence {
            if f.closes(line) {
                let mut source = buf.join("\n");
                source.push('\n');
                cur.blocks.push((f.lang.clone(), source));
                fence = None;
            } else {
                buf.push(f.strip(line));
            }
            continue;
        }
        if let Some(f) = Fence::open(line) {
            fence = Some((f, Vec::new()));
            opts = None;
            continue;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(top) = &mut opts {
            if let Some((indent, item)) = bullet(line) {
                if indent <= *top.get_or_insert(indent) {
                    cur.opts.push(Opt {
                        name: item.trim_matches('`').to_string(),
                        short: None,
                        long: None,
                        kind: Kind::Bool,
                        choices: Vec::new(),
                        required: false,
                        desc: String::new(),
                    });
                } else if let Some(opt) = cur.opts.last_mut() {
                    option_property(opt, item);
                }
                continue;
            }
            for opt in cur
                .opts
                .iter_mut()
                .filter(|o| o.short.is_none() && o.long.is_none())
            {
                opt.long = Some(opt.name.clone());
            }
            opts = None;
        }

        let hashes = text.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && text[hashes..].starts_with([' ', '\t']) {
            secs.push(Section {
                level: hashes,
                title: text[hashes..]
                    .trim()
                    .trim_end_matches('#')
                    .trim()
                    .to_string(),
                ..Section::default()
            });
        } else if let Some(quote) = text.strip_prefix('>') {
            let quote = quote.trim();
            if cur.quote.is_none() && cur.blocks.is_empty() && !quote.is_empty() {
                cur.quote = Some(quote.to_string());
            }
        } else if text
            .trim_matches('*')
            .trim_end_matches(':')
            .eq_ignore_ascii_case("options")
            && text.starts_with("**")
        {
            opts = Some(None);
        } else {
            let before = cur.links.len();
            cur.links.extend(at_links(text).map(str::to_string));
            let plain = cur.links.len() == before && bullet(line).is_none();
            if plain && cur.para.is_none() && cur.blocks.is_empty() {
                cur.para = Some(text.to_string());
            }
        }
    }
    for sec in &mut secs {
        for opt in sec
            .opts
            .iter_mut()
            .filter(|o| o.short.is_none() && o.long.is_none())
        {
            opt.long = Some(opt.name.clone());
        }
    }
    secs
}

fn option_property(opt: &mut Opt, item: &str) {
    let (key, value) = match item.split_once(':') {
        Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim()),
        None => (item.trim().to_ascii_lowercase(), ""),
    };
    match key.as_str() {
        "flags" => {
            for flag in value.split([' ', ',']).filter(|f| !f.is_empty()) {
                if let Some(long) = flag.strip_prefix("--") {
                    opt.long = Some(long.to_string());
                } else if let Some(short) = flag.strip_prefix('-') {
                    opt.short = short.chars().next();
                }
            }
        }
        "type" => {
            opt.kind = match value.to_ascii_lowercase().as_str() {
                "string" => Kind::String,
                "number" => Kind::Number,
                _ => Kind::Bool,
            }
        }
        "choices" => {
            opt.choices = value
                .split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();
            if opt.kind == Kind::Bool {
                opt.kind = Kind::String;
            }
        }
        "desc" | "description" => opt.desc = value.to_string(),
        "required" => opt.required = value.is_empty() || yaml::parse_bool(value) == Some(true),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        let (names, args) = parse_title("services start (name) [port] [rest...]").unwrap();
        assert_eq!(names, ["services", "start"]);
        assert_eq!(args.len(), 3);
        assert!(args[0].required && !args[1].required && args[2].variadic);
        assert!(parse_title("Why (not) this?").is_none());
        assert!(parse_title("build (a...) (b)").is_none());
        assert_eq!(parse_title("`lint`").unwrap().0, ["lint"]);
    }

    #[test]
    fn scans_sections() {
        let secs = scan(
            "# Title\n\nIntro with @SKILL.md link.\n\n## serve (dir)\n\n> Serve files\n\n\
             **OPTIONS**\n* port\n    * flags: -p --port\n    * type: number\n    * required\n\
             * watch\n\n~~~bash\necho \"## not a heading\"\n~~~\n",
        );
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[1].links, ["SKILL.md"]);
        let serve = &secs[2];
        assert_eq!(serve.quote.as_deref(), Some("Serve files"));
        assert_eq!(serve.blocks[0].0, "bash");
        assert_eq!(serve.opts.len(), 2);
        assert_eq!(serve.opts[0].short, Some('p'));
        assert!(serve.opts[0].required && serve.opts[0].kind == Kind::Number);
        assert_eq!(serve.opts[1].long.as_deref(), Some("watch"));
    }

    #[test]
    fn at_link_words() {
        let found: Vec<&str> =
            at_links("See @docs/a.md, (@b.sh) not @mention or me@x.org").collect();
        assert_eq!(found, ["docs/a.md", "b.sh"]);
    }
}
