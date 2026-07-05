//! Resolves the current git branch for a directory by reading `.git/HEAD`
//! directly — no `git` subprocess, so the ~1s live refresh can afford one
//! lookup per pane. Handles linked worktrees (`.git` as a `gitdir:` file)
//! and detached HEADs.

use std::path::{Path, PathBuf};

use crate::herdr_client::{PaneInfo, WorkspaceInfo};

/// Columns of a detached-HEAD hash to show, git's default short length.
const SHORT_HASH_LEN: usize = 7;

/// Fills in the locally-resolved `branch` on a fresh snapshot. Panes prefer
/// `foreground_cwd` (where a `cd`-ed agent actually runs) over the shell's
/// `cwd`; workspaces resolve at their worktree checkout, when they have one.
pub fn annotate(workspaces: &mut [WorkspaceInfo], panes: &mut [PaneInfo]) {
    // Split panes usually share a directory; resolve each one once per
    // snapshot. Owned keys: borrowed ones cannot outlive their pane's
    // loop iteration.
    let mut cache: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut resolve = |dir: &str| {
        cache
            .entry(dir.to_string())
            .or_insert_with(|| branch_for(Path::new(dir)))
            .clone()
    };
    for pane in panes {
        pane.branch = pane
            .foreground_cwd
            .as_deref()
            .or(pane.cwd.as_deref())
            .and_then(&mut resolve);
    }
    for ws in workspaces {
        ws.branch = ws
            .worktree
            .as_ref()
            .and_then(|wt| resolve(&wt.checkout_path));
    }
}

/// Walks up from `dir` to the repository root and describes its HEAD:
/// `Some("main")` on a branch, `Some("abc1234 (detached)")` when detached,
/// `None` outside a repository (or on any read/parse failure).
pub fn branch_for(dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir(dir)?.join("HEAD")).ok()?;
    parse_head(&head)
}

/// The actual git directory governing `dir`: the nearest `.git` ancestor
/// entry, following a linked worktree's `gitdir:` pointer when `.git` is a
/// file instead of a directory.
fn git_dir(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir
        .ancestors()
        .map(|a| a.join(".git"))
        .find(|c| c.exists())?;
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let target = Path::new(pointer.strip_prefix("gitdir:")?.trim());
    if target.is_absolute() {
        Some(target.to_path_buf())
    } else {
        // Relative gitdir pointers resolve against the .git file's directory.
        Some(dot_git.parent()?.join(target))
    }
}

fn parse_head(head: &str) -> Option<String> {
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        // Non-branch refs (bisect on a tag, etc.) fall through with their
        // full name rather than pretending to be a branch.
        return Some(
            reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_string(),
        );
    }
    // Detached HEAD: a bare commit hash.
    if head.len() >= SHORT_HASH_LEN && head.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("{} (detached)", &head[..SHORT_HASH_LEN]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake repo: `<root>/.git/HEAD` holding `head`.
    fn fake_repo(root: &Path, head: &str) {
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), head).unwrap();
    }

    #[test]
    fn resolves_the_branch_from_head() {
        let dir = tempfile::tempdir().unwrap();
        fake_repo(dir.path(), "ref: refs/heads/main\n");
        assert_eq!(branch_for(dir.path()), Some("main".to_string()));
    }

    #[test]
    fn walks_up_from_a_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        fake_repo(dir.path(), "ref: refs/heads/main\n");
        let sub = dir.path().join("src/deeply/nested");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(branch_for(&sub), Some("main".to_string()));
    }

    #[test]
    fn keeps_slashes_in_branch_names() {
        let dir = tempfile::tempdir().unwrap();
        fake_repo(dir.path(), "ref: refs/heads/feature/nice-things\n");
        assert_eq!(
            branch_for(dir.path()),
            Some("feature/nice-things".to_string())
        );
    }

    #[test]
    fn detached_head_shows_a_short_hash() {
        let dir = tempfile::tempdir().unwrap();
        fake_repo(dir.path(), "3951e23a63877f2b194179c3321664d63877f2aa\n");
        assert_eq!(
            branch_for(dir.path()),
            Some("3951e23 (detached)".to_string())
        );
    }

    #[test]
    fn follows_a_linked_worktree_gitdir_file() {
        let dir = tempfile::tempdir().unwrap();
        let main_repo = dir.path().join("repo");
        fake_repo(&main_repo, "ref: refs/heads/main\n");
        let wt_git_dir = main_repo.join(".git/worktrees/wt");
        std::fs::create_dir_all(&wt_git_dir).unwrap();
        std::fs::write(wt_git_dir.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        // Relative pointer, like git writes for worktrees next to the repo.
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../repo/.git/worktrees/wt\n").unwrap();
        assert_eq!(branch_for(&worktree), Some("feature/x".to_string()));

        // An absolute pointer resolves too.
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", wt_git_dir.display()),
        )
        .unwrap();
        assert_eq!(branch_for(&worktree), Some("feature/x".to_string()));
    }

    #[test]
    fn annotate_prefers_the_foreground_cwd_and_fills_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let shell_repo = dir.path().join("shell");
        fake_repo(&shell_repo, "ref: refs/heads/shell-branch\n");
        let agent_repo = dir.path().join("agent");
        fake_repo(&agent_repo, "ref: refs/heads/agent-branch\n");

        let mut pane = PaneInfo {
            pane_id: "w1:p1".to_string(),
            tab_id: "w1:t1".to_string(),
            workspace_id: "w1".to_string(),
            focused: false,
            agent: None,
            display_agent: None,
            agent_status: crate::herdr_client::AgentStatus::Idle,
            cwd: Some(shell_repo.display().to_string()),
            foreground_cwd: Some(agent_repo.display().to_string()),
            label: None,
            title: None,
            custom_status: None,
            terminal_id: "term".to_string(),
            branch: None,
        };
        let mut ws = WorkspaceInfo {
            workspace_id: "w1".to_string(),
            number: 1,
            label: "alpha".to_string(),
            focused: false,
            pane_count: 1,
            tab_count: 1,
            active_tab_id: "w1:t1".to_string(),
            agent_status: crate::herdr_client::AgentStatus::Idle,
            worktree: Some(crate::herdr_client::WorkspaceWorktree {
                repo_name: "shell".to_string(),
                checkout_path: shell_repo.display().to_string(),
                is_linked_worktree: false,
            }),
            branch: None,
        };
        annotate(
            std::slice::from_mut(&mut ws),
            std::slice::from_mut(&mut pane),
        );
        assert_eq!(pane.branch.as_deref(), Some("agent-branch"));
        assert_eq!(ws.branch.as_deref(), Some("shell-branch"));

        // Without a foreground cwd the shell cwd decides.
        pane.foreground_cwd = None;
        ws.worktree = None;
        annotate(
            std::slice::from_mut(&mut ws),
            std::slice::from_mut(&mut pane),
        );
        assert_eq!(pane.branch.as_deref(), Some("shell-branch"));
        assert_eq!(ws.branch, None, "no worktree, no workspace branch");
    }

    #[test]
    fn non_repo_and_garbage_head_yield_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(branch_for(dir.path()), None);
        fake_repo(dir.path(), "not a head at all\n");
        assert_eq!(branch_for(dir.path()), None);
    }
}
