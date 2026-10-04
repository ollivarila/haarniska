//! Running the user's command that gets new AWS credentials.

use std::process::Stdio;

use tokio::process::Command;

use crate::inference::Error;

/// Runs `command` in a shell and waits for it. Its output is captured, not
/// shown, so it cannot draw over a terminal UI.
pub(super) async fn run(command: &str) -> Result<(), Error> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| Error::new(format!("auth refresh `{command}` did not start: {error}")))?;

    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(Error::new(format!(
        "auth refresh `{command}` failed ({}): {}",
        output.status,
        stderr.trim()
    )))
}
