use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::{method, path};

/// 创建一个 mock OpenAI chat completions 响应
#[allow(dead_code)]
pub async fn spawn_openai_chat_mock(response_body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&server)
        .await;
    server
}

/// 创建一个 mock Anthropic messages 响应
#[allow(dead_code)]
pub async fn spawn_anthropic_messages_mock(response_body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&server)
        .await;
    server
}

/// 创建一个 mock OpenAI embeddings 响应
#[allow(dead_code)]
pub async fn spawn_openai_embeddings_mock() -> MockServer {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "object": "list",
        "data": [{"object": "embedding", "embedding": [0.1, 0.2, 0.3], "index": 0}],
        "model": "text-embedding-3-small",
        "usage": {"prompt_tokens": 5, "total_tokens": 5}
    });
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

/// 创建一个 mock OpenAI images 响应
#[allow(dead_code)]
pub async fn spawn_openai_images_mock() -> MockServer {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "created": 1234567890,
        "data": [{"url": "https://example.com/image.png"}]
    });
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

/// 创建一个 mock OpenAI responses 响应
#[allow(dead_code)]
pub async fn spawn_openai_responses_mock() -> MockServer {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "id": "resp-001",
        "object": "response",
        "status": "completed",
        "output": [{"type": "message", "role": "assistant", "content": []}]
    });
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    server
}

/// 创建一个返回错误的 mock
#[allow(dead_code)]
pub async fn spawn_error_mock(status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(status))
        .mount(&server)
        .await;
    server
}
