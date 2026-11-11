//! The TUI's view of the invented data in `capcom::demo` (no polling, no GitHub, no Herdr).
use crate::app::App;
use capcom::demo::{build_stories, pull_requests, runs};
pub use capcom::demo::make_root;
use std::path::Path;

/// An app showing the invented data (no polling, no GitHub, no Herdr).
pub fn app(root: &Path) -> App {
    build_stories(root);
    let mut app = App::new(root.to_path_buf(), None);
    app.prs.disabled = false;
    app.apply_prs(Ok(pull_requests()));
    app.apply_runs(Ok(runs()));
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use capcom::model::Board;
    use capcom::{model::{Status}, rules, store};

    #[test]
    fn the_demo_has_stories_in_every_column_and_nothing_real() {
        let root = make_root();
        let app = app(&root);
        assert_eq!(app.keys, vec!["DEMO-101", "DEMO-102", "DEMO-103", "DEMO-104", "DEMO-105"]);
        let board: Board = store::load(&root, "DEMO-101").unwrap();
        let statuses: Vec<Status> = board.tasks.iter().map(|t| t.status).collect();
        for want in [Status::Todo, Status::Planning, Status::Planned, Status::Implementing, Status::Done] {
            assert!(statuses.contains(&want), "{want:?} missing in {statuses:?}");
        }
        assert_eq!(app.pr_rows().len(), 4);
        let text = std::fs::read_to_string(root.join("DEMO-101/board.json")).unwrap();
        assert!(text.contains("acme/") && text.contains("jira.example.com"), "only invented names and hosts");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_repo_listed_twice_shows_the_backfill_task_expecting_two_prs() {
        let root = make_root();
        let _ = app(&root);
        let board = store::load(&root, "DEMO-101").unwrap();
        assert_eq!(rules::repo_summary(&board.task("T5").unwrap().repos), "api ×2");
        let _ = std::fs::remove_dir_all(&root);
    }
}
