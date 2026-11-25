use std::path::Path;

use anyhow::{Context, Result};

pub const MAX_RETRIES: u32 = 5;
pub const INITIAL_BACKOFF_MS: u64 = 500;

/// Sleep with exponential backoff if the response is 429 (Too Many Requests).
/// Returns `true` if the caller should retry, `false` if retries are exhausted
/// or the response was not 429.
pub async fn backoff_on_429(
    response: &reqwest::Response,
    attempt: &mut u32,
    backoff_ms: &mut u64,
) -> bool {
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && *attempt < MAX_RETRIES {
        *attempt += 1;
        eprintln!(
            "  rate limited, retrying in {}ms (attempt {}/{})",
            backoff_ms, *attempt, MAX_RETRIES,
        );
        tokio::time::sleep(std::time::Duration::from_millis(*backoff_ms)).await;
        *backoff_ms *= 2;
        true
    } else {
        false
    }
}

/// Send a request, retrying with exponential backoff on HTTP 429.
///
/// `build` is invoked once per attempt to produce a fresh request, since
/// `RequestBuilder::send` consumes the builder (and multipart forms cannot be
/// cloned). Non-429 responses and transport errors are returned immediately.
pub async fn send_with_retry<F>(mut build: F) -> reqwest::Result<reqwest::Response>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let mut attempt = 0u32;
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    loop {
        match build().send().await {
            Ok(r) if backoff_on_429(&r, &mut attempt, &mut backoff_ms).await => continue,
            other => break other,
        }
    }
}

pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::env::var("CI").is_err()
}

/// Find the top-level directory of the git repository containing `dir`.
///
/// Asks `git` first, then falls back to the nearest ancestor holding a `.git`
/// entry (a directory, or a file in worktrees and submodules), so CI images
/// without `git`, or where `git` refuses a workspace owned by another user,
/// still resolve the repository. Returns `None` if `dir` is not inside a
/// working tree, so callers can fall back to treating `dir` itself as the
/// path root.
pub fn find_git_root(dir: &Path) -> Option<std::path::PathBuf> {
    git_toplevel(dir).or_else(|| dot_git_ancestor(dir))
}

fn git_toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        return None;
    }
    // Match the canonicalized scan root, including Windows' verbatim prefix.
    // Otherwise strip_prefix fails for `\\?\C:\repo` versus `C:/repo`.
    std::path::PathBuf::from(path).canonicalize().ok()
}

fn dot_git_ancestor(dir: &Path) -> Option<std::path::PathBuf> {
    let dir = dir.canonicalize().ok()?;
    dir.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Try to detect a git remote URL from `root`.
/// Prefers `origin`; falls back to the first listed remote.
pub fn detect_git_remote(root: &Path) -> Option<String> {
    let get_url = |remote: &str| -> Option<String> {
        let out = std::process::Command::new("git")
            .args(["remote", "get-url", remote])
            .current_dir(root)
            .output()
            .ok()?;
        if out.status.success() {
            let url = String::from_utf8(out.stdout).ok()?.trim().to_string();
            if !url.is_empty() {
                Some(url)
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(url) = get_url("origin") {
        return Some(url);
    }

    let out = std::process::Command::new("git")
        .arg("remote")
        .current_dir(root)
        .output()
        .ok()?;
    if out.status.success() {
        let remotes = String::from_utf8(out.stdout).ok()?;
        let first = remotes.lines().next()?.trim().to_string();
        if !first.is_empty() {
            return get_url(&first);
        }
    }
    None
}

/// Prompt the user for a source ID and persist it to the config file.
/// If `suggestion` is provided it is shown as the default (accepted on empty input).
pub fn prompt_and_persist_source_id(
    config_path: &Path,
    suggestion: Option<&str>,
) -> Result<String> {
    use std::io::Write;

    eprintln!();
    eprintln!("Warning: '.lekton.yml' is missing the required 'id' field.");
    eprintln!();
    eprintln!("This ID uniquely identifies this repository as a document source.");
    eprintln!("It scopes auto-archiving so multiple repos can share the same token");
    eprintln!("without interfering with each other.");
    eprintln!();

    match suggestion {
        Some(s) => eprint!("Enter a source ID [{}]: ", s),
        None => eprint!("Enter a source ID (e.g. \"my-org/my-repo\", \"user-service\"): "),
    }
    std::io::stderr().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    let id = match (trimmed.is_empty(), suggestion) {
        (true, Some(s)) => s.to_string(),
        (true, None) => anyhow::bail!("Source ID cannot be empty."),
        (false, _) => trimmed.to_string(),
    };

    let existing = if config_path.exists() {
        std::fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?
    } else {
        String::new()
    };
    let new_content = format!("id: {id}\n{existing}");
    std::fs::write(config_path, &new_content)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;

    eprintln!("Saved 'id: {id}' to {}", config_path.display());
    eprintln!();

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_git_root_resolves_from_subdirectory() {
        let repo = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        assert!(status.success());

        let sub = repo.path().join("docs");
        std::fs::create_dir(&sub).unwrap();

        let found = find_git_root(&sub).unwrap();
        assert_eq!(found, repo.path().canonicalize().unwrap());
    }

    #[test]
    fn find_git_root_falls_back_to_dot_git_without_git() {
        // An empty `.git` is not a repository to `git`, as when the binary is
        // missing or refuses the workspace: the marker alone must still count.
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let sub = repo.path().join("docs").join("guides");
        std::fs::create_dir_all(&sub).unwrap();

        assert!(git_toplevel(&sub).is_none());
        assert_eq!(
            find_git_root(&sub).unwrap(),
            repo.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn find_git_root_none_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_git_root(dir.path()).is_none());
    }
}
