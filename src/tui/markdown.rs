//! Markdown as styled screen lines.
//!
//! Fenced code blocks are cut out and highlighted here; the text between
//! them is rendered piece by piece. So a list that holds a code block is
//! rendered as two lists, and a link defined on the other side of a code
//! block is not found.

use std::sync::LazyLock;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::parsing::SyntaxSet;
use tui_markdown::StyleSheet;
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

use super::{Break, wrap};

const CODE_THEME: EmbeddedThemeName = EmbeddedThemeName::Base16OceanDark;

/// Loaded at the first code block, not at startup.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
static THEMES: LazyLock<EmbeddedLazyThemeSet> = LazyLock::new(two_face::theme::extra);

/// Markdown `text` as styled screen lines no wider than `width`. Text
/// breaks at spaces; code is cut at the width, so its layout holds.
pub(super) fn lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut sections = Vec::new();
    // Where the text after the last code block starts.
    let mut prose_start = 0;
    // The language and the code of the block being read.
    let mut block = None;

    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            // A block still open, as while the reply streams, runs to the
            // end of the text.
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language))) => {
                sections.push(prose_lines(&text[prose_start..range.start], width));
                prose_start = range.end;
                block = Some((language, String::new()));
            }
            Event::Text(part) => {
                if let Some((_, code)) = &mut block {
                    code.push_str(&part);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, code)) = block.take() {
                    sections.push(code_lines(&language, &code, width));
                }
            }
            _ => {}
        }
    }
    sections.push(prose_lines(&text[prose_start..], width));

    sections.retain(|section| !section.is_empty());
    sections.join(&Line::default())
}

/// How markdown looks. Colours only: a background behind text shows as
/// ragged bars, since it ends where each line does.
#[derive(Clone)]
struct Styles;

impl StyleSheet for Styles {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().cyan().bold().underlined(),
            2 => Style::new().cyan().bold(),
            _ => Style::new().cyan().italic(),
        }
    }

    /// No `#` before a heading: its style marks it.
    fn heading_marker(&self, _level: u8) -> &str {
        ""
    }

    fn code(&self) -> Style {
        Style::new().green()
    }

    /// No fence lines around a code block: its colour marks it.
    fn code_block_fence(&self) -> &str {
        ""
    }
}

/// Everything but fenced code blocks.
fn prose_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let options = tui_markdown::Options::new(Styles).table_width(width as u16);
    tui_markdown::from_str_with_options(text, &options)
        .lines
        .iter()
        .flat_map(|line| wrap(line, width, Break::AtSpaces))
        .collect()
}

/// `code` highlighted as `language`, or in one colour if it is not known.
fn code_lines(language: &str, code: &str, width: usize) -> Vec<Line<'static>> {
    // A fence may say more than the language, as in `rust,ignore`.
    let language = language.split([' ', ',']).next().unwrap_or_default();
    let Some(syntax) = SYNTAXES.find_syntax_by_token(language) else {
        return code
            .lines()
            .flat_map(|line| wrap(&Line::styled(line, Styles.code()), width, Break::Anywhere))
            .collect();
    };

    let mut highlighter = HighlightLines::new(syntax, THEMES.get(CODE_THEME));
    let mut lines = Vec::new();
    for line in code.split_inclusive('\n') {
        // A line the grammar fails on is shown plain.
        let parts = highlighter
            .highlight_line(line, &SYNTAXES)
            .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);
        let spans = parts.into_iter().map(|(style, text)| {
            let colour = style.foreground;
            let colour = Color::Rgb(colour.r, colour.g, colour.b);
            Span::styled(text.trim_end_matches('\n'), Style::new().fg(colour))
        });
        lines.extend(wrap(&Line::from_iter(spans), width, Break::Anywhere));
    }
    lines
}
