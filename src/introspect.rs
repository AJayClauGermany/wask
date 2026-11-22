//! `--introspect`: the command tree as JSON, for tools and agents.

use crate::cli;
use crate::markdown::Doc;
use crate::model::{Command, Kind, Script, env_name};

fn string(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn optional(s: Option<&str>) -> String {
    s.map_or("null".to_string(), string)
}

fn array(items: impl Iterator<Item = String>) -> String {
    format!("[{}]", items.collect::<Vec<_>>().join(","))
}

/// `path` is the command's parents, then the command itself.
fn command(path: &mut Vec<&Command>) -> String {
    let cmd = *path.last().expect("path ends with the command");
    let args = array(cmd.args.iter().map(|a| {
        format!(
            "{{\"name\":{},\"env\":{},\"required\":{},\"variadic\":{}}}",
            string(&a.name),
            string(&env_name(&a.name)),
            a.required,
            a.variadic
        )
    }));
    let options = array(cmd.options.iter().map(|o| {
        let kind = match o.kind {
            Kind::Bool => "boolean",
            Kind::String => "string",
            Kind::Number => "number",
        };
        format!(
            "{{\"name\":{},\"env\":{},\"short\":{},\"long\":{},\"type\":\"{kind}\",\"choices\":{},\"required\":{},\"description\":{}}}",
            string(&o.name),
            string(&env_name(&o.name)),
            optional(o.short.map(String::from).as_deref()),
            optional(o.long.as_deref()),
            array(o.choices.iter().map(|c| string(c))),
            o.required,
            string(&o.desc)
        )
    }));
    let script = match &cmd.script {
        None => "null".to_string(),
        Some(Script::Inline { lang, source }) => {
            format!(
                "{{\"language\":{},\"source\":{}}}",
                string(lang),
                string(source)
            )
        }
        Some(Script::File(path)) => format!("{{\"file\":{}}}", string(&path.to_string_lossy())),
    };
    let usage = cli::usage(path);
    let subs = array(cmd.subs.iter().map(|sub| {
        path.push(sub);
        let json = command(path);
        path.pop();
        json
    }));
    format!(
        "{{\"name\":{},\"description\":{},\"usage\":{},\"file\":{},\"line\":{},\"dir\":{},\"args\":{args},\"options\":{options},\"script\":{script},\"error\":{},\"subcommands\":{subs}}}",
        string(&cmd.name),
        string(&cmd.desc),
        string(&usage),
        string(&cmd.file.to_string_lossy()),
        cmd.line,
        string(&cmd.dir.to_string_lossy()),
        optional(cmd.conflict.as_deref()),
    )
}

pub fn document(doc: &Doc, file: &std::path::Path) -> String {
    format!(
        "{{\"file\":{},\"name\":{},\"title\":{},\"description\":{},\"commands\":{}}}",
        string(&file.to_string_lossy()),
        optional(doc.name.as_deref()),
        optional(doc.title.as_deref()),
        optional(doc.description.as_deref()),
        array(doc.commands.iter().map(|c| command(&mut vec![c])))
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes() {
        assert_eq!(super::string("a\"b\\\n\u{1}"), "\"a\\\"b\\\\\\n\\u0001\"");
    }
}
