use std::collections::HashSet;

use notion_store::{BlockRec, PageRec};
use once_cell::sync::Lazy;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

use crate::ui::theme::Theme;

static SYNTAXES: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);
static THEMES: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);

fn highlight_code_line(line: &str, language: &str) -> Line<'static> {
    let syntax = SYNTAXES
        .find_syntax_by_token(language)
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let theme = &THEMES.themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);
    let spans: Vec<Span<'static>> = h
        .highlight_line(line, &SYNTAXES)
        .unwrap_or_default()
        .into_iter()
        .map(|(style, text)| {
            Span::styled(
                text.to_string(),
                ratatui::style::Style::default().fg(Color::Rgb(
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                )),
            )
        })
        .collect();
    Line::from(spans)
}

fn caption_text(payload: &serde_json::Value) -> String {
    payload["caption"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t["plain_text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

pub struct BlockLine {
    pub block_id: String,
    pub text: String,
    pub indent: usize,
    pub link_page_id: Option<String>,
    pub spans: Option<Line<'static>>,
}

pub struct PageView {
    pub page: PageRec,
    pub blocks: Vec<BlockRec>,
    pub cursor: usize,
    pub collapsed_toggles: HashSet<String>,
    pub list_state: ListState,
}

impl PageView {
    pub fn new(page: PageRec, blocks: Vec<BlockRec>) -> PageView {
        PageView {
            page,
            blocks,
            cursor: 0,
            collapsed_toggles: HashSet::new(),
            list_state: ListState::default(),
        }
    }

    pub fn lines(&self) -> Vec<BlockLine> {
        let mut out = Vec::new();
        self.push_children(None, 0, &mut out);
        out
    }

    fn push_children(&self, parent: Option<&str>, indent: usize, out: &mut Vec<BlockLine>) {
        let mut numbered = 0usize;
        let siblings: Vec<&BlockRec> = self
            .blocks
            .iter()
            .filter(|b| b.parent_block_id.as_deref() == parent)
            .collect();
        for b in siblings {
            if b.block_type == "numbered_list_item" {
                numbered += 1;
            } else {
                numbered = 0;
            }
            let payload: serde_json::Value = serde_json::from_str(&b.payload).unwrap_or_default();
            let collapsed = self.collapsed_toggles.contains(&b.id);
            let (text, link) = match b.block_type.as_str() {
                "heading_1" => (format!("# {}", b.plain_text), None),
                "heading_2" => (format!("## {}", b.plain_text), None),
                "heading_3" => (format!("### {}", b.plain_text), None),
                "to_do" => {
                    let mark = if payload["checked"].as_bool().unwrap_or(false) {
                        "x"
                    } else {
                        " "
                    };
                    (format!("[{mark}] {}", b.plain_text), None)
                }
                "bulleted_list_item" => (format!("• {}", b.plain_text), None),
                "numbered_list_item" => (format!("{numbered}. {}", b.plain_text), None),
                "toggle" => {
                    let arrow = if collapsed { "▸" } else { "▾" };
                    (format!("{arrow} {}", b.plain_text), None)
                }
                "quote" => (format!("┃ {}", b.plain_text), None),
                "callout" => (format!("💡 {}", b.plain_text), None),
                "divider" => ("────────".to_string(), None),
                "child_page" | "child_database" => (format!("→ {}", b.plain_text), Some(b.id.clone())),
                "paragraph" => (b.plain_text.clone(), None),
                _ if b.block_type == "code" => {
                    let language = payload["language"].as_str().unwrap_or("").to_string();
                    for src_line in b.plain_text.lines() {
                        out.push(BlockLine {
                            block_id: b.id.clone(),
                            text: format!("│ {src_line}"),
                            indent,
                            link_page_id: None,
                            spans: Some(highlight_code_line(src_line, &language)),
                        });
                    }
                    continue;
                }
                "image" => {
                    let caption = caption_text(&payload);
                    let caption = if caption.is_empty() {
                        "untitled".to_string()
                    } else {
                        caption
                    };
                    (format!("[image: {caption}]"), None)
                }
                "bookmark" => (
                    format!("[bookmark: {}]", payload["url"].as_str().unwrap_or("")),
                    None,
                ),
                "embed" => (
                    format!("[embed: {}]", payload["url"].as_str().unwrap_or("")),
                    None,
                ),
                _ => (format!("[{}]", b.block_type), None),
            };
            out.push(BlockLine {
                block_id: b.id.clone(),
                text,
                indent,
                link_page_id: link,
                spans: None,
            });
            if !(b.block_type == "toggle" && collapsed) {
                self.push_children(Some(&b.id), indent + 1, out);
            }
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.lines().len();
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn toggle_at_cursor(&mut self) {
        if let Some(line) = self.lines().get(self.cursor) {
            let id = line.block_id.clone();
            let is_toggle = self.blocks.iter().any(|b| b.id == id && b.block_type == "toggle");
            if is_toggle && !self.collapsed_toggles.remove(&id) {
                self.collapsed_toggles.insert(id);
            }
        }
    }

    pub fn link_at_cursor(&self) -> Option<String> {
        self.lines().get(self.cursor).and_then(|l| l.link_page_id.clone())
    }

    pub fn block_id_at_cursor(&self) -> Option<String> {
        self.lines().get(self.cursor).map(|l| l.block_id.clone())
    }

    pub fn todo_block_at_cursor(&self) -> Option<String> {
        let id = self.block_id_at_cursor()?;
        self.blocks
            .iter()
            .any(|b| b.id == id && b.block_type == "to_do")
            .then_some(id)
    }

    pub fn child_database_id_at_cursor(&self) -> Option<String> {
        let id = self.block_id_at_cursor()?;
        self.blocks
            .iter()
            .any(|b| b.id == id && b.block_type == "child_database")
            .then_some(id)
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &mut PageView, focused: bool, theme: &Theme) {
    let items: Vec<ListItem> = view
        .lines()
        .iter()
        .map(|l| {
            let content = match &l.spans {
                Some(styled) => {
                    let mut spans = vec![Span::raw(format!("{}│ ", "  ".repeat(l.indent)))];
                    spans.extend(styled.spans.iter().cloned());
                    Line::from(spans)
                }
                None => Line::from(format!("{}{}", "  ".repeat(l.indent), l.text)),
            };
            ListItem::new(content)
        })
        .collect();
    let title = format!(" {} ", view.page.title);
    let highlight = if focused {
        theme.highlight
    } else {
        ratatui::style::Style::default()
    };
    view.list_state.select(Some(view.cursor));
    f.render_stateful_widget(
        List::new(items).highlight_style(highlight).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title(Span::styled(title, theme.title)),
        ),
        area,
        &mut view.list_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, parent: Option<&str>, ord: i64, ty: &str, text: &str, payload: &str) -> BlockRec {
        BlockRec {
            id: id.into(),
            page_id: "p".into(),
            parent_block_id: parent.map(Into::into),
            ordinal: ord,
            block_type: ty.into(),
            payload: payload.into(),
            plain_text: text.into(),
            has_children: false,
        }
    }

    fn page() -> PageRec {
        PageRec {
            id: "p".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "T".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        }
    }

    #[test]
    fn renders_prefixes_and_numbering() {
        let v = PageView::new(
            page(),
            vec![
                rec("h", None, 0, "heading_1", "Title", "{}"),
                rec("t1", None, 1, "to_do", "done thing", r#"{"checked": true}"#),
                rec("n1", None, 2, "numbered_list_item", "first", "{}"),
                rec("n2", None, 3, "numbered_list_item", "second", "{}"),
                rec("d", None, 4, "divider", "", "{}"),
            ],
        );
        let texts: Vec<String> = v.lines().iter().map(|l| l.text.clone()).collect();
        assert_eq!(texts[0], "# Title");
        assert_eq!(texts[1], "[x] done thing");
        assert_eq!(texts[2], "1. first");
        assert_eq!(texts[3], "2. second");
        assert!(texts[4].starts_with("────"));
    }

    #[test]
    fn toggle_collapse_hides_children() {
        let mut v = PageView::new(
            page(),
            vec![
                rec("tg", None, 0, "toggle", "More", "{}"),
                rec("c1", Some("tg"), 0, "paragraph", "hidden", "{}"),
            ],
        );
        assert_eq!(v.lines().len(), 2);
        assert_eq!(v.lines()[1].indent, 1);
        v.cursor = 0;
        v.toggle_at_cursor();
        assert_eq!(v.lines().len(), 1);
        assert!(v.lines()[0].text.starts_with("▸"));
    }

    #[test]
    fn child_page_is_a_link() {
        let v = PageView::new(page(), vec![rec("cp", None, 0, "child_page", "Sub page", "{}")]);
        assert_eq!(v.lines()[0].link_page_id.as_deref(), Some("cp"));
        assert_eq!(v.lines()[0].text, "→ Sub page");
    }

    #[test]
    fn render_scrolls_so_cursor_stays_visible() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let blocks: Vec<BlockRec> = (0..30)
            .map(|i| {
                rec(
                    &format!("b{i}"),
                    None,
                    i as i64,
                    "paragraph",
                    &format!("Line {i}"),
                    "{}",
                )
            })
            .collect();
        let mut v = PageView::new(page(), blocks);
        v.cursor = 29;
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(20, 10);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut v, true, &theme)).unwrap();
        let content: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            content.contains("Line 29"),
            "cursor's row should be visible:\n{content}"
        );
        assert!(
            !content.contains("Line 0"),
            "top row should have scrolled out:\n{content}"
        );
    }

    #[test]
    fn code_block_renders_per_line_with_syntax_styling() {
        let v = PageView::new(
            page(),
            vec![rec(
                "c",
                None,
                0,
                "code",
                "let x = 1;\nlet y = 2;",
                r#"{"language": "rust"}"#,
            )],
        );
        let lines = v.lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].text.contains("let x = 1;"));
        let spans = lines[0].spans.as_ref().expect("code lines carry styled spans");
        assert!(spans.spans.iter().any(|s| s.style.fg.is_some()));
    }

    #[test]
    fn image_block_shows_caption_placeholder() {
        let v = PageView::new(
            page(),
            vec![rec(
                "i1",
                None,
                0,
                "image",
                "",
                r#"{"caption": [{"plain_text": "A chart"}]}"#,
            )],
        );
        assert_eq!(v.lines()[0].text, "[image: A chart]");
    }

    #[test]
    fn image_block_without_caption_shows_untitled() {
        let v = PageView::new(page(), vec![rec("i1", None, 0, "image", "", "{}")]);
        assert_eq!(v.lines()[0].text, "[image: untitled]");
    }

    #[test]
    fn bookmark_block_shows_url_placeholder() {
        let v = PageView::new(
            page(),
            vec![rec(
                "b1",
                None,
                0,
                "bookmark",
                "",
                r#"{"url": "https://example.com"}"#,
            )],
        );
        assert_eq!(v.lines()[0].text, "[bookmark: https://example.com]");
    }

    #[test]
    fn table_and_embed_and_other_unsupported_blocks_get_named_placeholders() {
        let v = PageView::new(
            page(),
            vec![
                rec("t1", None, 0, "table", "", "{}"),
                rec("e1", None, 1, "embed", "", r#"{"url": "https://youtu.be/x"}"#),
                rec("v1", None, 2, "video", "", "{}"),
            ],
        );
        let lines = v.lines();
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["[table]", "[embed: https://youtu.be/x]", "[video]"]);
    }
}
