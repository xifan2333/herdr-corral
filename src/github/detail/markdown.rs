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
pub struct Page {
    lines: Vec<Line<'static>>,
    images: Vec<ImagePlacement>,
}

impl Page {
    pub(crate) const fn new() -> Self {
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

struct MarkdownRenderer<'a> {
    palette: &'a Palette,
    width: usize,
    out: Vec<Line<'static>>,
    images: Vec<(usize, String, String)>,
    segs: Vec<(String, Style)>,
    link_url: Option<String>,
    modifiers: Modifier,
    link: bool,
    heading: Option<HeadingLevel>,
    quote_depth: usize,
    in_code: bool,
    list_stack: Vec<Option<u64>>,
    item_pending_marker: Option<String>,
    image_url: Option<String>,
}

impl<'a> MarkdownRenderer<'a> {
    const fn new(width: usize, palette: &'a Palette) -> Self {
        Self {
            palette,
            width,
            out: Vec::new(),
            images: Vec::new(),
            segs: Vec::new(),
            link_url: None,
            modifiers: Modifier::empty(),
            link: false,
            heading: None,
            quote_depth: 0,
            in_code: false,
            list_stack: Vec::new(),
            item_pending_marker: None,
            image_url: None,
        }
    }

    fn seg_style(&self) -> Style {
        if self.heading.is_some() {
            return Style::default()
                .fg(self.palette.accent)
                .add_modifier(Modifier::BOLD);
        }
        let mut style = if self.quote_depth > 0 {
            Style::default()
                .fg(self.palette.subtext0)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default().fg(self.palette.text)
        };
        style = style.add_modifier(self.modifiers);
        if self.link {
            style = style
                .fg(self.palette.blue)
                .add_modifier(Modifier::UNDERLINED);
        }
        style
    }

    fn quote_prefix(&self) -> String {
        "▏ ".repeat(self.quote_depth)
    }

    fn flush_block_with_prefix(&mut self) {
        let prefix = self.quote_prefix();
        let first = self.item_pending_marker.take();
        flush_block(
            &mut self.out,
            &mut self.segs,
            self.width,
            &prefix,
            first.as_deref(),
            self.palette,
        );
    }

