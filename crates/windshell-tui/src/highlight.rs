use anyhow::Result;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::{path::Path, sync::LazyLock};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};
static SYNTAX: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
pub fn code(path: &Path, text: &str) -> Result<Vec<Line<'static>>> {
    let syntax = SYNTAX
        .find_syntax_by_extension(path.extension().and_then(|s| s.to_str()).unwrap_or(""))
        .or_else(|| SYNTAX.find_syntax_by_first_line(text.lines().next().unwrap_or("")))
        .unwrap_or_else(|| SYNTAX.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, &THEMES.themes["base16-ocean.dark"]);
    LinesWithEndings::from(text)
        .map(|line| {
            let mut spans = vec![];
            for (style, value) in highlighter.highlight_line(line, &SYNTAX)? {
                let c = style.foreground;
                let mut style2 = Style::default().fg(Color::Rgb(c.r, c.g, c.b));
                if style.font_style.contains(FontStyle::BOLD) {
                    style2 = style2.add_modifier(Modifier::BOLD);
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    style2 = style2.add_modifier(Modifier::ITALIC);
                }
                spans.push(Span::styled(
                    value.trim_end_matches('\n').replace('\t', "    "),
                    style2,
                ));
            }
            Ok(Line::from(spans))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rust_keywords_strings_and_multiline_comments_have_distinct_styles() -> Result<()> {
        let lines = code(
            Path::new("main.rs"),
            "fn main() {\n /* comment\n still comment */ let x = \"hello\";\n}\n",
        )?;
        assert_eq!(lines.len(), 4);
        let styles: std::collections::HashSet<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.style.fg))
            .collect();
        assert!(styles.len() >= 3);
        assert!(
            lines[2]
                .spans
                .iter()
                .any(|s| s.content.contains("still comment"))
        );
        Ok(())
    }
}
