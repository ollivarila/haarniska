//! Inference: model APIs behind one interface.

pub mod anthropic;

use std::ops::AddAssign;

use futures_util::Stream;
use serde_json::Value;

/// A model API. One call sends the conversation and streams back the reply.
pub trait Inference {
    /// Yields text deltas as they arrive, then [`Chunk::Done`] with the
    /// complete reply. A failure is yielded as an error and ends the stream.
    fn infer(&self, request: Request<'_>) -> impl Stream<Item = Result<Chunk, Error>>;
}

#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolSpec],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Message {
    User(String),
    Assistant(Vec<Block>),
    /// Every result of one round of tool calls.
    ToolResults(Vec<ToolResult>),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Block {
    Text(String),
    ToolCall(ToolCall),
    /// Owned by the adapter that produced it. The harness stores it and
    /// sends it back unchanged.
    Opaque(Value),
}

impl Block {
    pub fn as_tool_call(&self) -> Option<&ToolCall> {
        match self {
            Block::ToolCall(call) => Some(call),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Chunk {
    /// A text delta, for display only.
    Text(String),
    /// The complete reply. Always last.
    Done(Reply),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub content: Vec<Block>,
    pub stop: StopReason,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
