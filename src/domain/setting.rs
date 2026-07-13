//! Settings 领域模型（v1.1.2 从 settings.rs 抽出）。

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SettingResponse {
    pub key: String,
    pub category: String,
    pub value: String,
    pub description: Option<String>,
}
