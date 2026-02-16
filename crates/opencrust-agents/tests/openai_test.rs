use opencrust_agents::{
    ChatRole, ContentBlock, LlmProvider, LlmRequest, MessagePart, OpenAiProvider, ToolDefinition, StreamContent,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use serde_json::json;
use futures::StreamExt;

#[tokio::test]
async fn test_openai_completion() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "chatcmpl-123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "gpt-3.5-turbo-0613",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello there!",
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 9,
            "completion_tokens": 12,
            "total_tokens": 21
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    let provider = OpenAiProvider::new("test-key".to_string(), Some(mock_server.uri()));
    let request = LlmRequest {
        model: "gpt-3.5-turbo".to_string(),
        messages: vec![opencrust_agents::ChatMessage {
            role: ChatRole::User,
            content: MessagePart::Text("Hello".to_string()),
        }],
        system: Some("You are a helpful assistant.".to_string()),
        max_tokens: None,
        temperature: None,
        tools: vec![],
    };

    let response = provider.complete(&request).await.unwrap();

    assert_eq!(response.content.len(), 1);
    match &response.content[0] {
        ContentBlock::Text { text } => assert_eq!(text, "Hello there!"),
        _ => panic!("Expected text content"),
    }
}

#[tokio::test]
async fn test_openai_tool_use() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "chatcmpl-123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "gpt-3.5-turbo-0613",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\": \"Boston\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 9,
            "completion_tokens": 12,
            "total_tokens": 21
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    let provider = OpenAiProvider::new("test-key".to_string(), Some(mock_server.uri()));
    let request = LlmRequest {
        model: "gpt-3.5-turbo".to_string(),
        messages: vec![opencrust_agents::ChatMessage {
            role: ChatRole::User,
            content: MessagePart::Text("What is the weather in Boston?".to_string()),
        }],
        system: None,
        max_tokens: None,
        temperature: None,
        tools: vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: "Get weather".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                }
            }),
        }],
    };

    let response = provider.complete(&request).await.unwrap();

    assert_eq!(response.content.len(), 1);
    match &response.content[0] {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "call_abc123");
            assert_eq!(name, "get_weather");
            assert_eq!(input["location"], "Boston");
        },
        _ => panic!("Expected tool use"),
    }
}

#[tokio::test]
async fn test_openai_stream() {
    let mock_server = MockServer::start().await;

    let chunk1 = json!({
        "id": "chatcmpl-123",
        "object": "chat.completion.chunk",
        "created": 1677652288,
        "model": "gpt-3.5-turbo-0613",
        "choices": [{
            "index": 0,
            "delta": {"content": "Hello"},
            "finish_reason": null
        }]
    });

    let chunk2 = json!({
        "id": "chatcmpl-123",
        "object": "chat.completion.chunk",
        "created": 1677652288,
        "model": "gpt-3.5-turbo-0613",
        "choices": [{
            "index": 0,
            "delta": {"content": " World"},
            "finish_reason": null
        }]
    });

    let chunk3 = json!({
        "id": "chatcmpl-123",
        "object": "chat.completion.chunk",
        "created": 1677652288,
        "model": "gpt-3.5-turbo-0613",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    });

    let chunk4 = json!({
         "id": "chatcmpl-123",
         "model": "gpt-3.5-turbo-0613",
         "choices": [],
         "usage": {
             "prompt_tokens": 5,
             "completion_tokens": 7,
             "total_tokens": 12
         }
    });

    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        chunk1.to_string(),
        chunk2.to_string(),
        chunk3.to_string(),
        chunk4.to_string()
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&mock_server)
        .await;

    let provider = OpenAiProvider::new("test-key".to_string(), Some(mock_server.uri()));
    let request = LlmRequest {
        model: "gpt-3.5-turbo".to_string(),
        messages: vec![opencrust_agents::ChatMessage {
            role: ChatRole::User,
            content: MessagePart::Text("Hi".to_string()),
        }],
        system: None,
        max_tokens: None,
        temperature: None,
        tools: vec![],
    };

    let mut stream = provider.complete_stream(&request).await.unwrap();

    let mut full_text = String::new();
    let mut usage_found = false;

    while let Some(result) = stream.next().await {
        let response = result.unwrap();
        match response.delta {
            StreamContent::Text(t) => full_text.push_str(&t),
            _ => {},
        }
        if let Some(usage) = response.usage {
            assert_eq!(usage.input_tokens, 5);
            usage_found = true;
        }
    }

    assert_eq!(full_text, "Hello World");
    assert!(usage_found, "Usage should be reported");
}

#[tokio::test]
async fn test_openai_health_check() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let provider = OpenAiProvider::new("test-key".to_string(), Some(mock_server.uri()));
    assert!(provider.health_check().await.unwrap());
}
