//! ModelInfo 数据访问层：model_info 表的 load/upsert（C6 新建）。
//!
//! model_info 是最后一个无 repository 的表；ModelRegistry（service）改持本 repo，
//! 不再直连 pool。route_repository 的 provider 识别跨表读 model_info 留 D3 移 service。

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::domain::pricing::ModelInfo;

type ModelInfoRow = (
    String,
    String,
    String,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<i64>,
    Option<i64>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
);

#[async_trait]
pub trait ModelInfoRepository: Send + Sync {
    /// 加载全部模型信息。
    async fn load_all(&self) -> Result<Vec<ModelInfo>, sqlx::Error>;
    /// upsert 模型信息（source = "remote" | "manual"）。
    async fn upsert(&self, info: &ModelInfo, source: &str) -> Result<(), sqlx::Error>;
}

pub struct SqliteModelInfoRepository {
    pool: SqlitePool,
}

impl SqliteModelInfoRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn row_to_info(row: ModelInfoRow) -> ModelInfo {
    ModelInfo {
        model: row.0,
        provider: row.1,
        mode: row.2,
        input_price: row.3,
        output_price: row.4,
        cache_read_price: row.5,
        cache_creation_price: row.6,
        max_input_tokens: row.7,
        max_output_tokens: row.8,
        supports_function_calling: row.9,
        supports_reasoning: row.10,
        supports_vision: row.11,
        supports_pdf_input: row.12,
        supports_prompt_caching: row.13,
        supports_system_messages: row.14,
        supports_tool_choice: row.15,
    }
}

#[async_trait]
impl ModelInfoRepository for SqliteModelInfoRepository {
    async fn load_all(&self) -> Result<Vec<ModelInfo>, sqlx::Error> {
        let rows: Vec<ModelInfoRow> = sqlx::query_as(
            "SELECT model, provider, mode,
                    input_price, output_price, cache_read_price, cache_creation_price,
                    max_input_tokens, max_output_tokens,
                    supports_function_calling, supports_reasoning, supports_vision,
                    supports_pdf_input, supports_prompt_caching,
                    supports_system_messages, supports_tool_choice
             FROM model_info",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_info).collect())
    }

    async fn upsert(&self, info: &ModelInfo, source: &str) -> Result<(), sqlx::Error> {
        let id = crate::util::id::generate_id();
        sqlx::query(
            r#"INSERT INTO model_info (
                id, model, provider, mode,
                input_price, output_price, cache_read_price, cache_creation_price,
                max_input_tokens, max_output_tokens,
                supports_function_calling, supports_reasoning, supports_vision,
                supports_pdf_input, supports_prompt_caching,
                supports_system_messages, supports_tool_choice,
                source
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(model) DO UPDATE SET
                provider = excluded.provider, mode = excluded.mode,
                input_price = excluded.input_price, output_price = excluded.output_price,
                cache_read_price = excluded.cache_read_price, cache_creation_price = excluded.cache_creation_price,
                max_input_tokens = excluded.max_input_tokens, max_output_tokens = excluded.max_output_tokens,
                supports_function_calling = excluded.supports_function_calling,
                supports_reasoning = excluded.supports_reasoning,
                supports_vision = excluded.supports_vision,
                supports_pdf_input = excluded.supports_pdf_input,
                supports_prompt_caching = excluded.supports_prompt_caching,
                supports_system_messages = excluded.supports_system_messages,
                supports_tool_choice = excluded.supports_tool_choice,
                source = excluded.source, updated_at = CURRENT_TIMESTAMP"#,
        )
        .bind(&id)
        .bind(&info.model)
        .bind(&info.provider)
        .bind(&info.mode)
        .bind(info.input_price)
        .bind(info.output_price)
        .bind(info.cache_read_price)
        .bind(info.cache_creation_price)
        .bind(info.max_input_tokens)
        .bind(info.max_output_tokens)
        .bind(info.supports_function_calling)
        .bind(info.supports_reasoning)
        .bind(info.supports_vision)
        .bind(info.supports_pdf_input)
        .bind(info.supports_prompt_caching)
        .bind(info.supports_system_messages)
        .bind(info.supports_tool_choice)
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
