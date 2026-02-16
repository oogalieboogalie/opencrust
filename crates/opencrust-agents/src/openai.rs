use crate::providers::{
    ChatMessage, ChatRole, ContentBlock, LlmProvider, LlmRequest, LlmResponse, LlmStream,
    LlmStreamResponse, MessagePart, StreamContent, ToolDefinition, ToolUseDelta, Usage,
};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use opencrust_common::{Error, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::pin::Pin;

#[derive(Clone)]
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn provider_id(&self) -> &str {
        "openai"
    }

    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let openai_request = self.convert_request(request, false)?;

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Agent(format!("OpenAI API error: {}", error_text)));
        }

        let openai_response: OpenAiResponse = response.json().await
            .map_err(|e| Error::Agent(format!("Failed to parse OpenAI response: {}", e)))?;

        self.convert_response(openai_response)
    }

    async fn complete_stream(&self, request: &LlmRequest) -> Result<LlmStream> {
        let url = format!("{}/chat/completions", self.base_url);
        let openai_request = self.convert_request(request, true)?;

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Agent(format!("OpenAI API error: {}", error_text)));
        }

        let stream = response.bytes_stream();
        let parser = SseParser::new(stream);
        Ok(Box::pin(parser))
    }

    async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/models", self.base_url);
        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await;

        match response {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

impl OpenAiProvider {
    fn convert_request(&self, request: &LlmRequest, stream: bool) -> Result<OpenAiRequest> {
        let mut messages = Vec::new();

        if let Some(system_prompt) = &request.system {
            messages.push(OpenAiMessage::System { content: system_prompt.clone() });
        }

        for msg in &request.messages {
            let converted = self.convert_message(msg)?;
            messages.push(converted);
        }

        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.iter().map(|t| OpenAiTool {
                kind: "function".to_string(),
                function: OpenAiFunctionDefinition {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            }).collect())
        };

        let stream_options = if stream {
            Some(OpenAiStreamOptions { include_usage: true })
        } else {
            None
        };

        Ok(OpenAiRequest {
            model: request.model.clone(),
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tools,
            stream,
            stream_options,
        })
    }

    fn convert_message(&self, msg: &ChatMessage) -> Result<OpenAiMessage> {
        match msg.role {
            ChatRole::System => {
                let content = match &msg.content {
                    MessagePart::Text(t) => t.clone(),
                    MessagePart::Parts(parts) => {
                        parts.iter().filter_map(|p| match p {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        }).collect::<Vec<_>>().join("\n")
                    }
                };
                Ok(OpenAiMessage::System { content })
            },
            ChatRole::User => {
                let content = match &msg.content {
                    MessagePart::Text(t) => OpenAiUserContent::Text(t.clone()),
                    MessagePart::Parts(parts) => {
                        let mut open_ai_parts = Vec::new();
                        for part in parts {
                            match part {
                                ContentBlock::Text { text } => {
                                    open_ai_parts.push(OpenAiContentPart::Text { text: text.clone() });
                                },
                                ContentBlock::Image { url } => {
                                    open_ai_parts.push(OpenAiContentPart::ImageUrl {
                                        image_url: OpenAiImageUrl { url: url.clone() },
                                    });
                                },
                                _ => {}
                            }
                        }
                        OpenAiUserContent::Parts(open_ai_parts)
                    }
                };
                Ok(OpenAiMessage::User { content })
            },
            ChatRole::Assistant => {
                let mut content_str = None;
                let mut tool_calls = Vec::new();

                match &msg.content {
                    MessagePart::Text(t) => {
                        content_str = Some(t.clone());
                    },
                    MessagePart::Parts(parts) => {
                        let mut text_parts = Vec::new();
                        for part in parts {
                            match part {
                                ContentBlock::Text { text } => text_parts.push(text.clone()),
                                ContentBlock::ToolUse { id, name, input } => {
                                    tool_calls.push(OpenAiToolCall {
                                        id: id.clone(),
                                        kind: "function".to_string(),
                                        function: OpenAiFunctionCall {
                                            name: name.clone(),
                                            arguments: serde_json::to_string(&input).unwrap_or_default(),
                                        },
                                    });
                                },
                                _ => {}
                            }
                        }
                        if !text_parts.is_empty() {
                            content_str = Some(text_parts.join("\n"));
                        }
                    }
                }

                Ok(OpenAiMessage::Assistant {
                    content: content_str,
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                })
            },
            ChatRole::Tool => {
                let (tool_call_id, content) = match &msg.content {
                    MessagePart::Parts(parts) => {
                         parts.iter().find_map(|p| match p {
                             ContentBlock::ToolResult { tool_use_id, content } => Some((tool_use_id.clone(), content.clone())),
                             _ => None
                         }).ok_or_else(|| Error::Agent("Tool message missing ToolResult content".to_string()))?
                    },
                     _ => return Err(Error::Agent("Tool message must have Parts content with ToolResult".to_string())),
                };

                Ok(OpenAiMessage::Tool {
                    tool_call_id,
                    content,
                })
            }
        }
    }

    fn convert_response(&self, response: OpenAiResponse) -> Result<LlmResponse> {
        let choice = response.choices.first().ok_or_else(|| Error::Agent("No choices in response".to_string()))?;
        let message = &choice.message;

        let mut content_blocks = Vec::new();

        if let Some(text) = &message.content {
            content_blocks.push(ContentBlock::Text { text: text.clone() });
        }

        if let Some(tool_calls) = &message.tool_calls {
            for tc in tool_calls {
                let input_json: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::String(tc.function.arguments.clone()));

                content_blocks.push(ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input: input_json,
                });
            }
        }

        Ok(LlmResponse {
            content: content_blocks,
            model: response.model.clone(),
            usage: response.usage.map(|u| Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
            }),
            stop_reason: Some(choice.finish_reason.clone()),
        })
    }
}

