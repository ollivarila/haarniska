//! Anthropic adapter, built on `genai`.

use futures_util::{Stream, StreamExt, TryFutureExt, future};
use genai::chat::{
    ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent, ContentPart, MessageContent, StreamEnd,
    Tool, ToolResponse,
};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ServiceTarget};
use serde_json::Value;

use crate::inference::{
    Block, Chunk, Error, Inference, Message, Reply, Request, StopReason, ToolCall, Usage,
};

/// Model IDs to pass to [`AnthropicInference::new`].
pub mod model {
    pub const CLAUDE_FABLE_5_1: &str = "claude-fable-5-1";
    pub const CLAUDE_OPUS_5_5: &str = "claude-opus-5-5";
    pub const CLAUDE_OPUS_5: &str = "claude-opus-5";
    pub const CLAUDE_SONNET_5: &str = "claude-sonnet-5";
    pub const CLAUDE_HAIKU_4_5: &str = "claude-haiku-4-5";
}

const API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const BASE_URL: &str = "https://api.anthropic.com";

pub struct AnthropicInference {
    client: Client,
    model: String,
}

impl AnthropicInference {
    /// Reads the API key from `ANTHROPIC_API_KEY`. Fails if it is not set.
    pub fn new(model: &str) -> Result<Self, Error> {
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| Error::new(format!("{API_KEY_ENV} is not set")))?;

        Ok(Self::with_endpoint(model, &api_key, BASE_URL))
    }

    /// Talks to the Messages API at `base_url` instead of Anthropic's.
    pub fn with_endpoint(model: &str, api_key: &str, base_url: &str) -> Self {
        let endpoint = Endpoint::from_owned(format!("{}/v1/", base_url.trim_end_matches('/')));
        let auth = AuthData::from_single(api_key);
        let target = ServiceTargetResolver::from_resolver_fn(
            move |target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                Ok(ServiceTarget {
                    endpoint: endpoint.clone(),
                    auth: auth.clone(),
                    model: target.model,
                })
            },
        );

        // Without capture, the end of the stream carries no reply.
        let options = ChatOptions::default()
            .with_capture_content(true)
            .with_capture_tool_calls(true)
            .with_capture_usage(true);

        Self {
            client: Client::builder()
                .with_service_target_resolver(target)
                .with_chat_options(options)
                .build(),
            model: format!("anthropic::{model}"),
        }
    }
}

impl Inference for AnthropicInference {
    fn infer(&self, request: Request<'_>) -> impl Stream<Item = Result<Chunk, Error>> {
        self.client
            .exec_chat_stream(self.model.as_str(), to_chat_request(request), None)
            .map_err(to_error)
            .map_ok(|response| {
                response
                    .stream
                    .filter_map(|event| future::ready(to_chunk(event)))
            })
            .try_flatten_stream()
    }
}

fn to_chat_request(request: Request<'_>) -> ChatRequest {
    let messages = request.messages.iter().map(to_chat_message).collect();
    let tools = request.tools.iter().map(|tool| {
        Tool::new(tool.name.as_str())
            .with_description(tool.description.as_str())
            .with_schema(tool.input_schema.clone())
    });

    let mut chat_request = ChatRequest::new(messages).with_tools(tools);
    if !request.system.is_empty() {
        chat_request = chat_request.with_system(request.system);
    }
    chat_request
}

fn to_chat_message(message: &Message) -> ChatMessage {
    match message {
        Message::User(text) => ChatMessage::user(text.as_str()),
        Message::Assistant(blocks) => {
            let parts: Vec<_> = blocks.iter().filter_map(to_content_part).collect();
            ChatMessage::assistant(MessageContent::from_parts(parts))
        }
        Message::ToolResults(results) => {
            // genai has no error flag on tool responses; the output text
            // has to carry it.
            let responses = results
                .iter()
                .map(|result| ToolResponse::new(result.call_id.as_str(), result.output.as_str()))
                .collect();
            ChatMessage::tool(MessageContent::from_tool_responses(responses))
        }
    }
}

fn to_content_part(block: &Block) -> Option<ContentPart> {
    match block {
        Block::Text(text) => Some(ContentPart::Text(text.clone())),
        Block::ToolCall(call) => Some(ContentPart::ToolCall(genai::chat::ToolCall {
            call_id: call.id.clone(),
            fn_name: call.name.clone(),
            fn_arguments: call.input.clone(),
            thought_signatures: None,
        })),
        Block::Opaque(Value::String(signature)) => {
            Some(ContentPart::ThoughtSignature(signature.clone()))
        }
        Block::Opaque(_) => None,
    }
}

fn to_chunk(event: genai::Result<ChatStreamEvent>) -> Option<Result<Chunk, Error>> {
    match event {
        Ok(ChatStreamEvent::Chunk(chunk)) => Some(Ok(Chunk::Text(chunk.content))),
        Ok(ChatStreamEvent::End(end)) => Some(Ok(Chunk::Done(to_reply(end)))),
        Ok(_) => None,
        Err(error) => Some(Err(to_error(error))),
    }
}

fn to_reply(end: StreamEnd) -> Reply {
    let content: Vec<_> = end
        .captured_content
        .map(MessageContent::into_parts)
        .unwrap_or_default()
        .into_iter()
        .filter_map(to_block)
        .collect();

    let has_tool_call = content
        .iter()
        .any(|block| matches!(block, Block::ToolCall(_)));
    let stop = match end.captured_stop_reason {
        Some(genai::chat::StopReason::ToolCall(_)) => StopReason::ToolUse,
        Some(genai::chat::StopReason::MaxTokens(_)) => StopReason::MaxTokens,
        Some(genai::chat::StopReason::ContentFilter(_)) => StopReason::Refusal,
        Some(genai::chat::StopReason::Other(reason)) if reason == "refusal" => StopReason::Refusal,
        None if has_tool_call => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    };

    let usage = end.captured_usage.unwrap_or_default();
    let count = |tokens: Option<i32>| tokens.and_then(|n| u64::try_from(n).ok()).unwrap_or(0);

    Reply {
        content,
        stop,
        usage: Usage {
            input_tokens: count(usage.prompt_tokens),
            output_tokens: count(usage.completion_tokens),
        },
    }
}

fn to_block(part: ContentPart) -> Option<Block> {
    match part {
        ContentPart::Text(text) => Some(Block::Text(text)),
        ContentPart::ToolCall(call) => Some(Block::ToolCall(ToolCall {
            id: call.call_id,
            name: call.fn_name,
            input: call.fn_arguments,
        })),
        ContentPart::ThoughtSignature(signature) => Some(Block::Opaque(Value::String(signature))),
        _ => None,
    }
}

fn to_error(error: genai::Error) -> Error {
    Error::new(error.to_string())
}
