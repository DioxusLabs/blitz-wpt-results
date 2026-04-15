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
