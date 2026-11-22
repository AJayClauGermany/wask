//! Turns a markdown file into a command tree.
//!
//! Headings define commands, the first blockquote (or paragraph) describes
//! them, `**OPTIONS**` lists define flags and fenced code blocks are the
//! scripts: the maskfile format. On top of that, `@path` links import other
//! markdown files (such as a `SKILL.md`) or point a command at a script file,
//! and frontmatter configures how the file is read and run.

use crate::config::{self, Settings};
use crate::model::{Arg, Command, Kind, NearMiss, Opt, Script};
use crate::yaml;
use std::fs;
use std::ops::Range;
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
    /// Headings that look like commands but are documentation, so that asking
    /// for one can say why.
    pub near_misses: Vec<NearMiss>,
}

pub fn warn(msg: &str) {
    eprintln!("wask: warning: {msg}");
}

/// A path relative to the current directory when it is below it, for
/// messages.
pub fn relative(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Info-string words that mark a code block as an example, never a script:
/// ```` ```bash example ````, rustdoc's `ignore` and `no_run`, or Runme's
/// `{ ignore=true }`.
const EXAMPLE_FLAGS: &[&str] = &["example", "ignore", "no_run", "no-run", "norun"];

/// Where commands live in a file that is not a WASK.md or maskfile.md, unless
/// its frontmatter says otherwise. `Tasks` is xc's convention.
const DEFAULT_SECTIONS: &[&str] = &["Commands", "Tasks"];

/// How a file says which of its headings are commands.
struct Rules {
    /// Only headings under the first heading with one of these names are
    /// read; `None` reads the whole file.
    section: Option<Vec<String>>,
    /// Only headings starting with this marker are commands.
    prefix: Option<String>,
    /// Whether `@file.md` links outside command headings import. In a skill
    /// that marks its commands, a link in the prose is a reference for the
    /// reader, not an import.
    prose_imports: bool,
}

impl Rules {
    /// The heading title with the marker removed, or `None` if it is unmarked.
    fn strip<'a>(&self, title: &'a str) -> Option<&'a str> {
        let Some(prefix) = &self.prefix else {
            return Some(title);
        };
        let head = title.get(..prefix.len())?;
        head.eq_ignore_ascii_case(prefix)
            .then(|| title[prefix.len()..].trim_start())
    }

    fn unmarked_reason(&self) -> String {
        let prefix = self.prefix.as_deref().unwrap_or_default();
        format!(
            "it does not start with `{prefix}`, which this file uses to mark commands \
             (`prefix: \"{prefix}\"` in its frontmatter)"
        )
    }
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
    let loc = |line: usize| format!("{}:{line}", relative(&path));

    let mut doc = Doc::default();
    let mut settings = base.clone();
    let mut section = None;
    let mut prefix = None;
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
                // How the file is read is the file's own business, so these
                // two are not settings: they are neither inherited by imports
                // nor accepted from the global config.
                "wask.section" => section = Some(value),
                "wask.prefix" => prefix = Some(value).filter(|p| !p.trim().is_empty()),
                _ => {
                    if let Some(key) = key.strip_prefix("wask.")
                        && let Err(e) = settings.apply(key, &value, false)
                    {
                        warn(&format!("{}: {e}", relative(&path)));
                    }
                }
            }
        }
    }
    let settings = Rc::new(settings);
    let first_line = src.len() - body.len();
    let first_line = src[..first_line].matches('\n').count() + 1;

    // WASK.md and maskfile.md are task files through and through. Any other
    // file (a SKILL.md, say) is mostly prose with example snippets, so only
    // its "Commands" (or "Tasks") section is read, unless frontmatter picks
    // another section or a prefix that marks commands wherever they are.
    let file_name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
    let maskfile = whole || file_name.as_deref() == Some("maskfile.md");
    let task_file = maskfile || file_name.as_deref() == Some("wask.md");
    let rules = Rules {
        prose_imports: task_file || prefix.is_none() || section.is_some(),
        section: match section {
            Some(s) if s == "*" => None,
            Some(s) => Some(vec![s]),
            None if task_file || prefix.is_some() => None,
            None => Some(DEFAULT_SECTIONS.iter().map(|s| s.to_string()).collect()),
        },
        prefix,
    };
    let pick = |blocks: &[Block]| pick_block(blocks, &settings, maskfile);

    let sections = scan(body, first_line);
    let h1 = sections.iter().find(|s| s.level == 1);
    doc.title = h1.map(|s| s.title.clone());
    if doc.description.is_none() {
        doc.description = h1.and_then(|s| s.quote.clone().or_else(|| s.para.clone()));
    }

    // Which sections hold commands, and which ones' `@file.md` links import
    // at the top level (the title area, or the commands section's heading).
    let (roots, scope): (Vec<usize>, Range<usize>) = match &rules.section {
        None => {
            let roots: Vec<usize> = (0..sections.len())
                .filter(|&i| sections[i].level <= 1)
                .collect();
            (roots, 0..sections.len())
        }
        Some(names) => {
            let found = sections
                .iter()
                .position(|s| s.level > 0 && names.iter().any(|n| s.title.eq_ignore_ascii_case(n)));
            match found {
                None => (Vec::new(), 0..0),
                Some(i) => {
                    let level = sections[i].level;
                    let end = sections[i + 1..]
                        .iter()
                        .position(|s| s.level <= level)
                        .map_or(sections.len(), |p| i + 1 + p);
                    (vec![i], i + 1..end)
                }
            }
        }
    };

    struct Node {
        cmd: Command,
        level: usize,
        parent: Option<usize>,
        imports: Vec<PathBuf>,
        /// Explicitly marked with the prefix, so failing to be a command is
        /// worth a warning rather than silence.
        marked: bool,
        /// Why the code blocks it has could not be its script.
        unrunnable: Option<String>,
    }
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut top_imports: Vec<PathBuf> = Vec::new();
    let is_md = |link: &str| link.to_ascii_lowercase().ends_with(".md");
    let md_links = |s: &Section| -> Vec<PathBuf> {
        if !rules.prose_imports {
            return Vec::new();
        }
        s.links
            .iter()
            .filter(|l| is_md(l))
            .map(|l| resolve(&dir, l))
            .collect()
    };
    let mut near = |s: &Section, name: &str, reason: String| {
        doc.near_misses.push(NearMiss {
            name: name.to_string(),
            file: path.clone(),
            line: s.line,
            reason,
        });
    };

    for (i, s) in sections.iter().enumerate() {
        if roots.contains(&i) {
            top_imports.extend(md_links(s));
            continue;
        }
        if !scope.contains(&i) {
            if let (Some(name), Some(names)) = (lone_name(&s.title), &rules.section)
                && pick(&s.blocks).script.is_some()
            {
                let names: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
                near(
                    s,
                    &name,
                    format!(
                        "in this file only headings under {} are commands",
                        names.join(" or ")
                    ),
                );
            }
            continue;
        }
        while stack.last().is_some_and(|&n| nodes[n].level >= s.level) {
            stack.pop();
        }
        let parent = stack.last().copied();
        let marked = rules.prefix.is_some();
        let Some(title) = rules.strip(&s.title) else {
            // Unmarked: documentation, transparent like any prose heading.
            if let Some(name) = lone_name(&s.title)
                && pick(&s.blocks).script.is_some()
            {
                near(s, &name, rules.unmarked_reason());
            }
            match parent {
                Some(p) => nodes[p].imports.extend(md_links(s)),
                None => top_imports.extend(md_links(s)),
            }
            continue;
        };
        let parsed = parse_title(title).and_then(|(mut names, args)| {
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
        let Some((name, args)) = parsed else {
            if marked {
                warn(&format!(
                    "{}: `{}` is marked as a command, but `{title}` is not a command \
                     heading: a name, then (required), [optional] or [rest...] arguments",
                    loc(s.line),
                    s.title
                ));
            }
            // A prose heading: transparent, its imports land one level up.
            match parent {
                Some(p) => nodes[p].imports.extend(md_links(s)),
                None => top_imports.extend(md_links(s)),
            }
            continue;
        };
        let Picked { script, conflict } = pick(&s.blocks);
        let script = script.or_else(|| {
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
                conflict,
                subs: Vec::new(),
                dir: dir.clone(),
                file: path.clone(),
                line: s.line,
                settings: settings.clone(),
            },
            level: s.level,
            parent,
            imports: s
                .links
                .iter()
                .filter(|l| is_md(l))
                .map(|l| resolve(&dir, l))
                .collect(),
            marked,
            unrunnable: unrunnable(s, &dir),
        });
        stack.push(nodes.len() - 1);
    }

    let mut import = |target: &Path, doc: &mut Doc, top: bool| -> Result<Vec<Command>, String> {
        let Ok(target) = target.canonicalize() else {
            warn(&format!(
                "{}: import not found: {}",
                relative(&path),
                relative(target)
            ));
            return Ok(Vec::new());
        };
        if seen.contains(&target) {
            return Ok(Vec::new());
        }
        let imported = load(&target, &settings, false, seen)?;
        if top {
            doc.name = doc.name.take().or(imported.name);
            doc.title = doc.title.take().or(imported.title);
            doc.description = doc.description.take().or(imported.description);
        }
        doc.near_misses.extend(imported.near_misses);
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
            cmd.subs.extend(import(target, &mut doc, false)?);
        }
        dedup(&mut cmd.subs, &path);
        if cmd.script.is_none() && cmd.subs.is_empty() {
            // A heading that is only documentation.
            if let Some(reason) = node.unrunnable {
                if node.marked {
                    warn(&format!(
                        "{}: `{}` is marked as a command, but {reason}",
                        loc(cmd.line),
                        cmd.name
                    ));
                }
                doc.near_misses.push(NearMiss {
                    name: cmd.name,
                    file: cmd.file,
                    line: cmd.line,
                    reason,
                });
            } else if node.marked {
                warn(&format!(
                    "{}: `{}` is marked as a command, but has no code block, \
                     `@script` link or subcommands",
                    loc(cmd.line),
                    cmd.name
                ));
            }
            continue;
        }
        match node.parent {
            Some(p) => children[p].push(cmd),
            None => top.push(cmd),
        }
    }
    top.reverse();
    for target in &top_imports {
        top.extend(import(target, &mut doc, true)?);
    }
    dedup(&mut top, &path);
    doc.commands = top;
    Ok(doc)
}

