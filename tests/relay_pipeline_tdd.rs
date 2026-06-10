use galaxy_router::api::handlers::admin::channels::EndpointType;
use galaxy_router::relay::converter::{RelayPipeline, RelayPipelineRequest};
use serde_json::json;

#[test]
fn relay_pipeline_passthrough_keeps_openai_chat_request_body_unchanged() {
    let body = json!({
        "model": "gpt-4o",
        "messages": [
            {"role": "system", "content": "Be concise"},
            {"role": "user", "content": "hello"}
        ],
        "stream": false,
        "temperature": 0.2
    });

    let prepared = RelayPipeline::prepare_request(RelayPipelineRequest {
        client_endpoint: EndpointType::OpenAiChat,
        upstream_endpoint: EndpointType::OpenAiChat,
        requested_model: "gpt-4o".to_string(),
        upstream_model: "gpt-4o".to_string(),
        body: body.clone(),
    })
    .expect("passthrough prepare succeeds");

    assert!(!prepared.needs_conversion);
    assert_eq!(prepared.body, body);
    assert_eq!(prepared.client_endpoint, EndpointType::OpenAiChat);
    assert_eq!(prepared.upstream_endpoint, EndpointType::OpenAiChat);
    assert_eq!(prepared.requested_model, "gpt-4o");
    assert_eq!(prepared.upstream_model, "gpt-4o");
}

#[test]
fn relay_pipeline_passthrough_rewrites_model_without_other_body_changes() {
    let body = json!({
        "model": "alias-model",
        "messages": [{"role": "user", "content": "hello"}],
        "metadata": {"trace": "abc"}
    });

    let prepared = RelayPipeline::prepare_request(RelayPipelineRequest {
        client_endpoint: EndpointType::OpenAiChat,
        upstream_endpoint: EndpointType::OpenAiChat,
        requested_model: "alias-model".to_string(),
        upstream_model: "real-upstream-model".to_string(),
        body,
    })
    .expect("passthrough prepare succeeds");

    assert!(!prepared.needs_conversion);
    assert_eq!(prepared.body["model"], "real-upstream-model");
    assert_eq!(prepared.body["messages"][0]["content"], "hello");
    assert_eq!(prepared.body["metadata"]["trace"], "abc");
}

#[test]
fn relay_pipeline_passthrough_keeps_openai_chat_response_body_unchanged() {
    let body = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "pong"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}
    });

    let finalized = RelayPipeline::finalize_response(
        EndpointType::OpenAiChat,
        EndpointType::OpenAiChat,
        body.clone(),
    )
    .expect("passthrough finalize succeeds");

    assert!(!finalized.was_converted);
    assert_eq!(finalized.body, body);
}

#[tokio::test]
async fn relay_pipeline_conversion_transforms_openai_chat_request_to_anthropic_request() {
    let body = json!({
        "model": "alias-claude",
        "messages": [
            {"role": "system", "content": "Be concise"},
            {"role": "user", "content": "hello"}
        ],
        "max_completion_tokens": 123,
        "temperature": 0.3,
        "stream": false
    });

    let prepared = RelayPipeline::prepare_request_async(RelayPipelineRequest {
        client_endpoint: EndpointType::OpenAiChat,
        upstream_endpoint: EndpointType::Anthropic,
        requested_model: "alias-claude".to_string(),
        upstream_model: "claude-3-5-sonnet".to_string(),
        body,
    })
    .await
    .expect("conversion prepare succeeds");

    assert!(prepared.needs_conversion);
    assert_eq!(prepared.body["model"], "claude-3-5-sonnet");
    assert_eq!(prepared.body["system"], "Be concise");
    assert_eq!(prepared.body["messages"][0]["role"], "user");
    assert_eq!(prepared.body["messages"][0]["content"], "hello");
    assert_eq!(prepared.body["max_tokens"], 123);
    assert_eq!(prepared.body["temperature"], 0.3);
    assert_eq!(prepared.body["stream"], false);
}

#[tokio::test]
async fn relay_pipeline_conversion_transforms_anthropic_response_to_openai_chat_response() {
    let body = json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-3-5-sonnet",
        "content": [{"type": "text", "text": "pong"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 3, "output_tokens": 1}
    });

    let finalized = RelayPipeline::finalize_response_async(
        EndpointType::OpenAiChat,
        EndpointType::Anthropic,
        body,
        200,
    )
    .await
    .expect("conversion finalize succeeds");

    assert!(finalized.was_converted);
    assert_eq!(finalized.body["id"], "msg_1");
    assert_eq!(finalized.body["object"], "chat.completion");
    assert_eq!(finalized.body["model"], "claude-3-5-sonnet");
    assert_eq!(finalized.body["choices"][0]["message"]["role"], "assistant");
    assert_eq!(finalized.body["choices"][0]["message"]["content"], "pong");
    assert_eq!(finalized.body["choices"][0]["finish_reason"], "stop");
    assert_eq!(finalized.body["usage"]["prompt_tokens"], 3);
    assert_eq!(finalized.body["usage"]["completion_tokens"], 1);
    assert_eq!(finalized.body["usage"]["total_tokens"], 4);
}
