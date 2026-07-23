//! Full-width interactive GitHub detail client used by `corral-github`.
//!
//! The 32-column sidebar remains a navigator. This app runs in the shared
//! owner-scoped nvim terminal and owns resource detail presentation.
//!
//! Layout:
//! - [`app`] — state machine, input, terminal loop
//! - [`markdown`] — pulldown-cmark → ratatui lines + CJK wrap
//! - [`images`] — text image links + external viewer
//! - [`pages`] — issue/PR/run page builders
//! - [`util`] — small shared helpers

mod app;
mod images;
mod markdown;
mod pages;
mod util;

pub use app::{run, DetailResource, InitialView};

#[cfg(test)]
mod tests {
    use super::app::{tabs_for, ComposeKind, Detail, DetailApp, DetailResource, InitialView, Mode, Tab};
    use super::images::extract_html_img;
    use super::markdown::{display_width, flush_block, render_markdown, wrap_text};
    use crate::config::Config;
    use crate::github::{
        GitHubDetailAdapter, GitHubMutation, IssueDetail, MergeMethod, PullRequestDetail,
        WorkflowRunDetail,
    };
    use crate::ui::Palette;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::style::Style;
    use std::sync::Arc;
    use std::time::Duration;

    struct FakeAdapter;

    impl GitHubDetailAdapter for FakeAdapter {
        fn issue_detail(&self, _repo: &str, _number: u64) -> Result<IssueDetail, String> {
            Err("unused".into())
        }

        fn pull_detail(&self, _repo: &str, _number: u64) -> Result<PullRequestDetail, String> {
            Err("unused".into())
        }

        fn run_detail(&self, _repo: &str, _run_id: u64) -> Result<WorkflowRunDetail, String> {
            Err("unused".into())
        }

        fn pull_patch(&self, _repo: &str, _number: u64) -> Result<String, String> {
            Err("unused".into())
        }

        fn run_log(&self, _repo: &str, _run_id: u64, _failed_only: bool) -> Result<String, String> {
            Err("unused".into())
        }

        fn mutate(&self, _repo: &str, _mutation: &GitHubMutation) -> Result<String, String> {
            Ok(String::new())
        }
    }

    fn app(resource: DetailResource) -> DetailApp {
        DetailApp::with_adapter(
            "owner/repo".into(),
            resource,
            InitialView::Overview,
            Arc::new(Config::for_test()),
            Arc::new(FakeAdapter),
        )
    }

    struct RecordingAdapter {
        mutations: Arc<std::sync::Mutex<Vec<GitHubMutation>>>,
    }

    impl GitHubDetailAdapter for RecordingAdapter {
        fn issue_detail(&self, _repo: &str, _number: u64) -> Result<IssueDetail, String> {
            Err("unused".into())
        }

        fn pull_detail(&self, _repo: &str, _number: u64) -> Result<PullRequestDetail, String> {
            Err("unused".into())
        }

        fn run_detail(&self, _repo: &str, _run_id: u64) -> Result<WorkflowRunDetail, String> {
            Err("unused".into())
        }

        fn pull_patch(&self, _repo: &str, _number: u64) -> Result<String, String> {
            Err("unused".into())
        }

        fn run_log(&self, _repo: &str, _run_id: u64, _failed_only: bool) -> Result<String, String> {
            Err("unused".into())
        }

        fn mutate(&self, _repo: &str, mutation: &GitHubMutation) -> Result<String, String> {
            self.mutations.lock().unwrap().push(mutation.clone());
            Ok(String::new())
        }
    }

    #[test]
    fn wraps_text_without_losing_words() {
        assert_eq!(wrap_text("one two three", 7), vec!["one two", "three"]);
    }

