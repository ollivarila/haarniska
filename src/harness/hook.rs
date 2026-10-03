//! Hooks: code that runs around tool calls.

use async_trait::async_trait;
use serde_json::Value;

use crate::harness::tool::ToolCx;
use crate::inference::{ToolCall, ToolResult};

/// Runs before and after every tool call, and can block or change it.
///
/// Both points are optional; a hook that changes nothing only records.
///
/// ```
/// use haarniska::harness::hook::{Decision, Hook};
/// use haarniska::harness::tool::ToolCx;
/// use serde_json::Value;
///
/// struct NoForceDelete;
///
/// impl Hook for NoForceDelete {
///     async fn before_tool(&self, _cx: &ToolCx, tool: &str, input: &mut Value) -> Decision {
///         let command = input["command"].as_str().unwrap_or_default();
///         if tool == "shell" && command.contains("rm -rf") {
///             return Decision::Block("rm -rf is not allowed in this project".into());
///         }
///         Decision::Continue
///     }
/// }
///
/// let agent = haarniska::builder().with_hook(NoForceDelete);
/// ```
pub trait Hook: Send + Sync + 'static {
    /// Runs before a tool call. May change `input`. A block gives the model
    /// an error result with the reason, and the tool does not run.
    fn before_tool(
        &self,
        _cx: &ToolCx,
        _tool: &str,
        _input: &mut Value,
    ) -> impl Future<Output = Decision> + Send {
        async { Decision::Continue }
    }

    /// Runs after a tool call, also for one that failed or was blocked. May
    /// change the output and whether it counts as an error.
    fn after_tool(
        &self,
        _cx: &ToolCx,
        _call: &ToolCall,
        _result: &mut ToolResult,
    ) -> impl Future<Output = ()> + Send {
        async {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decision {
    Continue,
    /// Stop the call, for this reason.
    Block(String),
}

/// A [`Hook`] usable as a trait object, so hooks of different types can sit
/// in one list.
#[async_trait]
pub(crate) trait DynHook: Send + Sync {
    async fn before_tool(&self, cx: &ToolCx, tool: &str, input: &mut Value) -> Decision;

    async fn after_tool(&self, cx: &ToolCx, call: &ToolCall, result: &mut ToolResult);
}

#[async_trait]
impl<H: Hook> DynHook for H {
    async fn before_tool(&self, cx: &ToolCx, tool: &str, input: &mut Value) -> Decision {
        Hook::before_tool(self, cx, tool, input).await
    }

    async fn after_tool(&self, cx: &ToolCx, call: &ToolCall, result: &mut ToolResult) {
        Hook::after_tool(self, cx, call, result).await
    }
}