    fn handle_tag_start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.segs.clear(),
            Tag::Heading { level, .. } => {
                self.heading = Some(level);
                self.segs.clear();
            }
            Tag::Strong => self.modifiers.insert(Modifier::BOLD),
            Tag::Emphasis => self.modifiers.insert(Modifier::ITALIC),
            Tag::Strikethrough => self.modifiers.insert(Modifier::CROSSED_OUT),
            Tag::Link { dest_url, .. } => {
                self.flush_block_with_prefix();
                self.link = true;
                self.link_url = Some(dest_url.to_string());
            }
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::List(start) => self.list_stack.push(start),
            Tag::Item => {
                let marker = match self.list_stack.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "• ".to_string(),
                };
                self.item_pending_marker = Some(marker);
                self.segs.clear();
            }
            Tag::CodeBlock(_) => {
                flush_block(
                    &mut self.out,
                    &mut self.segs,
                    self.width,
                    "",
                    None,
                    self.palette,
                );
                self.in_code = true;
            }
            Tag::Image { dest_url, .. } => {
                self.image_url = Some(dest_url.to_string());
                self.segs.clear();
            }
            _ => {}
        }
    }

    fn handle_tag_end(&mut self, tag_end: TagEnd) {
        match tag_end {
            TagEnd::Paragraph | TagEnd::Heading(_) => {
                if let Some(url) = take_lone_image_url(&mut self.segs) {
                    push_image_link(&mut self.out, &mut self.images, url, "", self.palette);
                    self.heading = None;
                    self.out.push(Line::raw(String::new()));
                    return;
                }
                self.flush_block_with_prefix();
                self.heading = None;
                self.out.push(Line::raw(String::new()));
            }
            TagEnd::Strong => self.modifiers.remove(Modifier::BOLD),
            TagEnd::Emphasis => self.modifiers.remove(Modifier::ITALIC),
            TagEnd::Strikethrough => self.modifiers.remove(Modifier::CROSSED_OUT),
            TagEnd::Link => {
                if let Some(url) = self.link_url.take() {
                    if looks_like_image_url(&url) {
                        let label: String =
                            self.segs.iter().map(|(text, _)| text.as_str()).collect();
                        let label = label.trim();
                        if label.is_empty() || label == url {
                            self.segs.clear();
                            push_image_link(&mut self.out, &mut self.images, url, "", self.palette);
                            self.link = false;
                            return;
                        }
                    }
                }
                self.link = false;
            }
            TagEnd::BlockQuote(_) => self.quote_depth = self.quote_depth.saturating_sub(1),
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => self.flush_block_with_prefix(),
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.out.push(Line::raw(String::new()));
            }
            TagEnd::Image => {
                let alt: String = std::mem::take(&mut self.segs)
                    .into_iter()
                    .map(|(text, _)| text)
                    .collect();
                if let Some(url) = self.image_url.take() {
                    flush_block(
                        &mut self.out,
                        &mut self.segs,
                        self.width,
                        "",
                        None,
                        self.palette,
                    );
                    push_image_link(&mut self.out, &mut self.images, url, &alt, self.palette);
                }
            }
            _ => {}
        }
    }

    fn handle_event(&mut self, event: MdEvent<'_>) {
        match event {
            MdEvent::Start(tag) => self.handle_tag_start(tag),
            MdEvent::End(tag_end) => self.handle_tag_end(tag_end),
            MdEvent::Html(html) | MdEvent::InlineHtml(html) => {
                if let Some((url, alt)) = extract_html_img(&html) {
                    flush_block(
                        &mut self.out,
                        &mut self.segs,
                        self.width,
                        "",
                        None,
                        self.palette,
                    );
                    push_image_link(&mut self.out, &mut self.images, url, &alt, self.palette);
                }
            }
            MdEvent::Text(text) => {
                if self.in_code {
                    for line in text.lines() {
                        self.out.push(Line::styled(
                            format!("  {line}"),
                            Style::default()
                                .fg(self.palette.yellow)
                                .bg(self.palette.surface0),
                        ));
                    }
                } else {
                    let style = self.seg_style();
                    self.segs.push((text.to_string(), style));
                }
            }
            MdEvent::Code(code) => self
                .segs
                .push((code.to_string(), Style::default().fg(self.palette.yellow))),
            MdEvent::SoftBreak => self.segs.push((" ".to_string(), Style::default())),
            MdEvent::HardBreak => {
                flush_block(
                    &mut self.out,
                    &mut self.segs,
                    self.width,
                    "",
                    None,
                    self.palette,
                );
            }
            MdEvent::Rule => {
                flush_block(
                    &mut self.out,
                    &mut self.segs,
                    self.width,
                    "",
                    None,
                    self.palette,
                );
                self.out.push(Line::styled(
                    "─".repeat(self.width),
                    Style::default().fg(self.palette.surface1),
                ));
            }
            MdEvent::TaskListMarker(done) => self.segs.push((
                if done { "[x] ".into() } else { "[ ] ".into() },
                Style::default().fg(self.palette.accent),
            )),
            _ => {}
        }
    }

    fn finish(mut self) -> (Vec<Line<'static>>, Vec<(usize, String, String)>) {
        flush_block(
            &mut self.out,
            &mut self.segs,
            self.width,
            "",
            None,
            self.palette,
        );
        while self.out.last().is_some_and(|line| line.width() == 0) {
            self.out.pop();
        }
        (self.out, self.images)
    }
}

/// Markdown → styled ratatui lines using pulldown-cmark for parsing. Returns
/// the lines and `(line_index, url, alt)` for any images (as text links).
pub fn render_markdown(
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
    let parser = Parser::new_ext(
        text,
        Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_GFM,
    );
    let mut renderer = MarkdownRenderer::new(width, palette);
    for event in parser {
        renderer.handle_event(event);
    }
    renderer.finish()
}

/// Terminal display columns for a string (CJK/emoji = 2, ASCII = 1).
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Split `text` into pieces that each fit within `max_cols` terminal columns.
/// CJK runs have no spaces, so we must break by display width, not by words.
pub fn split_to_width(text: &str, max_cols: usize) -> Vec<String> {
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
pub fn flush_block(
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

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
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
