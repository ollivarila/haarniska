//! The harness: agent loop and everything around the model.

pub mod hook;
pub mod instructions;
pub mod tool;

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::pin::pin;

use async_stream::stream;
use futures_util::future::{Either, select};
use futures_util::{FutureExt, Stream, StreamExt};

use crate::Agent;
use crate::inference::{
    Block, Chunk, Error, Inference, Message, Request, StopReason, ToolCall, ToolResult, ToolSpec,
    Usage,
};
use crate::ui::{Input, Ui};
use hook::{Decision, DynHook};
use tool::{DynTool, ToolCx, ToolError};

/// What happens during a turn, in order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A piece of the model's reply.
    Text(String),
    ToolStarted(ToolCall),
    ToolFinished(ToolResult),
    /// The turn is over. Always last, unless the turn failed.
    Done {
        stop: StopReason,
        usage: Usage,
    },
    /// The turn ended because the model call failed. Always last.
    Failed(Error),
}

/// The loops that run tools against a model.
pub(crate) struct Harness<I> {
    inference: I,
    tools: Vec<Box<dyn DynTool>>,
    tool_specs: Vec<ToolSpec>,
    hooks: Vec<Box<dyn DynHook>>,
    tool_cx: ToolCx,
    system_prompt: String,
    messages: Vec<Message>,
}

impl<I: Inference> Agent for Harness<I> {
    fn prompt(&mut self, text: &str) -> impl Stream<Item = Event> {
        let text = text.to_owned();
        stream! {
            self.messages.push(Message::User(text));
            let mut usage = Usage::default();

            loop {
                let mut reply = None;
                {
                    let mut chunks = pin!(self.inference.infer(self.request()));
                    while let Some(chunk) = chunks.next().await {
                        match chunk {
                            Ok(Chunk::Text(text)) => yield Event::Text(text),
                            Ok(Chunk::Done(done)) => {
                                reply = Some(done);
                                break;
                            }
                            Err(error) => {
                                yield Event::Failed(error);
                                return;
                            }
                        }
                    }
                }
                let Some(reply) = reply else {
                    yield Event::Failed(Error::new("the model's reply ended early"));
                    return;
                };
                usage.input_tokens += reply.usage.input_tokens;
                usage.output_tokens += reply.usage.output_tokens;

                let calls: Vec<_> = reply.content.iter().filter_map(as_tool_call).cloned().collect();
                if calls.is_empty() {
                    self.messages.push(Message::Assistant(reply.content));
                    yield Event::Done { stop: reply.stop, usage };
                    return;
                }

                let mut results = Vec::new();
                for mut call in calls {
                    let allowed = self.before_tool(&mut call).await;
                    yield Event::ToolStarted(call.clone());
                    let outcome = match allowed {
                        Ok(()) => self.run_tool(&call).await,
                        Err(blocked) => Err(blocked),
                    };
                    let mut result = to_result(&call, outcome);
                    self.after_tool(&call, &mut result).await;
                    yield Event::ToolFinished(result.clone());
                    results.push(result);
                }

                // A reply and its results are added together, so a turn
                // dropped mid-round leaves no tool call without a result.
                self.messages.push(Message::Assistant(reply.content));
                self.messages.push(Message::ToolResults(results));
            }
        }
    }

    async fn run(mut self, mut ui: impl Ui) {
        loop {
            let text = match ui.next().await {
                Input::Prompt(text) => text,
                Input::Cancel => continue,
                Input::Quit => return,
            };

            let mut events = pin!(self.prompt(&text));
            loop {
                // Scoped so the wait for input is dropped before `ui` is
                // used again.
                let step = {
                    let input = pin!(ui.next());
                    match select(events.next(), input).await {
                        Either::Left((event, _)) => Either::Left(event),
                        Either::Right((input, _)) => Either::Right(input),
                    }
                };
                match step {
                    Either::Left(Some(event)) => ui.show(event),
                    Either::Left(None) => break,
                    // Dropping `events` cancels the turn.
                    Either::Right(Input::Cancel) => break,
                    Either::Right(Input::Quit) => return,
                    Either::Right(Input::Prompt(_)) => {}
                }
            }
        }
    }
}

impl<I> Harness<I> {
    pub(crate) fn new(
        inference: I,
        tools: Vec<Box<dyn DynTool>>,
        hooks: Vec<Box<dyn DynHook>>,
        system_prompt: String,
        cwd: PathBuf,
    ) -> Self {
        Self {
            tool_specs: tools.iter().map(|tool| tool.spec()).collect(),
            inference,
            tools,
            hooks,
            tool_cx: ToolCx::new(cwd),
            system_prompt,
            messages: Vec::new(),
        }
    }

    /// The whole conversation so far, as the next model call.
    fn request(&self) -> Request<'_> {
        Request {
            system: &self.system_prompt,
            messages: &self.messages,
            tools: &self.tool_specs,
        }
    }

    /// Asks every hook, in order, whether `call` may run. Hooks may change
    /// its input.
    async fn before_tool(&self, call: &mut ToolCall) -> Result<(), CallError> {
        for hook in &self.hooks {
            let decision = hook.before_tool(&self.tool_cx, &call.name, &mut call.input);
            match AssertUnwindSafe(decision).catch_unwind().await {
                Ok(Decision::Continue) => {}
                Ok(Decision::Block(reason)) => return Err(CallError::Blocked(reason)),
                // A guard that fails must not let the call through.
                Err(_) => return Err(CallError::HookCrashed),
            }
        }
        Ok(())
    }

    /// Runs one tool call. A failure, also a crash, is returned as an error
    /// so the model can react and the turn goes on.
    async fn run_tool(&self, call: &ToolCall) -> Result<String, CallError> {
        let index = self
            .tool_specs
            .iter()
            .position(|spec| spec.name == call.name)
            .ok_or_else(|| CallError::UnknownTool(call.name.clone()))?;
        let run = self.tools[index].call(&self.tool_cx, call.input.clone());
        match AssertUnwindSafe(run).catch_unwind().await {
            Ok(outcome) => Ok(outcome?),
            Err(_) => Err(CallError::ToolCrashed),
        }
    }

    /// Lets every hook, in order, change `result`. A hook that crashes
    /// leaves it as it was.
    async fn after_tool(&self, call: &ToolCall, result: &mut ToolResult) {
        for hook in &self.hooks {
            let before = result.clone();
            let changed = hook.after_tool(&self.tool_cx, call, result);
            if AssertUnwindSafe(changed).catch_unwind().await.is_err() {
                *result = before;
            }
            // A result always belongs to its call.
            result.call_id.clone_from(&call.id);
        }
    }
}

/// Why a tool call gave no output. The message is what the model is told.
#[derive(Debug, thiserror::Error)]
enum CallError {
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("no tool named `{0}`")]
    UnknownTool(String),
    #[error("the tool crashed")]
    ToolCrashed,
    #[error("blocked by a hook: {0}")]
    Blocked(String),
    #[error("blocked: a hook crashed")]
    HookCrashed,
}

fn to_result(call: &ToolCall, outcome: Result<String, CallError>) -> ToolResult {
    match outcome {
        Ok(output) => ToolResult {
            call_id: call.id.clone(),
            output,
            is_error: false,
        },
        // Not every adapter passes `is_error` on, so the text says it too.
        Err(error) => ToolResult {
            call_id: call.id.clone(),
            output: format!("Error: {error}"),
            is_error: true,
        },
    }
}

fn as_tool_call(block: &Block) -> Option<&ToolCall> {
    match block {
        Block::ToolCall(call) => Some(call),
        _ => None,
    }
}
