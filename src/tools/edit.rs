use schemars::JsonSchema;
use serde::Deserialize;

use crate::harness::tool::{Tool, ToolCx, ToolError};

pub struct Edit;

impl Tool for Edit {
    type Input = EditInput;

    const NAME: &str = "edit";
    const DESCRIPTION: &str =
        "Replace one exact piece of text in a file. The text must occur exactly once.";

    async fn call(&self, cx: &ToolCx, input: EditInput) -> Result<String, ToolError> {
        let path = cx.cwd().join(&input.path);
        let content = tokio::fs::read_to_string(&path).await?;

        match content.matches(&input.old).count() {
            0 => Err(ToolError::other("`old` was not found in the file")),
            1 => {
                let content = content.replacen(&input.old, &input.new, 1);
                tokio::fs::write(&path, content).await?;
                Ok(format!("Edited {}", input.path))
            }
            count => Err(ToolError::other(format!(
                "`old` was found {count} times; it must occur exactly once"
            ))),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct EditInput {
    /// Path of the file, absolute or relative to the working directory.
    pub path: String,
    /// The exact text to replace.
    pub old: String,
    /// The text to put in its place.
    pub new: String,
}