/// The command name a heading would have on its own, if it reads as one.
fn lone_name(title: &str) -> Option<String> {
    let (mut names, _) = parse_title(title)?;
    (names.len() == 1).then(|| names.remove(0))
}

/// Why a heading that has code blocks or script links has no script.
fn unrunnable(s: &Section, dir: &Path) -> Option<String> {
    let missing = s
        .links
        .iter()
        .find(|l| !l.to_ascii_lowercase().ends_with(".md") && !resolve(dir, l).is_file());
    if let Some(link) = missing.filter(|_| s.blocks.is_empty()) {
        return Some(format!("its script `@{link}` was not found"));
    }
    let blocks = &s.blocks;
    if blocks.is_empty() {
        return None;
    }
    if blocks.iter().all(|b| b.example) {
        return Some("its code blocks are all marked as examples".into());
    }
    let mut langs: Vec<String> = Vec::new();
    for b in blocks.iter().filter(|b| !b.example) {
        let lang = if b.lang.is_empty() {
            "(no language)".to_string()
        } else {
            format!("`{}`", b.lang)
        };
        if !langs.contains(&lang) {
            langs.push(lang);
        }
    }
    let what = if langs.len() == 1 {
        format!("its code block is {}", langs[0])
    } else {
        format!("its code blocks are {}", langs.join(", "))
    };
    Some(format!(
        "{what}, and wask has no runner for that: use a language such as `sh`, `bash` \
         or `python`, or add one under `runners:`"
    ))
}

