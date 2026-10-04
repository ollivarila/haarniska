//! `BedrockInference` against a local server speaking Bedrock's InvokeModel
//! stream.

use std::path::{Path, PathBuf};

use aws_config::{BehaviorVersion, Region, SdkConfig};
use aws_credential_types::Credentials;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::{ProvideCredentials, SharedCredentialsProvider, future};
use aws_smithy_types::event_stream::{Header, HeaderValue, Message as EventMessage};
use base64::Engine;
use futures_util::TryStreamExt;
use haarniska::inference::bedrock::BedrockInference;
use haarniska::inference::{
    Block, Chunk, Error, Inference, Message, Reply, Request, StopReason, ToolResult, Usage,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL: &str = "anthropic.claude-test";
const STREAM_PATH: &str = "/model/anthropic.claude-test/invoke-with-response-stream";

fn text_reply() -> Vec<Value> {
    vec![
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant",
               "content": [], "usage": {"input_tokens": 10, "cache_read_input_tokens": 90,
               "cache_creation_input_tokens": 5, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hello"}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": " world"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ]
}

fn thinking_tool_reply() -> Vec<Value> {
    vec![
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant",
               "content": [], "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0,
               "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
        json!({"type": "content_block_delta", "index": 0,
               "delta": {"type": "thinking_delta", "thinking": "I need the file."}}),
        json!({"type": "content_block_delta", "index": 0,
               "delta": {"type": "signature_delta", "signature": "sig123"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1,
               "content_block": {"type": "tool_use", "id": "toolu_1", "name": "read", "input": {}}}),
        json!({"type": "content_block_delta", "index": 1,
               "delta": {"type": "input_json_delta", "partial_json": "{\"path\": \"a.rs\"}"}}),
        json!({"type": "content_block_stop", "index": 1}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 20}}),
        json!({"type": "message_stop"}),
    ]
}

/// Bedrock's event stream: each Anthropic event is a `chunk` event holding
/// the event's JSON, base64-encoded.
fn event_stream(events: &[Value]) -> ResponseTemplate {
    let mut body = Vec::new();
    for event in events {
        let bytes = base64::engine::general_purpose::STANDARD.encode(event.to_string());
        let message = EventMessage::new(json!({"bytes": bytes}).to_string())
            .add_header(string_header(":event-type", "chunk"))
            .add_header(string_header(":message-type", "event"))
            .add_header(string_header(":content-type", "application/json"));
        aws_smithy_eventstream::frame::write_message_to(&message, &mut body).unwrap();
    }
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/vnd.amazon.eventstream")
        .set_body_bytes(body)
}

fn string_header(name: &'static str, value: &'static str) -> Header {
    Header::new(name, HeaderValue::String(value.into()))
}

fn expired_token() -> ResponseTemplate {
    ResponseTemplate::new(403)
        .insert_header("x-amzn-errortype", "ExpiredTokenException")
        .set_body_json(json!({"message": "The security token included in the request is expired"}))
}

/// A mock answering every call with an expired-token error.
fn rejecting_expired_token() -> Mock {
    Mock::given(method("POST"))
        .and(path(STREAM_PATH))
        .respond_with(expired_token())
}

async fn server_replying(events: &[Value]) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(STREAM_PATH))
        .respond_with(event_stream(events))
        .mount(&server)
        .await;
    server
}

fn config_for(server: &MockServer, credentials: impl ProvideCredentials + 'static) -> SdkConfig {
    SdkConfig::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(server.uri())
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .build()
}

fn inference_for(server: &MockServer) -> BedrockInference {
    let credentials = Credentials::new("key", "secret", None, None, "test");
    BedrockInference::from_config(MODEL, config_for(server, credentials))
}

/// Credentials that only load once `marker` exists, as after a login.
#[derive(Debug)]
struct LoggedInAfter {
    marker: PathBuf,
}

impl ProvideCredentials for LoggedInAfter {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::ready(if self.marker.exists() {
            Ok(Credentials::new("key", "secret", None, None, "test"))
        } else {
            Err(CredentialsError::not_loaded("the SSO session has expired"))
        })
    }
}

fn touch(file: &Path) -> String {
    format!("touch '{}'", file.display())
}

/// Every chunk of one model call with `messages`, or the first error.
async fn infer(inference: &BedrockInference, messages: &[Message]) -> Result<Vec<Chunk>, Error> {
    let request = Request {
        system: "Be brief.",
        messages,
        tools: &[],
    };
    inference.infer(request).try_collect().await
}

async fn last_request_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    requests.last().unwrap().body_json().unwrap()
}

