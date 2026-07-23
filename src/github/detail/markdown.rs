//! Markdown → ratatui lines for GitHub issue/PR bodies and comments.

use super::images::{
    extract_html_img, looks_like_image_url, push_image_link, take_lone_image_url, ImagePlacement,
};
use crate::ui::Palette;
use pulldown_cmark::{Event as MdEvent, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A rendered page: styled lines plus any image placements (by line index).
pub(crate) struct Page {
    lines: Vec<Line<'static>>,
    images: Vec<ImagePlacement>,
}

impl Page {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            images: Vec::new(),
        }
    }

    pub(crate) fn blank(&mut self) {
        self.lines.push(Line::raw(String::new()));
    }

    pub(crate) fn push(&mut self, line: Line<'static>) {
        self.lines.push(line);
    }

    pub(crate) fn rule(&mut self, width: usize, palette: &Palette) {
        self.lines.push(Line::styled(
            "─".repeat(width),
            Style::default().fg(palette.surface1),
        ));
    }

    pub(crate) fn markdown(&mut self, text: &str, width: usize, palette: &Palette) {
        let (lines, images) = render_markdown(text, width, palette);
        let base = self.lines.len();
        for (offset, url, alt) in images {
            self.images.push(ImagePlacement {
                line: base + offset,
                url,
                alt,
            });
        }
        self.lines.extend(lines);
    }

    pub(crate) fn into_parts(self) -> (Vec<Line<'static>>, Vec<ImagePlacement>) {
        (self.lines, self.images)
    }
}

