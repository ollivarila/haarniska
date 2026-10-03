//! The agent loop, driven by scripted model replies and real tools in a
//! temporary directory.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures_util::{Stream, StreamExt, stream};
use haarniska::Agent;
use haarniska::harness::Event;
use haarniska::harness::tool::{Tool, ToolCx, ToolError};
use haarniska::inference::{
    Block, Chunk, Error, Inference, Message, Reply, Request, StopReason, ToolCall, ToolResult,
    Usage,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn text_reply_is_streamed_and_ends_the_turn() {
    let (inference, _requests) = ScriptedInference::new([text_reply("Hello")]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let events: Vec<Event> = agent.prompt("Say hello").collect().await;

    assert_eq!(
        events,
        [
            Event::Text("Hello".into()),
            Event::Done {
                stop: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ]
    );
}

#[tokio::test]
async fn tool_call_runs_and_its_result_goes_back_to_the_model() {
    let sandbox = TempDir::new().unwrap();
    let call = ToolCall {
        id: "call_1".into(),
        name: "write".into(),
        input: json!({"path": "a.txt", "content": "hi"}),
    };
    let (inference, requests) =
        ScriptedInference::new([tool_reply(call.clone()), text_reply("Written.")]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    let result = ToolResult {
        call_id: "call_1".into(),
        output: "Wrote 2 bytes to a.txt".into(),
        is_error: false,
    };
    assert_eq!(
        events,
        [
            Event::ToolStarted(call.clone()),
            Event::ToolFinished(result.clone()),
            Event::Text("Written.".into()),
            Event::Done {
                stop: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ]
    );
    let written = std::fs::read_to_string(sandbox.path().join("a.txt")).unwrap();
    assert_eq!(written, "hi");
    assert_eq!(
        requests.lock().unwrap()[1],
        [
            Message::User("Write a.txt".into()),
            Message::Assistant(vec![Block::ToolCall(call)]),
            Message::ToolResults(vec![result]),
        ]
    );
}

#[tokio::test]
async fn failing_tool_gives_the_model_an_error_and_the_turn_goes_on() {
    let sandbox = TempDir::new().unwrap();
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("read", json!({"path": "missing.txt"}))),
        text_reply("No such file."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Read missing.txt").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert!(result.output.starts_with("Error: "));
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn unknown_tool_gives_the_model_an_error() {
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("nope", json!({}))),
        text_reply("Sorry."),
    ]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let events: Vec<Event> = agent.prompt("Do it").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(result.output, "Error: no tool named `nope`");
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn input_that_does_not_fit_gives_the_model_an_error() {
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("read", json!({"file": "a.txt"}))),
        text_reply("Sorry."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .build();

    let events: Vec<Event> = agent.prompt("Read a.txt").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert!(result.output.starts_with("Error: invalid input"));
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn panicking_tool_gives_the_model_an_error_and_the_turn_goes_on() {
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("panic", json!({}))),
        text_reply("It crashed."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_tool(Panic)
        .build();

    let events: Vec<Event> = agent.prompt("Crash").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(result.output, "Error: the tool crashed");
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn failing_model_call_ends_the_turn_with_a_failure() {
    let (inference, _requests) = ScriptedInference::new([]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let events: Vec<Event> = agent.prompt("Hello").collect().await;

    assert_eq!(
        events,
        [Event::Failed(Error::new("no scripted reply left"))]
    );
}

#[tokio::test]
async fn next_prompt_continues_the_conversation() {
    let (inference, requests) = ScriptedInference::new([text_reply("Hi."), text_reply("Fine.")]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let _: Vec<Event> = agent.prompt("Hello").collect().await;
    let _: Vec<Event> = agent.prompt("How are you?").collect().await;

    assert_eq!(
        requests.lock().unwrap()[1],
        [
            Message::User("Hello".into()),
            Message::Assistant(vec![Block::Text("Hi.".into())]),
            Message::User("How are you?".into()),
        ]
    );
}

/// A tool that always panics.
struct Panic;

#[derive(Deserialize, JsonSchema)]
struct NoInput {}

impl Tool for Panic {
    type Input = NoInput;

    const NAME: &str = "panic";
    const DESCRIPTION: &str = "Panics.";

    async fn call(&self, _cx: &ToolCx, _input: NoInput) -> Result<String, ToolError> {
        panic!("boom")
    }
}

/// An [`Inference`] that plays back `replies` in order and records the
/// messages of every request it receives.
struct ScriptedInference {
    replies: Mutex<VecDeque<Reply>>,
    requests: Requests,
}

type Requests = Arc<Mutex<Vec<Vec<Message>>>>;

impl ScriptedInference {
    fn new(replies: impl IntoIterator<Item = Reply>) -> (Self, Requests) {
        let requests = Requests::default();
        let inference = Self {
            replies: Mutex::new(replies.into_iter().collect()),
            requests: requests.clone(),
        };
        (inference, requests)
    }
}

impl Inference for ScriptedInference {
    fn infer(&self, request: Request<'_>) -> impl Stream<Item = Result<Chunk, Error>> {
        self.requests
            .lock()
            .unwrap()
            .push(request.messages.to_vec());
        let reply = self.replies.lock().unwrap().pop_front();

        let chunks: Vec<_> = match reply {
            Some(reply) => {
                let texts = reply.content.iter().filter_map(|block| match block {
                    Block::Text(text) => Some(Ok(Chunk::Text(text.clone()))),
                    _ => None,
                });
                texts.chain([Ok(Chunk::Done(reply.clone()))]).collect()
            }
            None => vec![Err(Error::new("no scripted reply left"))],
        };
        stream::iter(chunks)
    }
}

fn text_reply(text: &str) -> Reply {
    Reply {
        content: vec![Block::Text(text.into())],
        stop: StopReason::EndTurn,
        usage: Usage::default(),
    }
}

fn tool_reply(call: ToolCall) -> Reply {
    Reply {
        content: vec![Block::ToolCall(call)],
        stop: StopReason::ToolUse,
        usage: Usage::default(),
    }
}

fn tool_call(name: &str, input: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call_1".into(),
        name: name.into(),
        input,
    }
}

/// The result of the only tool call in `events`.
fn tool_result(events: &[Event]) -> &ToolResult {
    let mut results = events.iter().filter_map(|event| match event {
        Event::ToolFinished(result) => Some(result),
        _ => None,
    });
    let result = results.next().expect("a tool call finished");
    assert!(results.next().is_none(), "only one tool call finished");
    result
}

fn done() -> Event {
    Event::Done {
        stop: StopReason::EndTurn,
        usage: Usage::default(),
    }
}
