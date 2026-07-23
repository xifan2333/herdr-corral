use corral::github::detail::{self, DetailResource, InitialView};

fn usage() -> ! {
    eprintln!("usage: corral-github <issue|pr|run> --repo [HOST/]OWNER/REPO <id> [--view VIEW]");
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(kind) = args.next() else { usage() };
    let mut repo = None;
    let mut id = None;
    let mut view = InitialView::Overview;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-R" | "--repo" => repo = args.next(),
            "--view" => {
                let Some(value) = args.next() else { usage() };
                view = match value.as_str() {
                    "overview" | "summary" => InitialView::Overview,
                    "conversation" | "comments" => InitialView::Conversation,
                    "files" => InitialView::Files,
                    "diff" => InitialView::Diff,
                    "checks" => InitialView::Checks,
                    "jobs" => InitialView::Jobs,
                    "log" => InitialView::Log,
                    "log-failed" | "failed-log" => InitialView::FailedLog,
                    _ => usage(),
                };
            }
            value if !value.starts_with('-') && id.is_none() => id = value.parse::<u64>().ok(),
            _ => usage(),
        }
    }
    let (Some(repo), Some(id)) = (repo, id) else {
        usage();
    };
    if repo.is_empty()
        || !repo
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '/' | ':'))
    {
        eprintln!("corral-github: invalid repository selector");
        std::process::exit(2);
    }
    let resource = match kind.as_str() {
        "issue" => DetailResource::Issue(id),
        "pr" | "pull" => DetailResource::Pull(id),
        "run" | "action" => DetailResource::Run(id),
        _ => usage(),
    };
    if let Err(error) = detail::run(repo, resource, view) {
        eprintln!("corral-github: {error}");
        std::process::exit(1);
    }
}
