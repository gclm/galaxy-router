//! 模型发现 service：从上游端点拉取可用模型列表（多端点回退 + 协议分页）。
//!
//! 从 api/handlers/admin/fetch_models.rs 归位（D4）。零依赖（仅 reqwest::Client + domain::channel）。
//! 多端点回退 + endpoint_type 分派 + FetchError 分类在此；handler 转 ApiError。

use reqwest::Client;
use serde::Deserialize;

use crate::domain::channel::{EndpointConfig, EndpointType};

#[derive(Clone)]
pub struct ModelDiscovery {
    client: Client,
}

impl ModelDiscovery {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// 多端点回退拉取：逐个端点尝试，首个成功即返回；AuthFailed 立即失败，其余累积到 last_error。
    pub async fn fetch_models(
        &self,
        endpoints: &[EndpointConfig],
        api_key: &str,
    ) -> Result<Vec<String>, FetchError> {
        let mut last_error: Option<FetchError> = None;
        for endpoint in endpoints {
            let result = match endpoint.endpoint_type {
                EndpointType::Anthropic => {
                    fetch_anthropic_models(&self.client, &endpoint.base_url, api_key).await
                }
                EndpointType::Gemini => {
                    fetch_gemini_models(&self.client, &endpoint.base_url, api_key).await
                }
                _ => fetch_openai_models(&self.client, &endpoint.base_url, api_key).await,
            };
            match result {
                Ok(models) => return Ok(models),
                Err(FetchError::AuthFailed) => return Err(FetchError::AuthFailed),
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| FetchError::Other("所有端点均失败".into())))
    }
}

/// 模型发现错误
pub enum FetchError {
    AuthFailed,
    Http(reqwest::StatusCode),
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl From<reqwest::Error> for FetchError {
    fn from(e: reqwest::Error) -> Self {
        if e.status().is_some_and(|s| s == 401 || s == 403) {
            FetchError::AuthFailed
        } else if let Some(status) = e.status() {
            FetchError::Http(status)
        } else {
            FetchError::Other(e.into())
        }
    }
}

/// 检查响应状态
fn check_response(resp: &reqwest::Response) -> Result<(), FetchError> {
    if resp.status() == 401 || resp.status() == 403 {
        return Err(FetchError::AuthFailed);
    }
    if !resp.status().is_success() {
        return Err(FetchError::Http(resp.status()));
    }
    Ok(())
}

// ── 响应 DTO ──

#[derive(Debug, Deserialize)]
struct OpenAIModelListResponse {
    data: Vec<OpenAIModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelListResponse {
    data: Vec<AnthropicModel>,
    has_more: bool,
    last_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GeminiModelListResponse {
    models: Vec<GeminiModel>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiModel {
    name: String,
}

// ── 协议 fetcher ──

/// 获取 OpenAI 兼容模型列表
async fn fetch_openai_models(
    client: &Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>, FetchError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(FetchError::from)?;

    check_response(&resp)?;

    let result: OpenAIModelListResponse = resp.json().await.map_err(FetchError::from)?;
    Ok(result.data.into_iter().map(|m| m.id).collect())
}

/// 获取 Anthropic 模型列表（含 after_id 分页）
async fn fetch_anthropic_models(
    client: &Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>, FetchError> {
    let mut all_models = Vec::with_capacity(100);
    let mut after_id: Option<String> = None;

    loop {
        let mut url = format!("{}/models", base_url.trim_end_matches('/'));
        if let Some(ref after) = after_id {
            url = format!("{}?after_id={}", url, after);
        }

        let resp = client
            .get(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(FetchError::from)?;

        check_response(&resp)?;

        let result: AnthropicModelListResponse = resp.json().await.map_err(FetchError::from)?;
        all_models.extend(result.data.into_iter().map(|m| m.id));

        if !result.has_more {
            break;
        }
        after_id = result.last_id;
    }

    Ok(all_models)
}

/// 获取 Gemini 模型列表（含 pageToken 分页）
async fn fetch_gemini_models(
    client: &Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>, FetchError> {
    let mut all_models = Vec::with_capacity(100);
    let mut page_token: Option<String> = None;

    loop {
        let mut url = format!("{}/models", base_url.trim_end_matches('/'));
        if let Some(ref token) = page_token {
            url = format!("{}?pageToken={}", url, token);
        }

        let resp = client
            .get(&url)
            .header("x-goog-api-key", api_key)
            .send()
            .await
            .map_err(FetchError::from)?;

        check_response(&resp)?;

        let result: GeminiModelListResponse = resp.json().await.map_err(FetchError::from)?;
        all_models.extend(result.models.into_iter().map(|m| {
            m.name
                .strip_prefix("models/")
                .unwrap_or(&m.name)
                .to_string()
        }));

        match result.next_page_token {
            Some(token) => page_token = Some(token),
            None => break,
        }
    }

    Ok(all_models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_openai_models() {
        let json = r#"{"data": [{"id": "gpt-4"}, {"id": "gpt-4-turbo"}]}"#;
        let result: OpenAIModelListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(result.data.len(), 2);
        assert_eq!(result.data[0].id, "gpt-4");
        assert_eq!(result.data[1].id, "gpt-4-turbo");
    }

    #[test]
    fn test_parse_anthropic_models() {
        let json =
            r#"{"data": [{"id": "claude-sonnet-4-20250514"}], "has_more": false, "last_id": null}"#;
        let result: AnthropicModelListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(result.data.len(), 1);
        assert_eq!(result.data[0].id, "claude-sonnet-4-20250514");
        assert!(!result.has_more);
    }

    #[test]
    fn test_parse_gemini_models() {
        let json = r#"{"models": [{"name": "models/gemini-pro"}], "nextPageToken": null}"#;
        let result: GeminiModelListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].name, "models/gemini-pro");
    }

    #[test]
    fn test_gemini_model_name_strip_prefix() {
        let name = "models/gemini-pro";
        let stripped = name.strip_prefix("models/").unwrap_or(name);
        assert_eq!(stripped, "gemini-pro");
    }
}
