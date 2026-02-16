use super::{
    ContentBlock, LlmProvider, LlmRequest, LlmResponse, MessagePart, Usage,
};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt, TryStreamExt};
use opencrust_common::{Error, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone)]
pub struct OllamaProvider {
    base_url: String,
    client: Client,
}

impl OllamaProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
            client: Client::new(),
        }
    }

    fn build_request_body(&self, request: &LlmRequest, stream: bool) -> Result<Value> {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|msg| {
                let (content, images) = match &msg.content {
                    MessagePart::Text(text) => (text.clone(), vec![]),
                    MessagePart::Parts(parts) => {
                        let mut text_parts = Vec::new();
                        let mut images = Vec::new();
                        for part in parts {
                            match part {
                                ContentBlock::Text { text } => text_parts.push(text.clone()),
                                ContentBlock::Image { url } => {
                                    // Ollama expects base64 encoded images.
                                    // LIMITATION: This implementation assumes the URL is a Data URI or
                                    // a raw Base64 string. It does NOT fetch remote HTTP URLs.
                                    // If a remote URL is provided, it will be passed as-is to Ollama,
                                    // which will likely fail or treat it as invalid base64.
                                    let b64 = if let Some(stripped) = url.strip_prefix("data:image/") {
                                        if let Some(idx) = stripped.find(";base64,") {
                                            stripped[idx + 8..].to_string()
                                        } else {
                                            url.clone()
                                        }
                                    } else {
                                        url.clone()
                                    };
                                    images.push(b64);
                                }
                                _ => {}
                            }
                        }
                        (text_parts.join("\n"), images)
                    }
                };

                let mut msg_obj = serde_json::json!({
                    "role": match msg.role {
                        super::ChatRole::System => "system",
                        super::ChatRole::User => "user",
                        super::ChatRole::Assistant => "assistant",
                        super::ChatRole::Tool => "tool",
                    },
                    "content": content,
                });

                if !images.is_empty() {
                    msg_obj["images"] = serde_json::json!(images);
                }

                msg_obj
            })
            .collect();

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
            "stream": stream,
        });

        let mut options = serde_json::Map::new();
        if let Some(temp) = request.temperature {
            options.insert("temperature".to_string(), serde_json::json!(temp));
        }
        if let Some(max_tokens) = request.max_tokens {
            options.insert("num_predict".to_string(), serde_json::json!(max_tokens));
        }

        #[allow(clippy::collapsible_if)]
        if let Some(obj) = body.as_object_mut() {
            if !options.is_empty() {
                obj.insert("options".to_string(), serde_json::Value::Object(options));
            }
        }

        Ok(body)
    }
}

#[derive(Deserialize)]
struct OllamaResponse {
    model: String,
    message: Option<OllamaMessage>,
    done: bool,
    #[serde(default)]
    eval_count: u32,
    #[serde(default)]
    prompt_eval_count: u32,
}

#[derive(Deserialize)]
struct OllamaMessage {
    content: String,
}

