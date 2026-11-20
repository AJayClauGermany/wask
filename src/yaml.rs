//! A small YAML subset, enough for skill frontmatter and the global config:
//! nested maps, plain and quoted scalars, inline and block lists, and block
//! scalars (`|`, `>`). The result is flattened to dotted keys; lists are
//! joined with ", ".

pub type Pairs = Vec<(String, String)>;

/// Splits a leading `---` frontmatter block from the rest of the document.
pub fn split_frontmatter(src: &str) -> (Option<&str>, &str) {
    let Some(rest) = src.strip_prefix("---") else {
        return (None, src);
    };
    let Some(nl) = rest.find('\n') else {
        return (None, src);
    };
    if !rest[..nl].trim().is_empty() {
        return (None, src);
    }
    let rest = &rest[nl + 1..];
    let mut off = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (Some(&rest[..off]), &rest[off + line.len()..]);
        }
        off += line.len();
    }
    (None, src)
}

pub fn parse(src: &str) -> Result<Pairs, String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out: Pairs = Vec::new();
    // (indent, dotted key) of the maps we are currently inside.
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        i += 1;
        let text = raw.trim_start();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let indent = raw.len() - text.len();
        if raw[..indent].contains('\t') {
            return Err(format!("line {i}: tabs are not allowed for indentation"));
        }
        if text == "-" || text.starts_with("- ") {
            // A list may sit at the same indent as its key.
            while stack.last().is_some_and(|(n, _)| *n > indent) {
                stack.pop();
            }
            let key = match stack.last() {
                Some((_, k)) => k.clone(),
                None => return Err(format!("line {i}: list item without a key")),
            };
            let item = scalar(strip_comment(text[1..].trim()));
            match out.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => {
                    v.push_str(", ");
                    v.push_str(&item);
                }
                None => out.push((key, item)),
            }
            continue;
        }
        while stack.last().is_some_and(|(n, _)| *n >= indent) {
            stack.pop();
        }
        let (key, value) =
            split_key(text).ok_or_else(|| format!("line {i}: expected `key: value`"))?;
        let path = match stack.last() {
            Some((_, p)) => format!("{p}.{key}"),
            None => key,
        };
        let value = strip_comment(value);
        if value.is_empty() {
            stack.push((indent, path));
        } else if is_block_marker(value) {
            let mut block: Vec<&str> = Vec::new();
            while i < lines.len() {
                let l = lines[i];
                if !l.trim().is_empty() && l.len() - l.trim_start().len() <= indent {
                    break;
                }
                block.push(l);
                i += 1;
            }
            out.push((path, block_scalar(&block, value.starts_with('>'))));
        } else {
            out.push((path, scalar(value)));
        }
    }
    Ok(out)
}

pub fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn split_key(text: &str) -> Option<(String, &str)> {
    let bytes = text.as_bytes();
    let start = match bytes[0] {
        q @ (b'"' | b'\'') => text[1..].find(q as char)? + 2,
        _ => 0,
    };
    let colon = (start..bytes.len())
        .find(|&j| bytes[j] == b':' && bytes.get(j + 1).is_none_or(|c| *c == b' '))?;
    let key = scalar(text[..colon].trim());
    (!key.is_empty()).then(|| (key, text[colon + 1..].trim()))
}

fn strip_comment(value: &str) -> &str {
    let value = value.trim();
    if let Some(q) = value.chars().next().filter(|c| *c == '"' || *c == '\'') {
        // Find the closing quote, skipping `\"` and `''` escapes.
        let mut chars = value.char_indices().skip(1).peekable();
        while let Some((at, c)) = chars.next() {
            if c == '\\' && q == '"' {
                chars.next();
            } else if c == q {
                if q == '\'' && chars.peek().is_some_and(|(_, n)| *n == q) {
                    chars.next();
                } else {
                    return &value[..=at];
                }
            }
        }
        return value;
    }
    match value.find(" #") {
        Some(at) => value[..at].trim_end(),
        None if value.starts_with('#') => "",
        None => value,
    }
}

fn is_block_marker(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('|' | '>'))
        && chars.all(|c| c == '+' || c == '-' || c.is_ascii_digit())
}

fn block_scalar(lines: &[&str], fold: bool) -> String {
    let dedent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut out = String::new();
    for line in lines {
        let line = line.get(dedent..).unwrap_or("").trim_end();
        if fold {
            if line.is_empty() {
                out.push('\n');
            } else {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(line);
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

fn scalar(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let mut out = String::new();
        let mut chars = v[1..v.len() - 1].chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        out
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else if v.len() >= 2 && v.starts_with('[') && v.ends_with(']') {
        let items: Vec<String> = v[1..v.len() - 1]
            .split(',')
            .map(scalar)
            .filter(|s| !s.is_empty())
            .collect();
        items.join(", ")
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(pairs: &'a Pairs, key: &str) -> &'a str {
        &pairs.iter().find(|(k, _)| k == key).unwrap().1
    }

    #[test]
    fn frontmatter_split() {
        let (front, body) = split_frontmatter("---\nname: x\n---\n# Title\n");
        assert_eq!(front, Some("name: x\n"));
        assert_eq!(body, "# Title\n");
        assert_eq!(split_frontmatter("# Title\n"), (None, "# Title\n"));
        assert_eq!(split_frontmatter("---\nopen\n").0, None);
    }

    #[test]
    fn nested_maps_lists_and_scalars() {
        let src = "\
name: demo # trailing
description: >
  Folded text
  over lines.
allowed-tools: [Bash, Read]
tools:
- a
- b
wask:
  dir: file
  runners:
    py: \"python3 -u\"
  env:
    GREETING: 'it''s # fine'
metadata:
  wask.section: Tasks
literal: |
  one
  two
";
        let p = parse(src).unwrap();
        assert_eq!(get(&p, "name"), "demo");
        assert_eq!(get(&p, "description"), "Folded text over lines.");
        assert_eq!(get(&p, "allowed-tools"), "Bash, Read");
        assert_eq!(get(&p, "tools"), "a, b");
        assert_eq!(get(&p, "wask.dir"), "file");
        assert_eq!(get(&p, "wask.runners.py"), "python3 -u");
        assert_eq!(get(&p, "wask.env.GREETING"), "it's # fine");
        assert_eq!(get(&p, "metadata.wask.section"), "Tasks");
        assert_eq!(get(&p, "literal"), "one\ntwo");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("just some words").is_err());
        assert!(parse("- orphan").is_err());
    }
}