/// Markdown → styled ratatui lines using pulldown-cmark for parsing. Returns
/// the lines and `(line_index, url, alt)` for any images (as text links).
pub(crate) fn render_markdown(
    text: &str,
    width: usize,
    palette: &Palette,
) -> (Vec<Line<'static>>, Vec<(usize, String, String)>) {
    if text.trim().is_empty() {
        return (
            vec![Line::styled(
                "(no description)",
                Style::default().fg(palette.overlay1),
            )],
            Vec::new(),
        );
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut images: Vec<(usize, String, String)> = Vec::new();
    let mut segs: Vec<(String, Style)> = Vec::new();
    let mut link_url: Option<String> = None;
    let (mut bold, mut italic, mut strike, mut link) = (false, false, false, false);
    let mut heading: Option<HeadingLevel> = None;
    let mut quote_depth: usize = 0;
    let mut in_code = false;
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut item_pending_marker: Option<String> = None;
    let mut image_url: Option<String> = None;

    let seg_style = |bold: bool,
                     italic: bool,
                     strike: bool,
                     link: bool,
                     heading: Option<HeadingLevel>,
                     quote: usize| {
        if let Some(_level) = heading {
            return Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD);
        }
        let mut style = if quote > 0 {
            Style::default()
                .fg(palette.subtext0)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(palette.text)
        };
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if strike {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if link {
            style = style.fg(palette.blue).add_modifier(Modifier::UNDERLINED);
        }
        style
    };

    let quote_prefix = |depth: usize| "▏ ".repeat(depth);

    // ENABLE_GFM turns bare `https://...` lines into autolinks so GitHub
    // attachment URLs can be promoted to image rows.
    let parser = Parser::new_ext(
        text,
        Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_GFM,
    );
    for event in parser {
        match event {
            MdEvent::Start(Tag::Paragraph | Tag::Heading { .. }) => {
                if let Tag::Heading { level, .. } = event_heading(&event) {
                    heading = Some(level);
                }
                segs.clear();
            }
            MdEvent::End(TagEnd::Paragraph | TagEnd::Heading(_)) => {
                // GitHub often pastes a bare attachment URL as its own paragraph.
                // pulldown-cmark keeps those as plain text, so promote them here.
                if let Some(url) = take_lone_image_url(&mut segs) {
                    push_image_link(&mut out, &mut images, url, "", palette);
                    heading = None;
                    out.push(Line::raw(String::new()));
                    continue;
                }
                let prefix = quote_prefix(quote_depth);
                let first = item_pending_marker.take();
                flush_block(
                    &mut out,
                    &mut segs,
                    width,
                    &prefix,
                    first.as_deref(),
                    palette,
                );
                heading = None;
                out.push(Line::raw(String::new()));
            }
            MdEvent::Start(Tag::Strong) => bold = true,
            MdEvent::End(TagEnd::Strong) => bold = false,
            MdEvent::Start(Tag::Emphasis) => italic = true,
            MdEvent::End(TagEnd::Emphasis) => italic = false,
            MdEvent::Start(Tag::Strikethrough) => strike = true,
            MdEvent::End(TagEnd::Strikethrough) => strike = false,
            MdEvent::Start(Tag::Link { dest_url, .. }) => {
                // Flush preceding text so an image-looking href cannot swallow
                // the rest of the paragraph as "alt".
                let prefix = quote_prefix(quote_depth);
                let first = item_pending_marker.take();
                flush_block(
                    &mut out,
                    &mut segs,
                    width,
                    &prefix,
                    first.as_deref(),
                    palette,
                );
                link = true;
                link_url = Some(dest_url.to_string());
            }
            MdEvent::End(TagEnd::Link) => {
                // Promote only when the link body is empty/equal to the URL
                // (autolink / bare attachment). Labeled links stay as links.
                if let Some(url) = link_url.take() {
                    if looks_like_image_url(&url) {
                        let label: String = segs.iter().map(|(text, _)| text.as_str()).collect();
                        let label = label.trim();
                        if label.is_empty() || label == url {
                            segs.clear();
                            push_image_link(&mut out, &mut images, url, "", palette);
                            link = false;
                            continue;
                        }
                    }
                }
                link = false;
            }
            MdEvent::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            MdEvent::End(TagEnd::BlockQuote(_)) => quote_depth = quote_depth.saturating_sub(1),
            MdEvent::Start(Tag::List(start)) => list_stack.push(start),
            MdEvent::End(TagEnd::List(_)) => {
                list_stack.pop();
            }
            MdEvent::Start(Tag::Item) => {
                let marker = match list_stack.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "• ".to_string(),
                };
                item_pending_marker = Some(marker);
                segs.clear();
            }
            MdEvent::End(TagEnd::Item) => {
                let prefix = quote_prefix(quote_depth);
                let first = item_pending_marker.take();
                flush_block(
                    &mut out,
                    &mut segs,
                    width,
                    &prefix,
                    first.as_deref(),
                    palette,
                );
            }
            MdEvent::Start(Tag::CodeBlock(_)) => {
                flush_block(&mut out, &mut segs, width, "", None, palette);
                in_code = true;
            }
            MdEvent::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push(Line::raw(String::new()));
            }
            MdEvent::Start(Tag::Image { dest_url, .. }) => {
                image_url = Some(dest_url.to_string());
                segs.clear();
            }
            MdEvent::End(TagEnd::Image) => {
                let alt: String = std::mem::take(&mut segs)
                    .into_iter()
                    .map(|(text, _)| text)
                    .collect();
                if let Some(url) = image_url.take() {
                    flush_block(&mut out, &mut segs, width, "", None, palette);
                    push_image_link(&mut out, &mut images, url, &alt, palette);
                }
            }
            // GitHub comments often paste raw HTML <img> tags instead of
            // markdown images. Capture both block and inline forms.
            MdEvent::Html(html) | MdEvent::InlineHtml(html) => {
                if let Some((url, alt)) = extract_html_img(&html) {
                    flush_block(&mut out, &mut segs, width, "", None, palette);
                    push_image_link(&mut out, &mut images, url, &alt, palette);
                }
            }
            MdEvent::Text(text) => {
                if in_code {
                    for line in text.lines() {
                        out.push(Line::styled(
                            format!("  {line}"),
                            Style::default().fg(palette.yellow).bg(palette.surface0),
                        ));
                    }
                } else {
                    segs.push((
                        text.to_string(),
                        seg_style(bold, italic, strike, link, heading, quote_depth),
                    ));
                }
            }
            MdEvent::Code(code) => segs.push((
                code.to_string(),
                Style::default().fg(palette.yellow).bg(palette.surface0),
            )),
            MdEvent::SoftBreak => {
                // Keep an explicit space so flush_block re-joins words.
                segs.push((" ".to_string(), Style::default()));
            }
            MdEvent::HardBreak => {
                flush_block(&mut out, &mut segs, width, "", None, palette);
            }
            MdEvent::Rule => {
                flush_block(&mut out, &mut segs, width, "", None, palette);
                out.push(Line::styled(
                    "─".repeat(width),
                    Style::default().fg(palette.surface1),
                ));
            }
            MdEvent::TaskListMarker(done) => segs.push((
                if done { "[x] ".into() } else { "[ ] ".into() },
                Style::default().fg(palette.accent),
            )),
            _ => {}
        }
    }
    flush_block(&mut out, &mut segs, width, "", None, palette);
    while out.last().is_some_and(|line| line.width() == 0) {
        out.pop();
    }
    (out, images)
}

pub(crate) fn event_heading(event: &MdEvent) -> Tag<'static> {
    if let MdEvent::Start(Tag::Heading { level, .. }) = event {
        Tag::Heading {
            level: *level,
            id: None,
            classes: Vec::new(),
            attrs: Vec::new(),
        }
    } else {
        Tag::Paragraph
    }
}

