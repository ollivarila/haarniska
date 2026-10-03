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
    /// Run the call only if the user agrees. The reason is shown to them.
    Ask(String),
}

/// Asks before the tools it was given. A starting point for a consent
/// policy; write a [`Hook`] for anything finer.
pub struct AskBefore {
    tools: Vec<String>,
}

impl AskBefore {
    pub fn tools<T: Into<String>>(tools: impl IntoIterator<Item = T>) -> Self {
        Self {
            tools: tools.into_iter().map(Into::into).collect(),
        }
    }
}

impl Hook for AskBefore {
    async fn before_tool(&self, _cx: &ToolCx, tool: &str, _input: &mut Value) -> Decision {
        if self.tools.iter().any(|name| name == tool) {
            Decision::Ask(format!("{tool} needs your consent"))
        } else {
            Decision::Continue
        }
    }
}

/// Puts a question from a hook to the user. A UI provides one.
pub trait Approver: Send + Sync + 'static {
    fn approve(&self, request: ApprovalRequest) -> impl Future<Output = Answer> + Send;
}

/// A tool call waiting for the user's consent.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequest {
    pub tool: String,
    /// As the tool will get it, after every hook's changes.
    pub input: Value,
    /// Why each hook that asked did so.
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Answer {
    Allow,
    /// Allow, and do not ask about this tool again in this session.
    AllowAlways,
    Deny,
}

/// A [`Hook`] usable as a trait object, so hooks of different types can sit
/// in one list.
#[async_trait]
pub(crate) trait DynHook: Send + Sync {
    async fn before_tool(&self, cx: &ToolCx, tool: &str, input: &mut Value) -> Decision;

    async fn after_tool(&self, cx: &ToolCx, call: &ToolCall, result: &mut ToolResult);
}

/// An [`Approver`] usable as a trait object.
#[async_trait]
pub(crate) trait DynApprover: Send + Sync {
    async fn approve(&self, request: ApprovalRequest) -> Answer;
}

#[async_trait]
impl<A: Approver> DynApprover for A {
    async fn approve(&self, request: ApprovalRequest) -> Answer {
        Approver::approve(self, request).await
    }
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
