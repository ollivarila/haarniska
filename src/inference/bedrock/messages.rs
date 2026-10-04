//! The Anthropic Messages API wire format, as Bedrock's InvokeModel takes it.

use serde_json::{Value, json};

use crate::inference::{
    Block, Chunk, Error, Message, Reply, Request, StopReason, ToolCall, ToolResult, Usage,
};

const ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";
const MAX_TOKENS: u32 = 64_000;

/// The request body for one model call.
pub(super) fn to_body(request: Request<'_>) -> Value {
    let mut messages: Vec<_> = request.messages.iter().map(to_message).collect();
    // A cache point on the last message caches everything up to it: tools,
    // system prompt, and the conversation so far.
    if let Some(block) = messages
        .last_mut()
        .and_then(|message| message["content"].as_array_mut())
        .and_then(|content| content.last_mut())
    {
        block["cache_control"] = json!({"type": "ephemeral"});
    }

    let mut body = json!({
        "anthropic_version": ANTHROPIC_VERSION,
        "max_tokens": MAX_TOKENS,
        "messages": messages,
    });
    if !request.system.is_empty() {
        body["system"] = json!(request.system);
    }
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect();
    }
    body
}

/// Builds a reply from the stream's events, one at a time.
#[derive(Debug, Default)]
pub(super) struct Assembler {
    blocks: Vec<PartialBlock>,
    stop: Option<StopReason>,
    usage: Usage,
}

impl Assembler {
    /// Takes one stream event. Returns what to yield for it, if anything.
    pub(super) fn push(&mut self, event: &[u8]) -> Option<Result<Chunk, Error>> {
        let event: Value = match serde_json::from_slice(event) {
            Ok(event) => event,
            Err(error) => return Some(Err(Error::new(format!("malformed event: {error}")))),
        };
        match event["type"].as_str() {
            Some("message_start") => self.usage = to_usage(&event["message"]["usage"]),
            Some("content_block_start") => {
                self.blocks.push(PartialBlock::new(&event["content_block"]))
            }
            Some("content_block_delta") => return self.push_delta(&event["delta"]),
            Some("message_delta") => {
                self.stop = event["delta"]["stop_reason"].as_str().map(to_stop_reason);
                if let Some(output_tokens) = event["usage"]["output_tokens"].as_u64() {
                    self.usage.output_tokens = output_tokens;
                }
            }
            Some("message_stop") => return Some(Ok(Chunk::Done(self.take_reply()))),
            Some("error") => {
                let message = event["error"]["message"].as_str().unwrap_or("stream error");
                return Some(Err(Error::new(message)));
            }
            _ => {}
        }
        None
    }

    fn push_delta(&mut self, delta: &Value) -> Option<Result<Chunk, Error>> {
        let block = self.blocks.last_mut()?;
        match (block, delta["type"].as_str()) {
            (PartialBlock::Text(text), Some("text_delta")) => {
                let delta = delta["text"].as_str().unwrap_or_default();
                text.push_str(delta);
                return Some(Ok(Chunk::Text(delta.to_owned())));
            }
            (PartialBlock::ToolCall { json, .. }, Some("input_json_delta")) => {
                json.push_str(delta["partial_json"].as_str().unwrap_or_default());
            }
            (PartialBlock::Opaque(block), Some("thinking_delta")) => {
                append(block, "thinking", &delta["thinking"]);
            }
            (PartialBlock::Opaque(block), Some("signature_delta")) => {
                append(block, "signature", &delta["signature"]);
            }
            _ => {}
        }
        None
    }

    fn take_reply(&mut self) -> Reply {
        Reply {
            content: self.blocks.drain(..).map(PartialBlock::finish).collect(),
            stop: self.stop.take().unwrap_or(StopReason::EndTurn),
            usage: std::mem::take(&mut self.usage),
        }
    }
}

#[derive(Debug)]
enum PartialBlock {
    Text(String),
    ToolCall {
        id: String,
        name: String,
        json: String,
    },
    /// Thinking, redacted thinking, or anything else: kept as the provider
    /// sent it, to be sent back unchanged.
    Opaque(Value),
}

impl PartialBlock {
    fn new(start: &Value) -> Self {
        let field = |key: &str| start[key].as_str().unwrap_or_default().to_owned();
        match start["type"].as_str() {
            Some("text") => Self::Text(field("text")),
            Some("tool_use") => Self::ToolCall {
                id: field("id"),
                name: field("name"),
                json: String::new(),
            },
            _ => Self::Opaque(start.clone()),
        }
    }

    fn finish(self) -> Block {
        match self {
            Self::Text(text) => Block::Text(text),
            Self::ToolCall { id, name, json } => {
                let input = match json.as_str() {
                    "" => json!({}),
                    // Kept as the raw text, so the tool rejects it and the
                    // model sees an error result it can retry from.
                    json => serde_json::from_str(json).unwrap_or_else(|_| json.into()),
                };
                Block::ToolCall(ToolCall { id, name, input })
            }
            Self::Opaque(block) => Block::Opaque(block),
        }
    }
}