/// Terminal display columns for a string (CJK/emoji = 2, ASCII = 1).
pub(crate) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Split `text` into pieces that each fit within `max_cols` terminal columns.
/// CJK runs have no spaces, so we must break by display width, not by words.
pub(crate) fn split_to_width(text: &str, max_cols: usize) -> Vec<String> {
    let max_cols = max_cols.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    if display_width(text) <= max_cols {
        return vec![text.to_string()];
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 0 {
            // Combining marks stick to the previous glyph.
            current.push(ch);
            continue;
        }
        if !current.is_empty() && used + w > max_cols {
            parts.push(std::mem::take(&mut current));
            used = 0;
        }
        // A single wide glyph wider than the line still has to go somewhere.
        current.push(ch);
        used += w;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Wrap styled segments into lines, applying a first-line marker + continuation
/// indent so list bullets and blockquote bars align.
///
/// Width is measured in *terminal columns* (unicode-width), not Unicode scalar
/// counts. Without that, CJK paragraphs overflow and the host terminal rewraps
/// mid-glyph, which looks like broken markdown / tofu blocks.
pub(crate) fn flush_block(
    out: &mut Vec<Line<'static>>,
    segs: &mut Vec<(String, Style)>,
    width: usize,
    prefix: &str,
    first_marker: Option<&str>,
    palette: &Palette,
) {
    if segs.is_empty() {
        return;
    }
    let marker = first_marker.unwrap_or("");
    let cont_indent = " ".repeat(display_width(marker));
    let prefix_style = Style::default().fg(palette.overlay1);
    let first_avail = width
        .saturating_sub(display_width(prefix) + display_width(marker))
        .max(1);
    let cont_avail = width
        .saturating_sub(display_width(prefix) + display_width(&cont_indent))
        .max(1);

    // Split segments into styled atoms: whitespace-separated words for Latin
    // text, plus hard display-width chunks for long CJK/code runs.
    let mut atoms: Vec<(String, Style)> = Vec::new();
    for (text, style) in segs.drain(..) {
        if text.chars().all(char::is_whitespace) {
            if atoms.last().is_some_and(|(word, _)| !word.is_empty()) {
                atoms.push((String::new(), style));
            }
            continue;
        }
        for (index, word) in text.split(' ').enumerate() {
            if word.is_empty() {
                continue;
            }
            if index > 0 {
                atoms.push((String::new(), style));
            }
            // Break overlong tokens (CJK paragraphs, long paths, code) so a
            // single atom never exceeds the continuation line budget.
            for part in split_to_width(word, cont_avail.max(first_avail)) {
                atoms.push((part, style));
            }
        }
    }
    while atoms.last().is_some_and(|(word, _)| word.is_empty()) {
        atoms.pop();
    }
    if atoms.is_empty() {
        return;
    }

    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut on_first = true;
    for (word, style) in atoms {
        let avail = if on_first { first_avail } else { cont_avail };
        if word.is_empty() {
            if !current.is_empty() && used < avail {
                current.push(Span::raw(" "));
                used += 1;
            }
            continue;
        }
        let word_len = display_width(&word);
        let needs_gap = current
            .last()
            .is_some_and(|span| !span.content.ends_with(' '));
        let extra = usize::from(needs_gap) + word_len;
        if !current.is_empty() && used + extra > avail {
            rows.push(std::mem::take(&mut current));
            used = 0;
            on_first = false;
        }
        if !current.is_empty()
            && current
                .last()
                .is_some_and(|span| !span.content.ends_with(' '))
        {
            current.push(Span::raw(" "));
            used += 1;
        }
        current.push(Span::styled(word, style));
        used += word_len;
    }
    if !current.is_empty() {
        rows.push(current);
    }

    for (index, mut spans) in rows.into_iter().enumerate() {
        let mut line_spans = Vec::new();
        if !prefix.is_empty() {
            line_spans.push(Span::styled(prefix.to_string(), prefix_style));
        }
        if index == 0 && !marker.is_empty() {
            line_spans.push(Span::styled(marker.to_string(), prefix_style));
        } else if !cont_indent.is_empty() {
            line_spans.push(Span::raw(cont_indent.clone()));
        }
        line_spans.append(&mut spans);
        out.push(Line::from(line_spans));
    }
}

pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut output = Vec::new();
    for raw in text.lines() {
        if raw.is_empty() {
            output.push(String::new());
            continue;
        }
        // Prefer word breaks for Latin; fall back to display-width chunks for
        // long CJK/code tokens that have no spaces.
        let mut line = String::new();
        for word in raw.split_whitespace() {
            for part in split_to_width(word, width) {
                let gap = usize::from(!line.is_empty());
                let extra = gap + display_width(&part);
                if !line.is_empty() && display_width(&line) + extra > width {
                    output.push(std::mem::take(&mut line));
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(&part);
            }
        }
        if !line.is_empty() {
            output.push(line);
        }
    }
    output
}
