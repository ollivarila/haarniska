use schemars::JsonSchema;
use serde::Deserialize;

use crate::harness::tool::{Tool, ToolCx, ToolError};

pub struct Write;

impl Tool for Write {
    type Input = WriteInput;

    const NAME: &str = "write";
    const DESCRIPTION: &str =
        "Write a file, replacing it if it exists. Missing parent directories are created.";

    async fn call(&self, cx: &ToolCx, input: WriteInput) -> Result<String, ToolError> {
        let path = cx.cwd().join(&input.path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, &input.content).await?;

        Ok(format!(
            "Wrote {} bytes to {}",
            input.content.len(),
            input.path
        ))
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct WriteInput {
    /// Path of the file, absolute or relative to the working directory.
    pub path: String,
    /// The full new contents of the file.
    pub content: String,
}
