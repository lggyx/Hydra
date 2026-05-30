use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use hydra_core::git::worktree::WorktreeManager;

use crate::{json_error, AppState};

#[derive(Debug, Serialize)]
pub(crate) struct WorktreeInfo {
    pub branch: String,
    pub path: String,
    pub has_changes: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateWorktreeRequest {
    pub name: String,
    #[serde(default = "default_base_ref")]
    pub base_ref: String,
}

fn default_base_ref() -> String {
    "HEAD".to_string()
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateWorktreeResponse {
    pub branch: String,
    pub path: String,
    pub base_branch: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListWorktreesResponse {
    pub items: Vec<WorktreeInfo>,
}

fn build_manager(working_dir: &std::path::Path) -> Result<WorktreeManager, (StatusCode, Json<crate::ApiError>)> {
    WorktreeManager::from_dir(working_dir.to_path_buf())
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to initialize worktree manager: {}", e)))
}

pub(crate) async fn list_worktrees(State(state): State<AppState>) -> impl IntoResponse {
    let working_dir = state.project.read().await.working_dir.clone();
    let manager = match build_manager(&working_dir) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    match manager.list() {
        Ok(worktrees) => {
            let items: Vec<WorktreeInfo> = worktrees
                .into_iter()
                .map(|(branch, path, has_changes)| WorktreeInfo {
                    branch,
                    path: path.to_string_lossy().to_string(),
                    has_changes,
                })
                .collect();
            Json(ListWorktreesResponse { items }).into_response()
        }
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to list worktrees: {}", e)).into_response(),
    }
}

pub(crate) async fn create_worktree(
    State(state): State<AppState>,
    Json(req): Json<CreateWorktreeRequest>,
) -> impl IntoResponse {
    let working_dir = state.project.read().await.working_dir.clone();
    let manager = match build_manager(&working_dir) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    match manager.create(&req.name, &req.base_ref) {
        Ok(worktree) => {
            (StatusCode::CREATED, Json(CreateWorktreeResponse {
                branch: worktree.branch,
                path: worktree.path.to_string_lossy().to_string(),
                base_branch: worktree.base_branch,
            })).into_response()
        }
        Err(e) => json_error(StatusCode::CONFLICT, format!("Failed to create worktree: {}", e)).into_response(),
    }
}

pub(crate) async fn delete_worktree(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let working_dir = state.project.read().await.working_dir.clone();
    let manager = match build_manager(&working_dir) {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    match manager.remove(&id, false) {
        Ok(()) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => json_error(StatusCode::CONFLICT, format!("Failed to remove worktree: {}", e)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn setup_repo() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo");
        run_git(&repo, &["init"]);
        run_git(&repo, &["config", "user.email", "test@example.com"]);
        run_git(&repo, &["config", "user.name", "Test"]);
        let file = repo.join("test.txt");
        std::fs::write(&file, "hello\n").expect("write file");
        run_git(&repo, &["add", "."]);
        run_git(&repo, &["commit", "-m", "initial"]);
        (temp, repo)
    }

    #[test]
    fn test_create_worktree_appears_in_list() {
        let (_temp, repo) = setup_repo();
        let manager = WorktreeManager::new(repo.clone());
        let branch = format!("test-wt-{}", uuid::Uuid::new_v4().simple());
        manager.create(&branch, "HEAD").expect("create worktree");
        let list = manager.list().expect("list worktrees");
        assert!(
            list.iter().any(|(b, _, _)| b == &branch),
            "created worktree should appear in list"
        );
    }

    #[test]
    fn test_delete_worktree_removes_from_list() {
        let (_temp, repo) = setup_repo();
        let manager = WorktreeManager::new(repo.clone());
        let branch = format!("test-wt-{}", uuid::Uuid::new_v4().simple());
        manager.create(&branch, "HEAD").expect("create worktree");
        manager.remove(&branch, true).expect("remove worktree");
        let list = manager.list().expect("list worktrees");
        assert!(
            !list.iter().any(|(b, _, _)| b == &branch),
            "deleted worktree should not appear in list"
        );
    }

    #[test]
    fn test_list_worktrees_includes_main() {
        let (_temp, repo) = setup_repo();
        let manager = WorktreeManager::new(repo.clone());
        let list = manager.list().expect("list worktrees");
        assert!(!list.is_empty(), "should have at least the main worktree");
    }
}