#[tokio::test]
async fn text_reply_streams_deltas_then_the_whole_reply() {
    let server = server_replying(&text_reply()).await;
    let inference = inference_for(&server);
    let messages = [Message::User("Say hello".into())];

    let chunks = infer(&inference, &messages).await.unwrap();

    assert_eq!(
        chunks,
        [
            Chunk::Text("Hello".into()),
            Chunk::Text(" world".into()),
            Chunk::Done(Reply {
                content: vec![Block::Text("Hello world".into())],
                stop: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 105,
                    output_tokens: 5,
                },
            }),
        ]
    );
}

#[tokio::test]
async fn request_is_a_messages_api_body() {
    let server = server_replying(&text_reply()).await;
    let inference = inference_for(&server);
    let messages = [Message::User("Say hello".into())];

    infer(&inference, &messages).await.unwrap();

    let body = last_request_body(&server).await;
    assert_eq!(body["anthropic_version"], "bedrock-2023-05-31");
    assert_eq!(body["system"], "Be brief.");
    assert!(body["max_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn last_message_is_a_cache_point() {
    let server = server_replying(&text_reply()).await;
    let inference = inference_for(&server);
    let messages = [
        Message::User("Say hello".into()),
        Message::Assistant(vec![Block::Text("Hello".into())]),
        Message::User("Again".into()),
    ];

    infer(&inference, &messages).await.unwrap();

    let body = last_request_body(&server).await;
    let cache_points: Vec<_> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message.to_string().contains("cache_control"))
        .collect();
    assert_eq!(cache_points, [false, false, true]);
    assert_eq!(
        body["messages"][2]["content"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );
}

#[tokio::test]
async fn thinking_is_sent_back_unchanged_with_the_tool_results() {
    let server = server_replying(&thinking_tool_reply()).await;
    let inference = inference_for(&server);
    let mut messages = vec![Message::User("Fix a.rs".into())];

    let chunks = infer(&inference, &messages).await.unwrap();
    let Some(Chunk::Done(reply)) = chunks.last() else {
        panic!("the stream ends with the reply");
    };
    assert_eq!(reply.stop, StopReason::ToolUse);
    messages.push(Message::Assistant(reply.content.clone()));
    messages.push(Message::ToolResults(vec![ToolResult {
        call_id: "toolu_1".into(),
        output: "no such file".into(),
        is_error: true,
    }]));
    infer(&inference, &messages).await.unwrap();

    let body = last_request_body(&server).await;
    assert_eq!(
        body["messages"][1]["content"],
        json!([
            {"type": "thinking", "thinking": "I need the file.", "signature": "sig123"},
            {"type": "tool_use", "id": "toolu_1", "name": "read", "input": {"path": "a.rs"}},
        ])
    );
    assert_eq!(
        body["messages"][2]["content"][0],
        json!({"type": "tool_result", "tool_use_id": "toolu_1", "content": "no such file",
               "is_error": true, "cache_control": {"type": "ephemeral"}})
    );
}

#[tokio::test]
async fn expired_token_runs_auth_refresh_and_retries() {
    let server = MockServer::start().await;
    rejecting_expired_token()
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(STREAM_PATH))
        .respond_with(event_stream(&text_reply()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("refreshed");
    let inference = inference_for(&server).with_auth_refresh(&touch(&marker));

    let chunks = infer(&inference, &[Message::User("Hi".into())])
        .await
        .unwrap();

    assert!(marker.exists());
    assert!(matches!(chunks.last(), Some(Chunk::Done(_))));
}

#[tokio::test]
async fn missing_credentials_run_auth_refresh_and_retry() {
    let server = server_replying(&text_reply()).await;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("logged-in");
    let credentials = LoggedInAfter {
        marker: marker.clone(),
    };
    let inference = BedrockInference::from_config(MODEL, config_for(&server, credentials))
        .with_auth_refresh(&touch(&marker));

    let chunks = infer(&inference, &[Message::User("Hi".into())])
        .await
        .unwrap();

    assert!(matches!(chunks.last(), Some(Chunk::Done(_))));
}

#[tokio::test]
async fn expired_token_without_auth_refresh_is_an_error() {
    let server = MockServer::start().await;
    rejecting_expired_token().mount(&server).await;
    let inference = inference_for(&server);

    let error = infer(&inference, &[Message::User("Hi".into())])
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("ExpiredTokenException"),
        "{error}"
    );
}

#[tokio::test]
async fn failing_auth_refresh_is_an_error() {
    let server = MockServer::start().await;
    rejecting_expired_token().mount(&server).await;
    let inference = inference_for(&server).with_auth_refresh("echo 'login failed' >&2; exit 1");

    let error = infer(&inference, &[Message::User("Hi".into())])
        .await
        .unwrap_err();

    assert!(error.to_string().contains("login failed"), "{error}");
}
