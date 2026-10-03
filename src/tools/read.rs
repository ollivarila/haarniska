use schemars::JsonSchema;
use serde::Deserialize;

use crate::harness::tool::{Tool, ToolCx, ToolError};

pub struct Read;

impl Tool for Read {
    type Input = ReadInput;

    const NAME: &str = "read";
    const DESCRIPTION: &str = "Read a text file and return its contents.";

    async fn call(&self, cx: &ToolCx, input: ReadInput) -> Result<String, ToolError> {
        Ok(tokio::fs::read_to_string(cx.cwd().join(input.path)).await?)
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadInput {
    /// Path of the file, absolute or relative to the working directory.
    pub path: String,
}