fn dedup(cmds: &mut Vec<Command>, path: &Path) {
    let mut names: Vec<String> = Vec::new();
    cmds.retain(|c| {
        if names.contains(&c.name) {
            warn(&format!(
                "{}: duplicate command `{}` ignored",
                relative(path),
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

struct Picked {
    script: Option<Script>,
    /// Why the script is not clear-cut: more than one block could be it.
    conflict: Option<String>,
}

/// The block that runs here, preferring ones written for this platform.
/// Blocks marked as examples never run. A maskfile runs the last candidate,
/// as mask does; anywhere else, more than one candidate is a conflict that
/// stops the command from running rather than a guess.
fn pick_block(blocks: &[Block], settings: &Settings, maskfile: bool) -> Picked {
    let usable: Vec<&Block> = blocks
        .iter()
        .filter(|b| !b.example && settings.runner(&b.lang).is_some())
        .collect();
    let native: Vec<&Block> = usable
        .iter()
        .copied()
        .filter(|b| config::is_windows_lang(&b.lang) == cfg!(windows))
        .collect();
    // Blocks for another platform are a fallback, not competing candidates.
    let competing = native.len() > 1 && !maskfile;
    let pool = if native.is_empty() { usable } else { native };
    let chosen = if maskfile { pool.last() } else { pool.first() };
    let conflict = competing.then(|| {
        format!(
            "it has {} code blocks that could be its script, and wask will not guess \
             which one to run. Mark each block that is not the script as an example: \
             ```{} example",
            pool.len(),
            pool[1].lang
        )
    });
    Picked {
        script: chosen.map(|b| Script::Inline {
            lang: b.lang.clone(),
            source: b.source.clone(),
        }),
        conflict,
    }
}

/// `name (required) [optional] [rest...]`. Backticks are ignored, so
/// `` `build (target)` `` and `` `build` (target) `` read the same.
fn parse_title(title: &str) -> Option<(Vec<String>, Vec<Arg>)> {
    let title = title.replace('`', "");
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
    /// Line of the heading in the file.
    line: usize,
    quote: Option<String>,
    para: Option<String>,
    blocks: Vec<Block>,
    opts: Vec<Opt>,
    links: Vec<String>,
}

#[derive(Debug)]
struct Block {
    lang: String,
    source: String,
    example: bool,
}

/// An open code fence: marker char, marker length, indent, language, and
/// whether the info string marks it as an example.
pub struct Fence {
    ch: char,
    len: usize,
    indent: usize,
    pub lang: String,
    pub example: bool,
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
        let mut words = info
            .split(|c: char| c.is_whitespace() || c == '{' || c == '}' || c == ',')
            .filter(|w| !w.is_empty());
        let lang = match info.starts_with('{') {
            true => String::new(),
            false => words.next().unwrap_or("").to_ascii_lowercase(),
        };
        // `bash example`, `bash,ignore`, `bash {.no_run}`, `sh { ignore=true }`,
        // `sh {"ignore":"true"}`.
        let example = words.any(|w| {
            let w = w.trim_start_matches('.').to_ascii_lowercase();
            let (key, value) = match w.split_once(['=', ':']) {
                Some((k, v)) => (k.trim_matches('"'), Some(v.trim_matches('"'))),
                None => (w.as_str(), None),
            };
            EXAMPLE_FLAGS.contains(&key) && value.is_none_or(|v| yaml::parse_bool(v) == Some(true))
        });
        Some(Fence {
            ch,
            len,
            indent: line.len() - text.len(),
            lang,
            example,
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

/// Splits the body into sections, one per heading. `first_line` is the line
/// number of the body's first line in the file.
fn scan(body: &str, first_line: usize) -> Vec<Section> {
    let mut secs = vec![Section::default()];
    let mut fence: Option<(Fence, Vec<&str>)> = None;
    // Inside an OPTIONS list: the indent of its top-level bullets, once known.
    let mut opts: Option<Option<usize>> = None;

    for (n, line) in body.lines().enumerate() {
        let cur = secs.last_mut().expect("starts non-empty");
        if let Some((f, buf)) = &mut fence {
            if f.closes(line) {
                let mut source = buf.join("\n");
                source.push('\n');
                cur.blocks.push(Block {
                    lang: f.lang.clone(),
                    source,
                    example: f.example,
                });
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
                line: first_line + n,
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
        assert_eq!(parse_title("`lint` (dir)").unwrap().1[0].name, "dir");
    }

    #[test]
    fn fence_flags() {
        let open = |line: &str| Fence::open(line).map(|f| (f.lang, f.example));
        assert_eq!(open("```bash"), Some(("bash".into(), false)));
        assert_eq!(open("```bash example"), Some(("bash".into(), true)));
        assert_eq!(open("```rust,ignore"), Some(("rust".into(), true)));
        assert_eq!(open("~~~sh {.no_run}"), Some(("sh".into(), true)));
        assert_eq!(open("```sh title=\"example\""), Some(("sh".into(), false)));
        assert_eq!(open("```{example}"), Some((String::new(), true)));
        assert_eq!(open("```sh { ignore=true }"), Some(("sh".into(), true)));
        assert_eq!(
            open("```sh {\"ignore\":\"false\"}"),
            Some(("sh".into(), false))
        );
    }

    #[test]
    fn prefix_rules() {
        let rules = Rules {
            section: None,
            prefix: Some("task:".into()),
            prose_imports: false,
        };
        assert_eq!(rules.strip("task: build (x)"), Some("build (x)"));
        assert_eq!(rules.strip("Task:build"), Some("build"));
        assert_eq!(rules.strip("Building"), None);
        let ticks = Rules {
            section: None,
            prefix: Some("`".into()),
            prose_imports: false,
        };
        assert_eq!(
            parse_title(ticks.strip("`build` [x]").unwrap()).unwrap().0,
            ["build"]
        );
    }

    #[test]
    fn scans_sections() {
        let secs = scan(
            "# Title\n\nIntro with @SKILL.md link.\n\n## serve (dir)\n\n> Serve files\n\n\
             **OPTIONS**\n* port\n    * flags: -p --port\n    * type: number\n    * required\n\
             * watch\n\n~~~bash\necho \"## not a heading\"\n~~~\n\n```sh example\nwask serve\n```\n",
            3,
        );
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[1].links, ["SKILL.md"]);
        let serve = &secs[2];
        assert_eq!(serve.quote.as_deref(), Some("Serve files"));
        assert_eq!((serve.line, serve.blocks[0].lang.as_str()), (7, "bash"));
        assert!(!serve.blocks[0].example && serve.blocks[1].example);
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
