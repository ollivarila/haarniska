//! `AnthropicInference` against a local server speaking the Messages API.

use futures_util::TryStreamExt;
use haarniska::inference::anthropic::{AnthropicInference, model};
use haarniska::inference::{
    Block, Chunk, Inference, Message, Reply, Request, StopReason, ToolResult, Usage,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// TODO: This is a stub smoke test. Not implemented more comprehensively because don't want to
// couple the test to the API implementation. Ideally should create fixtures that are easy to
// regenerate.
const TEXT_REPLY: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-opus-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"#;

const THINKING_TOOL_REPLY: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-opus-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I need the file."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig123"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"a.rs\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":20}}

event: message_stop
data: {"type":"message_stop"}

"#;

/// A server that answers every Messages API call with `sse`.
async fn server_replying(sse: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;
    server
}

fn inference_for(server: &MockServer) -> AnthropicInference {
    AnthropicInference::with_endpoint(model::CLAUDE_OPUS_5, "test-key", &server.uri()).unwrap()
}

/// Every chunk of one model call with `messages`.
async fn infer(inference: &AnthropicInference, messages: &[Message]) -> Vec<Chunk> {
    let request = Request {
        system: "",
        messages,
        tools: &[],
    };
    inference.infer(request).try_collect().await.unwrap()
}

#[tokio::test]
async fn text_reply_streams_deltas_then_the_whole_reply() {
    let server = server_replying(TEXT_REPLY).await;
    let inference = inference_for(&server);
    let messages = [Message::User("Say hello".into())];

    let chunks = infer(&inference, &messages).await;

    assert_eq!(
        chunks,
        [
            Chunk::Text("Hello".into()),
            Chunk::Text(" world".into()),
            Chunk::Done(Reply {
                content: vec![Block::Text("Hello world".into())],
                stop: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            }),
        ]
    );
}

#[tokio::test]
async fn last_message_is_a_cache_point() {
    let server = server_replying(TEXT_REPLY).await;
    let inference = inference_for(&server);
    let messages = [
        Message::User("Say hello".into()),
        Message::Assistant(vec![Block::Text("Hello".into())]),
        Message::User("Again".into()),
    ];

    infer(&inference, &messages).await;

    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = requests[0].body_json().unwrap();
    let cache_points: Vec<_> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message.to_string().contains("cache_control"))
        .collect();
    assert_eq!(cache_points, [false, false, true]);
    assert_eq!(
        body["messages"][2]["content"][0]["cache_control"],
        serde_json::json!({"type": "ephemeral"})
    );
}

#[tokio::test]
async fn thinking_is_sent_back_unchanged_with_the_tool_results() {
    let server = server_replying(THINKING_TOOL_REPLY).await;
    let inference = inference_for(&server);
    let mut messages = vec![Message::User("Fix a.rs".into())];

    let chunks = infer(&inference, &messages).await;
    let Some(Chunk::Done(reply)) = chunks.last() else {
        panic!("the stream ends with the reply");
    };
    assert_eq!(reply.stop, StopReason::ToolUse);
    messages.push(Message::Assistant(reply.content.clone()));
    messages.push(Message::ToolResults(vec![ToolResult {
        call_id: "toolu_1".into(),
        output: "fn main() {}".into(),
        is_error: false,
    }]));
    infer(&inference, &messages).await;

    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = requests[1].body_json().unwrap();
    assert_eq!(
        body["messages"][1]["content"],
        serde_json::json!([
            {"type": "thinking", "thinking": "I need the file.", "signature": "sig123"},
            {"type": "tool_use", "id": "toolu_1", "name": "read", "input": {"path": "a.rs"}},
        ])
    );
}