#[derive(Deserialize)]
struct OllamaModelsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn provider_id(&self) -> &str {
        "ollama"
    }

    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let body = self.build_request_body(request, false)?;
        let url = format!("{}/api/chat", self.base_url);

        let res = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("Ollama request failed: {}", e)))?;

        if !res.status().is_success() {
             return Err(Error::Agent(format!("Ollama error status: {}", res.status())));
        }

        let ollama_res: OllamaResponse = res
            .json()
            .await
            .map_err(|e| Error::Agent(format!("Failed to parse Ollama response: {}", e)))?;

        let content = if let Some(msg) = ollama_res.message {
            vec![ContentBlock::Text { text: msg.content }]
        } else {
            vec![]
        };

        Ok(LlmResponse {
            content,
            model: ollama_res.model,
            usage: Some(Usage {
                input_tokens: ollama_res.prompt_eval_count,
                output_tokens: ollama_res.eval_count,
            }),
            stop_reason: if ollama_res.done {
                Some("stop".to_string())
            } else {
                None
            },
        })
    }

    async fn stream_complete(
        &self,
        request: &LlmRequest,
    ) -> Result<BoxStream<'static, Result<LlmResponse>>> {
        let body = self.build_request_body(request, true)?;
        let url = format!("{}/api/chat", self.base_url);

        let res = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("Ollama request failed: {}", e)))?;

        if !res.status().is_success() {
             return Err(Error::Agent(format!("Ollama error status: {}", res.status())));
        }

        let stream = res
            .bytes_stream()
            .map_err(|e| Error::Agent(format!("Stream error: {}", e)));

        let stream: BoxStream<'static, Result<Bytes>> = Box::pin(stream);

        // unfold state: (stream, buffer)
        let stream = futures::stream::unfold((stream, Vec::new()), |(mut stream, mut buffer): (BoxStream<'static, Result<Bytes>>, Vec<u8>)| async move {
            loop {
                // Check if buffer contains a newline
                if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                    let line_bytes: Vec<u8> = buffer.drain(0..=pos).collect();
                    // Remove the trailing newline
                    let s = String::from_utf8_lossy(&line_bytes[..line_bytes.len()-1]).to_string();
                    if !s.is_empty() {
                         return Some((Ok(s), (stream, buffer)));
                    }
                    continue; // Loop to find next line or fetch more data
                }

                // Need more data
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        buffer.extend_from_slice(&chunk);
                        // Loop again to process buffer
                    }
                    Some(Err(e)) => return Some((Err(e), (stream, buffer))),
                    None => {
                        // End of stream, flush remaining buffer
                        if !buffer.is_empty() {
                             let line_bytes = buffer.clone();
                             buffer.clear();
                             let s = String::from_utf8_lossy(&line_bytes).to_string();
                             if !s.is_empty() {
                                 return Some((Ok(s), (stream, buffer)));
                             }
                        }
                        return None;
                    }
                }
            }
        });

        let output_stream = stream.map(|line_res: Result<String>| {
            let line = line_res?;
            let ollama_res: OllamaResponse = serde_json::from_str(&line)
                .map_err(|e| Error::Agent(format!("Failed to parse stream chunk: {}", e)))?;

             let content = if let Some(msg) = ollama_res.message {
                vec![ContentBlock::Text { text: msg.content }]
            } else {
                vec![]
            };

            Ok(Some(LlmResponse {
                content,
                model: ollama_res.model,
                usage: if ollama_res.done {
                    Some(Usage {
                        input_tokens: ollama_res.prompt_eval_count,
                        output_tokens: ollama_res.eval_count,
                    })
                } else {
                    None
                },
                 stop_reason: if ollama_res.done {
                    Some("stop".to_string())
                } else {
                    None
                },
            }))
        })
        .try_filter_map(|x| async move { Ok(x) });

        Ok(Box::pin(output_stream))
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url);
        let res = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("Failed to list models: {}", e)))?;

        if !res.status().is_success() {
             return Err(Error::Agent(format!("Ollama error status: {}", res.status())));
        }

        let models_res: OllamaModelsResponse = res
            .json()
            .await
            .map_err(|e| Error::Agent(format!("Failed to parse models response: {}", e)))?;

        Ok(models_res.models.into_iter().map(|m| m.name).collect())
    }

    async fn health_check(&self) -> Result<bool> {
        match self.list_models().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_request_serialization() {
        let provider = OllamaProvider::new(None);
        let req = LlmRequest {
            model: "llama3".to_string(),
            messages: vec![
                super::super::ChatMessage {
                    role: super::super::ChatRole::User,
                    content: super::super::MessagePart::Text("Hello".to_string()),
                }
            ],
            system: None,
            max_tokens: Some(100),
            temperature: Some(0.7),
            tools: vec![],
        };

        let body = provider.build_request_body(&req, false).unwrap();

        assert_eq!(body["model"], "llama3");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
        assert_eq!(body["options"]["temperature"], 0.7);
        assert_eq!(body["options"]["num_predict"], 100);
    }

    // Integration tests with axum
    use axum::{routing::{get, post}, Json, Router};
    use tokio::sync::oneshot;
    use serde_json::Value;

    async fn run_mock_server() -> (String, oneshot::Sender<()>) {
        let (tx, rx) = oneshot::channel::<()>();

        let app = Router::new()
            .route("/api/tags", get(|| async {
                Json(json!({
                    "models": [
                        { "name": "llama3:latest" },
                        { "name": "mistral:latest" }
                    ]
                }))
            }))
            .route("/api/chat", post(|Json(payload): Json<Value>| async move {
                 // Check if stream
                 let stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
                 if stream {
                     // Streaming response - we'll simulate by returning a long string with newlines
                     "{\"model\":\"llama3\",\"created_at\":\"...\",\"message\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"done\":false}\n{\"model\":\"llama3\",\"created_at\":\"...\",\"message\":{\"role\":\"assistant\",\"content\":\" World\"},\"done\":true}".to_string()
                 } else {
                     // Non-streaming
                     let res = json!({
                        "model": "llama3",
                        "created_at": "...",
                        "message": {
                            "role": "assistant",
                            "content": "Hello World"
                        },
                        "done": true,
                        "prompt_eval_count": 10,
                        "eval_count": 5
                     });
                     serde_json::to_string(&res).unwrap()
                 }
            }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await
                .unwrap();
        });

        (url, tx)
    }

    #[tokio::test]
    async fn test_list_models() {
        let (url, stop) = run_mock_server().await;
        let provider = OllamaProvider::new(Some(url));

        let models = provider.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
        assert!(models.contains(&"llama3:latest".to_string()));

        let _ = stop.send(());
    }

    #[tokio::test]
    async fn test_complete() {
        let (url, stop) = run_mock_server().await;
        let provider = OllamaProvider::new(Some(url));

        let req = LlmRequest {
            model: "llama3".to_string(),
            messages: vec![
                super::super::ChatMessage {
                    role: super::super::ChatRole::User,
                    content: super::super::MessagePart::Text("Hi".to_string()),
                }
            ],
            system: None,
            max_tokens: None,
            temperature: None,
            tools: vec![],
        };

        let res = provider.complete(&req).await.unwrap();

        // Check content
        match &res.content[0] {
            super::super::ContentBlock::Text { text } => assert_eq!(text, "Hello World"),
            _ => panic!("Expected text content"),
        }

        let _ = stop.send(());
    }

    #[tokio::test]
    async fn test_stream_complete() {
        let (url, stop) = run_mock_server().await;
        let provider = OllamaProvider::new(Some(url));

        let req = LlmRequest {
            model: "llama3".to_string(),
            messages: vec![],
            system: None,
            max_tokens: None,
            temperature: None,
            tools: vec![],
        };

        let mut stream = provider.stream_complete(&req).await.unwrap();

        let mut full_text = String::new();
        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.unwrap();
             match &chunk.content[0] {
                super::super::ContentBlock::Text { text } => full_text.push_str(text),
                _ => {},
            }
        }

        assert_eq!(full_text, "Hello World");

        let _ = stop.send(());
    }
}
