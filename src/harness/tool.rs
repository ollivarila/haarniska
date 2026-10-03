//! Tools: what the model can call.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::inference::ToolSpec;

/// Something the model can call.
///
/// ```
/// use haarniska::harness::tool::{Tool, ToolCx, ToolError};
/// use schemars::JsonSchema;
/// use serde::Deserialize;
///
/// #[derive(Deserialize, JsonSchema)]
/// struct ReadInput {
///     /// Path of the file to read.
///     path: String,
/// }
///
/// struct Read;
///
/// impl Tool for Read {
///     type Input = ReadInput;
///
///     const NAME: &str = "read";
///     const DESCRIPTION: &str = "Read a file and return its contents.";
///
///     async fn call(&self, cx: &ToolCx, input: ReadInput) -> Result<String, ToolError> {
///         Ok(std::fs::read_to_string(cx.cwd().join(input.path))?)
///     }
/// }
///
/// let agent = haarniska::builder().tool(Read);
/// ```
pub trait Tool: Send + Sync + 'static {
    /// Parsed from the model's JSON before the tool runs. Its schema is what
    /// the model sees.
    type Input: DeserializeOwned + JsonSchema + Send;

    const NAME: &str;
    const DESCRIPTION: &str;

    fn call(
        &self,
        cx: &ToolCx,
        input: Self::Input,
    ) -> impl Future<Output = Result<String, ToolError>> + Send;
}

/// What the harness gives a tool besides its input.
pub struct ToolCx {
    cwd: PathBuf,
}

impl ToolCx {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Where relative paths resolve.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

/// Why a tool call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ToolError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("invalid input: {0}")]
    InvalidInput(#[from] serde_json::Error),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl ToolError {
    /// Wraps any other error, or a plain message.
    pub fn other(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Other(error.into())
    }
}

/// A [`Tool`] with its input type erased, so tools of different types can
/// sit in one list.
#[async_trait]
pub(crate) trait DynTool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    async fn call(&self, cx: &ToolCx, input: Value) -> Result<String, ToolError>;
}

#[async_trait]
impl<T: Tool> DynTool for T {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: T::NAME.into(),
            description: T::DESCRIPTION.into(),
            input_schema: schemars::schema_for!(T::Input).to_value(),
        }
    }

    async fn call(&self, cx: &ToolCx, input: Value) -> Result<String, ToolError> {
        let input = serde_json::from_value(input)?;
        Tool::call(self, cx, input).await
    }
}