fn append(block: &mut Value, key: &str, delta: &Value) {
    let delta = delta.as_str().unwrap_or_default();
    match &mut block[key] {
        Value::String(text) => text.push_str(delta),
        other => *other = delta.into(),
    }
}

fn to_message(message: &Message) -> Value {
    match message {
        Message::User(text) => json!({
            "role": "user",
            "content": [{"type": "text", "text": text}],
        }),
        Message::Assistant(blocks) => json!({
            "role": "assistant",
            "content": blocks.iter().filter_map(to_content_block).collect::<Vec<_>>(),
        }),
        Message::ToolResults(results) => json!({
            "role": "user",
            "content": results.iter().map(to_tool_result).collect::<Vec<_>>(),
        }),
    }
}

fn to_content_block(block: &Block) -> Option<Value> {
    match block {
        Block::Text(text) => Some(json!({"type": "text", "text": text})),
        Block::ToolCall(call) => {
            // The API only takes an object; malformed input was answered
            // with an error result already.
            let input = if call.input.is_object() {
                call.input.clone()
            } else {
                json!({})
            };
            Some(json!({"type": "tool_use", "id": call.id, "name": call.name, "input": input}))
        }
        // Only blocks this adapter wrote carry a type.
        Block::Opaque(block) if block["type"].is_string() => Some(block.clone()),
        Block::Opaque(_) => None,
    }
}

fn to_tool_result(result: &ToolResult) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": result.call_id,
        "content": result.output,
        "is_error": result.is_error,
    })
}

fn to_stop_reason(reason: &str) -> StopReason {
    match reason {
        "tool_use" => StopReason::ToolUse,
        "max_tokens" | "model_context_window_exceeded" => StopReason::MaxTokens,
        "refusal" => StopReason::Refusal,
        _ => StopReason::EndTurn,
    }
}

/// Cached input is input too: reads and writes count toward the total.
fn to_usage(usage: &Value) -> Usage {
    let count = |key: &str| usage[key].as_u64().unwrap_or(0);
    Usage {
        input_tokens: count("input_tokens")
            + count("cache_read_input_tokens")
            + count("cache_creation_input_tokens"),
        output_tokens: count("output_tokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assemble(events: &[Value]) -> Reply {
        let mut assembler = Assembler::default();
        let last = events
            .iter()
            .filter_map(|event| assembler.push(event.to_string().as_bytes()))
            .last();
        match last {
            Some(Ok(Chunk::Done(reply))) => reply,
            other => panic!("expected a reply, got {other:?}"),
        }
    }

    fn tool_call_with_input(partial_json: &str) -> Reply {
        assemble(&[
            json!({"type": "content_block_start", "index": 0,
                   "content_block": {"type": "tool_use", "id": "t1", "name": "read", "input": {}}}),
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "input_json_delta", "partial_json": partial_json}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}}),
            json!({"type": "message_stop"}),
        ])
    }

    #[test]
    fn malformed_tool_input_is_kept_as_text() {
        let reply = tool_call_with_input("{\"path\": ");

        assert_eq!(
            reply.content[0].as_tool_call().unwrap().input,
            json!("{\"path\": ")
        );
    }

    #[test]
    fn malformed_tool_input_is_sent_back_as_an_empty_object() {
        let reply = tool_call_with_input("{\"path\": ");

        let block = to_content_block(&reply.content[0]).unwrap();

        assert_eq!(block["input"], json!({}));
    }

    #[test]
    fn tool_call_without_input_deltas_has_empty_input() {
        let reply = tool_call_with_input("");

        assert_eq!(reply.content[0].as_tool_call().unwrap().input, json!({}));
    }

    #[test]
    fn stop_reasons_map_to_neutral_ones() {
        let cases = [
            ("end_turn", StopReason::EndTurn),
            ("stop_sequence", StopReason::EndTurn),
            ("tool_use", StopReason::ToolUse),
            ("max_tokens", StopReason::MaxTokens),
            ("model_context_window_exceeded", StopReason::MaxTokens),
            ("refusal", StopReason::Refusal),
        ];

        for (reason, expected) in cases {
            assert_eq!(to_stop_reason(reason), expected, "{reason}");
        }
    }

    #[test]
    fn opaque_blocks_from_other_adapters_are_not_sent() {
        let block = Block::Opaque(json!({"thought_signature": "sig"}));

        assert_eq!(to_content_block(&block), None);
    }

    #[test]
    fn stream_error_event_is_an_error() {
        let mut assembler = Assembler::default();
        let event = json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}});

        let result = assembler.push(event.to_string().as_bytes());

        assert_eq!(result, Some(Err(Error::new("Overloaded"))));
    }
}
