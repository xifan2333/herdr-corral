//! Page builders for issue / PR / run detail tabs.

use super::images::ImagePlacement;
use super::markdown::{wrap_text, Page};
use super::util::{actor, display_or, short_sha, state_color};
use crate::github::{IssueDetail, PullRequestDetail, Review, WorkflowRunDetail};
use crate::ui::Palette;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

pub(crate) fn metadata_line(parts: &[String], palette: &Palette) -> Line<'static> {
    Line::styled(parts.join("  ·  "), Style::default().fg(palette.subtext0))
}

pub(crate) fn comment_header(login: &str, timestamp: &str, palette: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            login.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {timestamp}"),
            Style::default().fg(palette.subtext0),
        ),
    ])
}

/// Issue overview + all comments on one scrollable page.
pub(crate) fn issue_page(
    issue: &IssueDetail,
    width: usize,
    palette: &Palette,
) -> (Vec<Line<'static>>, Vec<ImagePlacement>) {
    let labels = issue
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut page = Page::new();
    page.push(metadata_line(
        &[
            actor(issue.author.as_ref()).to_string(),
            if labels.is_empty() {
                "no labels".into()
            } else {
                labels
            },
            issue.updated_at.clone(),
        ],
        palette,
    ));
    page.blank();
    page.markdown(&issue.body, width, palette);
    for comment in &issue.comments {
        page.blank();
        page.rule(width, palette);
        page.push(comment_header(
            actor(comment.author.as_ref()),
            &comment.created_at,
            palette,
        ));
        page.blank();
        page.markdown(&comment.body, width, palette);
    }
    page.into_parts()
}

/// PR overview + conversation + reviews on one scrollable page.
pub(crate) fn pull_page(
    pull: &PullRequestDetail,
    width: usize,
    palette: &Palette,
) -> (Vec<Line<'static>>, Vec<ImagePlacement>) {
    let mut page = Page::new();
    page.push(metadata_line(
        &[
            actor(pull.author.as_ref()).to_string(),
            format!("{} → {}", pull.head_ref_name, pull.base_ref_name),
            format!("+{} -{}", pull.additions, pull.deletions),
            format!("{} files", pull.changed_files),
        ],
        palette,
    ));
    page.push(metadata_line(
        &[
            format!("review {}", display_or(&pull.review_decision, "pending")),
            format!("merge {}", display_or(&pull.mergeable, "unknown")),
            display_or(&pull.merge_state_status, "unknown").to_string(),
        ],
        palette,
    ));
    page.blank();
    page.markdown(&pull.body, width, palette);
    for comment in &pull.comments {
        page.blank();
        page.rule(width, palette);
        page.push(comment_header(
            actor(comment.author.as_ref()),
            &comment.created_at,
            palette,
        ));
        page.blank();
        page.markdown(&comment.body, width, palette);
    }
    for review in &pull.reviews {
        page.blank();
        page.rule(width, palette);
        page.push(review_header(review, palette));
        if !review.body.is_empty() {
            page.blank();
            page.markdown(&review.body, width, palette);
        }
    }
    page.into_parts()
}

pub(crate) fn review_header(review: &Review, palette: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            actor(review.author.as_ref()).to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(palette.overlay1)),
        Span::styled(
            review.state.clone(),
            Style::default().fg(state_color(&review.state, palette)),
        ),
        Span::styled(
            format!("  ·  {}", review.submitted_at),
            Style::default().fg(palette.subtext0),
        ),
    ])
}

pub(crate) fn pull_files(pull: &PullRequestDetail, palette: &Palette) -> Vec<Line<'static>> {
    if pull.files.is_empty() {
        return vec![Line::styled(
            "No changed files",
            Style::default().fg(palette.overlay1),
        )];
    }
    pull.files
        .iter()
        .map(|file| {
            Line::from(vec![
                Span::styled(
                    format!("+{} ", file.additions),
                    Style::default().fg(palette.green),
                ),
                Span::styled(
                    format!("-{} ", file.deletions),
                    Style::default().fg(palette.red),
                ),
                Span::styled(file.path.clone(), Style::default().fg(palette.text)),
            ])
        })
        .collect()
}

