use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};

#[derive(Clone, Debug)]
pub struct Theme {
    pub highlight: Style,
    pub status: Style,
    pub border: Style,
    pub title: Style,
}

pub fn named(name: &str) -> Theme {
    match name {
        "dark" => Theme {
            highlight: Style::default().bg(Color::Indexed(24)).fg(Color::White),
            status: Style::default().bg(Color::Indexed(236)).fg(Color::Indexed(250)),
            border: Style::default().fg(Color::Indexed(240)),
            title: Style::default()
                .fg(Color::Indexed(75))
                .add_modifier(Modifier::BOLD),
        },
        "light" => Theme {
            highlight: Style::default().bg(Color::Indexed(153)).fg(Color::Black),
            status: Style::default().bg(Color::Indexed(252)).fg(Color::Indexed(236)),
            border: Style::default().fg(Color::Indexed(248)),
            title: Style::default()
                .fg(Color::Indexed(25))
                .add_modifier(Modifier::BOLD),
        },
        _ => Theme {
            // Terminal-native: exactly the pre-theme look.
            highlight: Style::default().add_modifier(Modifier::REVERSED),
            status: Style::default().add_modifier(Modifier::REVERSED),
            border: Style::default(),
            title: Style::default(),
        },
    }
}

/// The one popup chrome every modal shares: themed border + themed title.
pub fn popup_block(title: String, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Span::styled(title, theme.title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_themes_differ_and_unknown_falls_back() {
        let dark = named("dark");
        let light = named("light");
        assert_ne!(dark.highlight, light.highlight);
        assert_eq!(named("nope").status, named("default").status);
    }

    #[test]
    fn dark_theme_uses_explicit_colors() {
        let t = named("dark");
        assert_eq!(t.highlight.bg, Some(Color::Indexed(24)));
    }
}
