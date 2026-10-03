use schemars::JsonSchema;
use serde::Deserialize;
use tokio::process::Command;

use crate::harness::tool::{Tool, ToolCx, ToolError};

pub struct Shell;

impl Tool for Shell {
    type Input = ShellInput;

    const NAME: &str = "shell";
    const DESCRIPTION: &str = "Run a command with `sh -c` in the working directory. \
        Returns its output, and its exit code if it failed.";

    async fn call(&self, cx: &ToolCx, input: ShellInput) -> Result<String, ToolError> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(&input.command)
            .current_dir(cx.cwd())
            .kill_on_drop(true)
            .output()
            .await?;

        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        if !output.status.success() {
            match output.status.code() {
                Some(code) => text.push_str(&format!("\n[exit code: {code}]")),
                None => text.push_str("\n[terminated by a signal]"),
            }
        }
        Ok(text)
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ShellInput {
    /// The command line to run.
    pub command: String,
}