pub(crate) fn patch_lines(patch: &str, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for raw in patch.lines() {
        let style =
            if raw.starts_with("diff --git") || raw.starts_with("+++") || raw.starts_with("---") {
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD)
            } else if raw.starts_with("@@") {
                Style::default().fg(palette.blue)
            } else if raw.starts_with('+') {
                Style::default().fg(palette.green).bg(palette.surface0)
            } else if raw.starts_with('-') {
                Style::default().fg(palette.red).bg(palette.surface0)
            } else {
                Style::default().fg(palette.subtext0)
            };
        for line in wrap_text(raw, width) {
            lines.push(Line::styled(line, style));
        }
    }
    lines
}

pub(crate) fn check_lines(checks: &Value, palette: &Palette) -> Vec<Line<'static>> {
    let Some(checks) = checks.as_array() else {
        return vec![Line::styled(
            "No checks",
            Style::default().fg(palette.overlay1),
        )];
    };
    if checks.is_empty() {
        return vec![Line::styled(
            "No checks",
            Style::default().fg(palette.overlay1),
        )];
    }
    checks
        .iter()
        .map(|check| {
            let state = check
                .get("conclusion")
                .or_else(|| check.get("state"))
                .or_else(|| check.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("pending");
            let name = check
                .get("name")
                .or_else(|| check.get("context"))
                .and_then(Value::as_str)
                .unwrap_or("check");
            let glyph = match state.to_ascii_lowercase().as_str() {
                "success" | "completed" => "✓",
                "failure" | "failed" | "error" | "timed_out" => "×",
                _ => "…",
            };
            Line::from(vec![
                Span::styled(
                    format!("{glyph} "),
                    Style::default().fg(state_color(state, palette)),
                ),
                Span::styled(name.to_string(), Style::default().fg(palette.text)),
                Span::styled(format!("  {state}"), Style::default().fg(palette.subtext0)),
            ])
        })
        .collect()
}

pub(crate) fn run_overview(run: &WorkflowRunDetail, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = vec![
        metadata_line(
            &[
                run.event.clone(),
                run.head_branch.clone(),
                short_sha(&run.head_sha),
                format!("attempt {}", run.attempt),
            ],
            palette,
        ),
        Line::raw(String::new()),
    ];
    lines.extend(
        wrap_text(&run.display_title, width)
            .into_iter()
            .map(|line| {
                Line::styled(
                    line,
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                )
            }),
    );
    lines.push(Line::raw(String::new()));
    lines.push(metadata_line(
        &[run.created_at.clone(), run.updated_at.clone()],
        palette,
    ));
    lines
}

pub(crate) fn run_jobs(run: &WorkflowRunDetail, palette: &Palette) -> Vec<Line<'static>> {
    if run.jobs.is_empty() {
        return vec![Line::styled(
            "No jobs",
            Style::default().fg(palette.overlay1),
        )];
    }
    let mut lines = Vec::new();
    for job in &run.jobs {
        let state = if job.status == "completed" {
            &job.conclusion
        } else {
            &job.status
        };
        let glyph = match state.as_str() {
            "success" => "✓",
            "failure" | "timed_out" => "×",
            "cancelled" => "■",
            _ => "…",
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{glyph} "),
                Style::default().fg(state_color(state, palette)),
            ),
            Span::styled(
                job.name.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for step in &job.steps {
            let state = if step.status == "completed" {
                &step.conclusion
            } else {
                &step.status
            };
            let glyph = if state == "success" {
                "✓"
            } else if state == "failure" {
                "×"
            } else {
                "·"
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("   {glyph} "),
                    Style::default().fg(state_color(state, palette)),
                ),
                Span::styled(step.name.clone(), Style::default().fg(palette.subtext0)),
            ]));
        }
        lines.push(Line::raw(String::new()));
    }
    lines
}

pub(crate) fn log_lines(log: &str, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    log.lines()
        .flat_map(|raw| {
            let lower = raw.to_ascii_lowercase();
            let style = if lower.contains("error") || lower.contains("failed") {
                Style::default().fg(palette.red)
            } else if lower.contains("warning") {
                Style::default().fg(palette.yellow)
            } else {
                Style::default().fg(palette.subtext0)
            };
            wrap_text(raw, width)
                .into_iter()
                .map(move |line| Line::styled(line, style))
        })
        .collect()
}