    #[test]
    fn wraps_cjk_by_display_columns_not_scalar_count() {
        // Each CJK ideograph is 2 terminal columns. A 10-column budget must
        // break before the 6th character (12 columns), not after 10 scalars.
        let text = "中文显示宽度测试段落内容";
        let lines = wrap_text(text, 10);
        assert!(lines.len() >= 2, "expected wrap, got {lines:?}");
        for line in &lines {
            assert!(
                display_width(line) <= 10,
                "line {line:?} is {} cols", display_width(line)
            );
        }
        assert_eq!(lines.concat(), text);
    }

    #[test]
    fn flush_block_keeps_cjk_lines_within_budget() {
        let palette = Palette::resolve();
        let mut out = Vec::new();
        let mut segs = vec![(
            "我们GUI里那个下拉列表是这样填的：先硬编码一项default再去枚举PW上media.class完全等于Audio/Source的节点追加进去。"
                .to_string(),
            Style::default(),
        )];
        flush_block(&mut out, &mut segs, 40, "", None, &palette);
        assert!(out.len() >= 2);
        for line in &out {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                display_width(&text) <= 40,
                "line {text:?} is {} cols", display_width(&text)
            );
        }
    }

    #[test]
    fn resource_tabs_are_context_specific() {
        // Issue/PR overview and comments now share one Conversation page.
        assert_eq!(tabs_for(DetailResource::Issue(1)), &[Tab::Conversation]);
        assert_eq!(
            tabs_for(DetailResource::Pull(2)),
            &[Tab::Conversation, Tab::Files, Tab::Diff, Tab::Checks]
        );
        assert_eq!(
            tabs_for(DetailResource::Run(3)),
            &[Tab::Overview, Tab::Jobs, Tab::Log]
        );
    }

    #[test]
    fn markdown_renders_headings_lists_and_extracts_images() {
        let palette = Palette::resolve();
        let (lines, images) = render_markdown(
            "# Title\n\nsome **bold** text\n\n- one\n- two\n\n![alt](https://example.test/a.png)",
            60,
            &palette,
        );
        assert!(lines
            .iter()
            .any(|line| line.spans.iter().any(|span| span.content.contains("Title"))));
        assert!(lines
            .iter()
            .any(|line| line.spans.iter().any(|span| span.content.contains('•'))));
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].1, "https://example.test/a.png");
        assert!(lines.iter().any(|line| line
            .spans
            .iter()
            .any(|span| span.content.contains("[image]"))));
    }

    #[test]
    fn heading_spaces_survive_soft_breaks() {
        let palette = Palette::resolve();
        let (lines, _) = render_markdown(
            "### Expected behavior\n\nSteps to reproduce",
            80,
            &palette,
        );
        let joined: String = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Expected behavior"), "got: {joined:?}");
        assert!(joined.contains("Steps to reproduce"), "got: {joined:?}");
    }

    #[test]
    fn html_img_tags_are_extracted_as_images() {
        let palette = Palette::resolve();
        let comment = "see it: <img width=\"398\" alt=\"Image\" src=\"https://example.test/pic.png\" />, done";
        let (_, images) = render_markdown(comment, 80, &palette);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].1, "https://example.test/pic.png");
    }

    #[test]
    fn bare_github_attachment_urls_become_image_links() {
        let palette = Palette::resolve();
        let body = "see\n\nhttps://github.com/user-attachments/assets/80598080-ce5b-4208-9fa7-b08f6338e51f\n\ndone";
        let (lines, images) = render_markdown(body, 80, &palette);
        assert_eq!(images.len(), 1);
        assert!(images[0]
            .1
            .contains("github.com/user-attachments/assets/"));
        assert!(lines.iter().any(|line| line
            .spans
            .iter()
            .any(|span| span.content.contains("[image]"))));
    }

    #[test]
    fn labeled_image_links_keep_surrounding_text() {
        let palette = Palette::resolve();
        let body = "hello [shot](https://github.com/user-attachments/assets/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee) world";
        let (lines, images) = render_markdown(body, 100, &palette);
        // Labeled links stay as links, not image rows.
        assert!(images.is_empty(), "got images: {images:?}");
        let joined: String = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("hello"), "lost leading text: {joined:?}");
        assert!(joined.contains("world"), "lost trailing text: {joined:?}");
        assert!(joined.contains("shot"), "lost link label: {joined:?}");
    }

    #[test]
    fn html_attr_reads_quoted_and_unquoted() {
        assert_eq!(
            extract_html_img("<img src=\"https://a.test/x.png\" alt='hi'>"),
            Some(("https://a.test/x.png".into(), "hi".into()))
        );
        assert_eq!(extract_html_img("<img src=/local/relative.png>"), None);
    }

    #[test]
    fn image_links_always_keep_a_textual_row() {
        let palette = Palette::resolve();
        let (lines, images) = render_markdown("![diagram](https://example.test/x.png)", 60, &palette);
        assert_eq!(images.len(), 1);
        assert!(lines.iter().any(|line| line
            .spans
            .iter()
            .any(|span| span.content.contains("diagram"))));
    }

    #[test]
    fn compose_submission_becomes_a_typed_comment_mutation() {
        let mutations = Arc::new(std::sync::Mutex::new(Vec::new()));
        let adapter = Arc::new(RecordingAdapter {
            mutations: Arc::clone(&mutations),
        });
        let mut app = DetailApp::with_adapter(
            "owner/repo".into(),
            DetailResource::Issue(7),
            InitialView::Overview,
            Arc::new(Config::for_test()),
            adapter,
        );
        app.mode = Mode::Compose {
            kind: ComposeKind::Comment,
            text: "hello from Corral".chars().collect(),
        };
        app.submit_compose();
        for _ in 0..20 {
            if !mutations.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            mutations.lock().unwrap().as_slice(),
            &[GitHubMutation::IssueComment {
                number: 7,
                body: "hello from Corral".into(),
            }]
        );
    }

    #[test]
    fn context_actions_are_typed_and_destructive_actions_confirm() {
        let mut issue_app = app(DetailResource::Issue(7));
        issue_app.detail = Some(Detail::Issue(
            serde_json::from_str(r#"{"number":7,"title":"bug","state":"OPEN"}"#).unwrap(),
        ));
        issue_app.context_action();
        assert!(matches!(
            issue_app.mode,
            Mode::Confirm {
                mutation: GitHubMutation::IssueState {
                    number: 7,
                    open: false
                },
                ..
            }
        ));

        let mut pull_app = app(DetailResource::Pull(8));
        pull_app.detail = Some(Detail::Pull(
            serde_json::from_str(
                r#"{"number":8,"title":"feature","state":"OPEN","headRefOid":"abcdef123456","statusCheckRollup":[]}"#,
            )
            .unwrap(),
        ));
        pull_app.merge_pull();
        assert!(matches!(
            pull_app.mode,
            Mode::MergeMethod {
                number: 8,
                selected: 1,
                ..
            }
        ));
        pull_app.confirm_selected_merge();
        assert!(matches!(
            pull_app.mode,
            Mode::Confirm {
                mutation: GitHubMutation::PullMerge {
                    number: 8,
                    method: MergeMethod::Squash,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn merge_method_picker_cycles_and_confirms_selected_strategy() {
        let mut pull_app = app(DetailResource::Pull(9));
        pull_app.detail = Some(Detail::Pull(
            serde_json::from_str(
                r#"{"number":9,"title":"feature","state":"OPEN","isDraft":false,"headRefOid":"abcdef123456","statusCheckRollup":[]}"#,
            )
            .unwrap(),
        ));
        pull_app.merge_pull();
        pull_app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE, 10);
        pull_app.confirm_selected_merge();
        assert!(matches!(
            pull_app.mode,
            Mode::Confirm {
                mutation: GitHubMutation::PullMerge {
                    number: 9,
                    method: MergeMethod::Rebase,
                    ..
                },
                ..
            }
        ));
    }
}


