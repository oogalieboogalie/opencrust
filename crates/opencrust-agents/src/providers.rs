use async_trait::async_trait;
use futures::Stream;
use opencrust_common::Result;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// Trait for LLM provider integrations (Anthropic, OpenAI, Ollama, etc.).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider identifier (e.g. "anthropic", "openai", "ollama").
    fn provider_id(&self) -> &str;

    /// Send a completion request and return the response.
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse>;

    /// Send a streaming completion request and return a stream of response chunks.
    async fn complete_stream(&self, request: &LlmRequest) -> Result<LlmStream>;

    /// Check if the provider is available and configured.
    async fn health_check(&self) -> Result<bool>;
}

pub type LlmStream = Pin<Box<dyn Stream<Item = Result<LlmStreamResponse>> + Send>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub system: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: MessagePart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessagePart {
    Text(String),
    Parts(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { url: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub usage: Option<Usage>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStreamResponse {
    pub delta: StreamContent,
    pub usage: Option<Usage>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamContent {
    Text(String),
    ToolUse(ToolUseDelta),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseDelta {
    pub index: u32,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: String, // Partial JSON
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}