// Request Types
#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OpenAiStreamOptions>,
}

#[derive(Serialize)]
struct OpenAiStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum OpenAiMessage {
    System { content: String },
    User { content: OpenAiUserContent },
    Assistant {
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<OpenAiToolCall>>
    },
    Tool {
        tool_call_id: String,
        content: String
    },
}

#[derive(Serialize)]
#[serde(untagged)]
enum OpenAiUserContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAiContentPart {
    Text { text: String },
    ImageUrl { image_url: OpenAiImageUrl },
}

#[derive(Serialize)]
struct OpenAiImageUrl {
    url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: OpenAiFunctionCall,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    kind: String,
    function: OpenAiFunctionDefinition,
}

#[derive(Serialize)]
struct OpenAiFunctionDefinition {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// Response Types
#[derive(Deserialize)]
struct OpenAiResponse {
    id: String,
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    finish_reason: String,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

// Stream Parser
struct SseParser {
    stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buffer: Vec<u8>,
    queue: std::collections::VecDeque<Result<LlmStreamResponse>>,
}

impl SseParser {
    fn new(stream: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static) -> Self {
        Self {
            stream: Box::pin(stream),
            buffer: Vec::new(),
            queue: std::collections::VecDeque::new(),
        }
    }
}

impl Stream for SseParser {
    type Item = Result<LlmStreamResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return std::task::Poll::Ready(Some(item));
            }

            match self.stream.as_mut().poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    self.buffer.extend_from_slice(&chunk);

                    loop {
                        let mut pos = None;
                        let mut len = 0;
                        if let Some(p) = self.buffer.windows(2).position(|w| w == b"\n\n") {
                            pos = Some(p);
                            len = 2;
                        } else if let Some(p) = self.buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            pos = Some(p);
                            len = 4;
                        }

                        if let Some(p) = pos {
                            let msg_bytes = self.buffer.drain(..p).collect::<Vec<u8>>();
                            self.buffer.drain(..len); // remove delimiter

                            if let Ok(msg_str) = String::from_utf8(msg_bytes) {
                            for line in msg_str.lines() {
                                if let Some(data) = line.strip_prefix("data: ") {
                                    if data.trim() == "[DONE]" {
                                        continue;
                                    }
                                    match serde_json::from_str::<OpenAiStreamChunk>(data) {
                                        Ok(chunk) => {
                                            for choice in chunk.choices {
                                                let mut yielded = false;

                                                if let Some(content) = choice.delta.content {
                                                    self.queue.push_back(Ok(LlmStreamResponse {
                                                        delta: StreamContent::Text(content),
                                                        usage: None,
                                                        stop_reason: choice.finish_reason.clone(),
                                                    }));
                                                    yielded = true;
                                                }

                                                if let Some(tool_calls) = choice.delta.tool_calls {
                                                    for tc in tool_calls {
                                                        self.queue.push_back(Ok(LlmStreamResponse {
                                                            delta: StreamContent::ToolUse(ToolUseDelta {
                                                                index: tc.index,
                                                                id: tc.id.clone(),
                                                                name: tc.function.as_ref().and_then(|f| f.name.clone()),
                                                                input: tc.function.as_ref().and_then(|f| f.arguments.clone()).unwrap_or_default(),
                                                            }),
                                                            usage: None,
                                                            stop_reason: choice.finish_reason.clone(),
                                                        }));
                                                        yielded = true;
                                                    }
                                                }

                                                if !yielded {
                                                    if let Some(reason) = choice.finish_reason {
                                                        self.queue.push_back(Ok(LlmStreamResponse {
                                                            delta: StreamContent::Text(String::new()),
                                                            usage: None,
                                                            stop_reason: Some(reason),
                                                        }));
                                                    }
                                                }
                                            }

                                            if let Some(usage) = chunk.usage {
                                                self.queue.push_back(Ok(LlmStreamResponse {
                                                    delta: StreamContent::Text(String::new()),
                                                    usage: Some(Usage {
                                                        input_tokens: usage.prompt_tokens,
                                                        output_tokens: usage.completion_tokens,
                                                    }),
                                                    stop_reason: None,
                                                }));
                                            }
                                        },
                                        Err(e) => {
                                            self.queue.push_back(Err(Error::Agent(format!("JSON parse error: {}", e))));
                                        }
                                    }
                                }
                            }
                        } else {
                            // Invalid UTF-8, drop
                        }
                    } else {
                        break;
                    }
                }
            }
                std::task::Poll::Ready(Some(Err(e))) => {
                    return std::task::Poll::Ready(Some(Err(Error::Agent(format!("Stream error: {}", e)))));
                }
                std::task::Poll::Ready(None) => {
                    if !self.queue.is_empty() {
                         continue;
                    }
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

// Stream Response Types
#[derive(Deserialize)]
struct OpenAiStreamChunk {
    id: String,
    model: String,
    choices: Vec<OpenAiStreamChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
    finish_reason: Option<String>,
    index: u32,
}

#[derive(Deserialize)]
struct OpenAiStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiStreamToolCall {
    index: u32,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<OpenAiStreamFunctionCall>,
}

#[derive(Deserialize)]
struct OpenAiStreamFunctionCall {
    name: Option<String>,
    arguments: Option<String>,
}
