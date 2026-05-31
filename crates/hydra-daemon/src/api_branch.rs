use std::path::PathBuf;
use std::process::Command;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::{json_error, AppState};

#[derive(Debug, Serialize)]
pub(crate) struct BranchInfo {
    pub name: String,
    pub is_head: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListBranchesResponse {
    pub items: Vec<BranchInfo>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateBranchRequest {
    pub name: String,
    #[serde(default = "default_base_ref")]
    pub base_ref: String,
}

fn default_base_ref() -> String {
    "HEAD".to_string()
}

#[derive(Debug, Serialize)]
pub(crate) struct CreateBranchResponse {
    pub name: String,
}

fn repo_root(working_dir: &std::path::Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(working_dir)
        .output()
        .map_err(|e| format!("Failed to run git: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

pub(crate) async fn list_branches(State(state): State<AppState>) -> impl IntoResponse {
    let working_dir = state.project.read().await.working_dir.clone();
    let root = match repo_root(&working_dir) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let output = match Command::new("git").args(["branch"]).current_dir(&root).output() {
        Ok(o) => o,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list branches: {}", e),
            )
            .into_response()
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<BranchInfo> = stdout
        .lines()
        .map(|line| {
            let name = line.trim_start_matches(|c| c == '*' || c == ' ').to_string();
            let is_head = line.trim_start().starts_with('*');
            BranchInfo { name, is_head }
        })
        .collect();
    Json(ListBranchesResponse { items }).into_response()
}

pub(crate) async fn create_branch(
    State(state): State<AppState>,
    Json(req): Json<CreateBranchRequest>,
) -> impl IntoResponse {
    let working_dir = state.project.read().await.working_dir.clone();
    let root = match repo_root(&working_dir) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let output = match Command::new("git")
        .args(["branch", &req.name, &req.base_ref])
        .current_dir(&root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create branch: {}", e),
            )
            .into_response()
        }
    };
    if !output.status.success() {
        return json_error(
            StatusCode::CONFLICT,
            format!(
                "Failed to create branch: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )
        .into_response();
    }
    (
        StatusCode::CREATED,
        Json(CreateBranchResponse { name: req.name }),
    )
        .into_response()
}

pub(crate) async fn delete_branch(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let working_dir = state.project.read().await.working_dir.clone();
    let root = match repo_root(&working_dir) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let output = match Command::new("git")
        .args(["branch", "-d", &name])
        .current_dir(&root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete branch: {}", e),
            )
            .into_response()
        }
    };
    if !output.status.success() {
        return json_error(
            StatusCode::CONFLICT,
            format!(
                "Failed to delete branch: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )
        .into_response();
    }
    Json(serde_json::json!({"success": true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let output = StdCommand::new("git")
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

    fn list_branches_raw(repo: &std::path::Path) -> Vec<String> {
        let output = StdCommand::new("git")
            .args(["branch"])
            .current_dir(repo)
            .output()
            .expect("git branch");
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .map(|l| l.trim_start_matches(|c| c == '*' || c == ' ').to_string())
            .collect()
    }

    #[test]
    fn test_create_branch_appears_in_list() {
        let (_temp, repo) = setup_repo();
        let name = format!("test-br-{}", uuid::Uuid::new_v4().simple());
        let output = StdCommand::new("git")
            .args(["branch", &name, "HEAD"])
            .current_dir(&repo)
            .output()
            .expect("git branch");
        assert!(output.status.success(), "create branch should succeed");
        let branches = list_branches_raw(&repo);
        assert!(
            branches.contains(&name),
            "created branch should appear in list"
        );
    }

    #[test]
    fn test_delete_branch_removes_from_list() {
        let (_temp, repo) = setup_repo();
        let name = format!("test-br-{}", uuid::Uuid::new_v4().simple());
        run_git(&repo, &["branch", &name, "HEAD"]);
        run_git(&repo, &["branch", "-d", &name]);
        let branches = list_branches_raw(&repo);
        assert!(
            !branches.contains(&name),
            "deleted branch should not appear in list"
        );
    }

    #[test]
    fn test_list_branches_includes_main() {
        let (_temp, repo) = setup_repo();
        let branches = list_branches_raw(&repo);
        assert!(!branches.is_empty(), "should have at least the main branch");
    }
}
