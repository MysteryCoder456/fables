pub struct ParsedLine {
    pub depth: usize,
    pub block_type: String,
    pub text: String,
    pub checked: Option<bool>,
    pub protected_id: Option<String>,
}

/// A non-fatal problem noticed while parsing; the parse still produces a
/// best-effort result, but the caller may want the user to confirm it.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseWarning {
    /// A ``` fence opened on `line` (1-based) was never closed, so everything
    /// after it was swallowed into a single code block.
    UnclosedFence { line: usize },
}

pub fn parse_markdown(md: &str) -> Vec<ParsedLine> {
    parse_markdown_checked(md).0
}

pub fn parse_markdown_checked(md: &str) -> (Vec<ParsedLine>, Vec<ParseWarning>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let mut lines = md.lines().enumerate();
    while let Some((idx, raw)) = lines.next() {
        if raw.trim().is_empty() {
            continue;
        }
        let indent = raw.chars().take_while(|c| *c == ' ').count();
        let depth = indent / 2;
        let trimmed = &raw[indent..];

        if let Some(id) = parse_protected_marker(trimmed) {
            out.push(ParsedLine {
                depth,
                block_type: "protected".into(),
                text: String::new(),
                checked: None,
                protected_id: Some(id),
            });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("```") {
            let _lang = rest.trim().to_string();
            let mut body = Vec::new();
            let mut closed = false;
            for (_, l) in lines.by_ref() {
                if l.trim() == "```" {
                    closed = true;
                    break;
                }
                body.push(l);
            }
            if !closed {
                warnings.push(ParseWarning::UnclosedFence { line: idx + 1 });
            }
            out.push(ParsedLine {
                depth,
                block_type: "code".into(),
                text: body.join("\n"),
                checked: None,
                protected_id: None,
            });
            continue;
        }
        let (block_type, text, checked) = classify(trimmed);
        out.push(ParsedLine {
            depth,
            block_type,
            text,
            checked,
            protected_id: None,
        });
    }
    (out, warnings)
}

fn parse_protected_marker(line: &str) -> Option<String> {
    line.strip_prefix("<!--notion:block:")
        .and_then(|rest| rest.strip_suffix("-->"))
        .map(str::to_string)
}

fn classify(line: &str) -> (String, String, Option<bool>) {
    if let Some(rest) = line.strip_prefix("### ") {
        return ("heading_3".into(), rest.into(), None);
    }
    if let Some(rest) = line.strip_prefix("## ") {
        return ("heading_2".into(), rest.into(), None);
    }
    if let Some(rest) = line.strip_prefix("# ") {
        return ("heading_1".into(), rest.into(), None);
    }
    if let Some(rest) = line.strip_prefix("- [x] ") {
        return ("to_do".into(), rest.into(), Some(true));
    }
    if let Some(rest) = line.strip_prefix("- [ ] ") {
        return ("to_do".into(), rest.into(), Some(false));
    }
    if let Some(rest) = line.strip_prefix("> ") {
        return ("quote".into(), rest.into(), None);
    }
    if line == "---" {
        return ("divider".into(), String::new(), None);
    }
    if let Some(rest) = line.strip_prefix("- ") {
        return ("bulleted_list_item".into(), rest.into(), None);
    }
    if let Some(dot) = line.find(". ") {
        if !line[..dot].is_empty() && line[..dot].chars().all(|c| c.is_ascii_digit()) {
            return ("numbered_list_item".into(), line[dot + 2..].into(), None);
        }
    }
    ("paragraph".into(), line.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings_todo_and_bullets() {
        let lines = parse_markdown("# Title\n- [x] done\n- [ ] pending\n- bullet");
        assert_eq!(lines[0].block_type, "heading_1");
        assert_eq!(lines[0].text, "Title");
        assert_eq!(lines[1].block_type, "to_do");
        assert_eq!(lines[1].checked, Some(true));
        assert_eq!(lines[2].checked, Some(false));
        assert_eq!(lines[3].block_type, "bulleted_list_item");
        assert_eq!(lines[3].text, "bullet");
    }

    #[test]
    fn parses_numbered_list_regardless_of_number() {
        let lines = parse_markdown("1. first\n7. second");
        assert_eq!(lines[0].block_type, "numbered_list_item");
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[1].text, "second");
    }

    #[test]
    fn indentation_maps_to_depth() {
        let lines = parse_markdown("- parent\n  - child");
        assert_eq!(lines[0].depth, 0);
        assert_eq!(lines[1].depth, 1);
    }

    #[test]
    fn protected_marker_is_recognized() {
        let lines = parse_markdown("<!--notion:block:t1-->\n- after");
        assert_eq!(lines[0].protected_id.as_deref(), Some("t1"));
        assert_eq!(lines[1].protected_id, None);
    }

    #[test]
    fn fenced_code_block_is_one_line_with_full_body() {
        let lines = parse_markdown("```rust\nlet x = 1;\nlet y = 2;\n```\n- after");
        assert_eq!(lines[0].block_type, "code");
        assert_eq!(lines[0].text, "let x = 1;\nlet y = 2;");
        assert_eq!(lines[1].block_type, "bulleted_list_item");
    }
}

#[cfg(test)]
mod fence_tests {
    use super::*;

    #[test]
    fn detects_unclosed_fence_with_line_number() {
        let md = "hello\n```rust\nlet x = 1;";
        let (lines, warnings) = parse_markdown_checked(md);
        assert_eq!(lines.len(), 2); // paragraph + code block
        assert!(matches!(warnings[..], [ParseWarning::UnclosedFence { line: 2 }]));
    }

    #[test]
    fn closed_fence_produces_no_warning() {
        let (_, warnings) = parse_markdown_checked("```\ncode\n```\nafter");
        assert!(warnings.is_empty());
    }
}
