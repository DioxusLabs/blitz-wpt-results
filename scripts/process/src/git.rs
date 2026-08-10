use std::{
    path::Path,
    process::{Command, Output},
};

// fn set_git_credentials(name: &str, email: &str) -> Result<Output, std::io::Error> {
//     Command::new("git")
//         .args(["config", "user.name", name])
//         .output()?;
//     Command::new("git")
//         .args(["config", "user.email", email])
//         .output()
// }

pub fn git_add(path: impl AsRef<Path>) -> Result<Output, std::io::Error> {
    Command::new("git")
        .arg("add")
        .arg(path.as_ref().as_os_str())
        .output()
}

pub fn git_commit(message: &str) -> Result<Output, std::io::Error> {
    Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg(message)
        .output()
}

/// The committer timestamp (seconds since the unix epoch) of a commit in the
/// git repository at `repo`
pub fn git_commit_timestamp(repo: &Path, sha: &str) -> Option<i64> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show", "-s", "--format=%ct", sha])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

/// The first line of the commit message of a commit in the git repository at `repo`
pub fn git_commit_message(repo: &Path, sha: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show", "-s", "--format=%s", sha])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}
