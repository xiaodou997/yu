#![forbid(unsafe_code)]
//! Shared metadata lexing for the document equation index and native helper.
//! Ranges refer to the original UTF-8 expression. No macro expansion, layout,
//! file access or source rewriting is performed by this crate.
use std::ops::Range;

#[derive(Clone, Debug)]
pub enum Command {
    Label(String),
    Tag { text: String, plain: bool },
    Reference { label: String, parentheses: bool },
    Suppress,
}
#[derive(Clone, Debug)]
pub struct Control {
    pub range: Range<usize>,
    pub command: Command,
}
// MiTeX's bundled scope currently ignores these commands. Reject them rather
// than producing a successful vector with missing semantic content. A helper
// contract test audits this list against the vendored scope on upgrades.
const UNSUPPORTED_COMMANDS: &[&str] = &[
    "@ifstar",
    "AtBeginDocument",
    "AtEndDocument",
    "AtEndOfClass",
    "AtEndOfPackage",
    "CheckCommand",
    "CurrentOption",
    "DeclareOption",
    "DeclareRobustCommand",
    "DeclareTextCommand",
    "DeclareTextCommandDefault",
    "ExecuteOptions",
    "Huge",
    "IfFileExists",
    "InputIfFileExists",
    "LARGE",
    "Large",
    "LoadClass",
    "LoadClassWithOptions",
    "PassOptionsToClass",
    "PassOptionsToPackage",
    "ProcessOptions",
    "ProvideTextCommand",
    "ProvideTextCommandDefault",
    "ProvidesClass",
    "ProvidesFile",
    "RequirePackage",
    "RequirePackageWithOptions",
    "begingroup",
    "caption",
    "centering",
    "cr",
    "def",
    "documentclass",
    "edef",
    "expandafter",
    "footnotesize",
    "gdef",
    "hline",
    "hskip",
    "huge",
    "if",
    "ifdim",
    "iffalse",
    "ifhbox",
    "ifhmode",
    "ifinner",
    "ifmmode",
    "ifnum",
    "ifodd",
    "iftrue",
    "ifvbox",
    "ifvmode",
    "ifvoid",
    "ifx",
    "ignorespaces",
    "ignorespacesafterend",
    "includegraphics",
    "item",
    "kern",
    "large",
    "let",
    "mathstrut",
    "mkern",
    "mskip",
    "newcommand",
    "newcounter",
    "newenvironment",
    "newfont",
    "newlength",
    "newsavebox",
    "newtheorem",
    "nobreak",
    "noexpand",
    "normalsize",
    "providecommand",
    "renewcommand",
    "renewenvironment",
    "scriptsize",
    "small",
    "tiny",
    "vline",
    "xdef",
];

/// Only document metadata commands are handled here. All other TeX is passed
/// to MiTeX unchanged and receives its normal strict syntax diagnostics.
pub fn parse_document_controls(text: &str) -> Result<Vec<Control>, String> {
    parse_controls_inner(text, false)
}
fn parse_controls_inner(text: &str, in_tag: bool) -> Result<Vec<Control>, String> {
    let bytes = text.as_bytes();
    let mut controls = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'@') {
            i += 1;
        }
        if i == name_start {
            i = (i + 1).min(bytes.len());
            continue;
        }
        let name = &text[name_start..i];
        if UNSUPPORTED_COMMANDS.contains(&name) {
            return Err(format!("Unsupported TeX command: \\{name}"));
        }
        if in_tag
            && matches!(
                name,
                "label" | "tag" | "ref" | "eqref" | "notag" | "nonumber"
            )
        {
            return Err("Equation tags cannot contain labels or references".into());
        }
        if matches!(name, "notag" | "nonumber") {
            controls.push(Control {
                range: start..i,
                command: Command::Suppress,
            });
            continue;
        }
        if !matches!(name, "label" | "tag" | "ref" | "eqref") {
            continue;
        }
        let plain = name == "tag" && bytes.get(i) == Some(&b'*');
        if plain {
            i += 1;
        }
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if bytes.get(i) != Some(&b'{') {
            return Err(format!("Missing argument for \\{name}"));
        }
        i += 1;
        let content_start = i;
        let mut depth = 1usize;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'\\' => {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                i += 1;
            }
        }
        if depth != 0 {
            return Err(format!("Unclosed argument for \\{name}"));
        }
        let value = text[content_start..i].trim().to_owned();
        i += 1;
        if value.is_empty() {
            return Err(format!("Empty argument for \\{name}"));
        }
        if name != "tag"
            && value
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '{' | '}' | '\\'))
        {
            return Err(format!("Invalid equation label: {value}"));
        }
        let command = match name {
            "label" => Command::Label(value),
            "tag" => {
                if !parse_controls_inner(&value, true)?.is_empty() {
                    return Err("Equation tags cannot contain labels or references".into());
                }
                Command::Tag { text: value, plain }
            }
            _ => Command::Reference {
                label: value,
                parentheses: name == "eqref",
            },
        };
        controls.push(Control {
            range: start..i,
            command,
        });
    }
    Ok(controls)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_and_commented_commands_are_not_metadata() {
        let source = "\\\\label{literal} % \\ref{comment}\nx + y";
        assert!(parse_document_controls(source).expect("literal").is_empty());
        let source = r"x\label{中文:a}\tag*{A}\eqref{中文:a}";
        let controls = parse_document_controls(source).expect("metadata");
        assert_eq!(controls.len(), 3);
        assert_eq!(&source[controls[0].range.clone()], r"\label{中文:a}");
        assert!(matches!(&controls[1].command, Command::Tag { text, plain: true } if text == "A"));
    }
    #[test]
    fn nested_tags_are_bounded_and_malformed_metadata_is_rejected() {
        let nested = format!("{}x{}", r"\tag{".repeat(10_000), "}".repeat(10_000));
        assert!(parse_document_controls(&nested).is_err());
        for source in [
            r"\label",
            r"\label{a",
            r"\label{a{b}}",
            r"\tag{\ref{x}}",
            r"\newcommand{\x}{1}",
        ] {
            assert!(parse_document_controls(source).is_err(), "{source}");
        }
    }
}
