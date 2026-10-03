//! `AnthropicInference` against a local server speaking the Messages API.

use futures_util::TryStreamExt;
use haarniska::inference::anthropic::{AnthropicInference, model};
use haarniska::inference::{Block, Chunk, Inference, Message, Reply, Request, StopReason, Usage};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn text_reply_streams_deltas_then_the_whole_reply() {
    let server = server_replying(TEXT_REPLY).await;
    let inference =
        AnthropicInference::with_endpoint(model::CLAUDE_OPUS_5, "test-key", &server.uri());
    let messages = [Message::User("Say hello".into())];

    let chunks: Vec<Chunk> = inference
        .infer(Request {
            system: "",
            messages: &messages,
            tools: &[],
        })
        .try_collect()
        .await
        .unwrap();

    assert_eq!(
        chunks,
        [
            Chunk::Text("Hello".into()),
            Chunk::Text(" world".into()),
            Chunk::Done(Reply {
                content: vec![Block::Text("Hello world".into())],
                stop: StopReason::EndTurn,
                // Anthropic's final count (5) is cumulative; genai 0.6.5
                // adds the count from message_start (1) on top.
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 6,
                },
            }),
        ]
    );
}
