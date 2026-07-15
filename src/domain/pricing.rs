//! 模型定价/元数据领域模型（纯数据，零框架依赖）。
//!
//! 从 service/pricing/model.rs 归位（C6）：repository 层需引用 ModelInfo，
//! 留在 service 会造成 repository→service 反向依赖。

use serde::{Deserialize, Serialize};

/// 模型信息（定价 + 元数据）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model: String,
    pub provider: String,
    pub mode: String,
    pub input_price: Option<f64>,
    pub output_price: Option<f64>,
    pub cache_read_price: Option<f64>,
    pub cache_creation_price: Option<f64>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub supports_function_calling: Option<bool>,
    pub supports_reasoning: Option<bool>,
    pub supports_vision: Option<bool>,
    pub supports_pdf_input: Option<bool>,
    pub supports_prompt_caching: Option<bool>,
    pub supports_system_messages: Option<bool>,
    pub supports_tool_choice: Option<bool>,
}
