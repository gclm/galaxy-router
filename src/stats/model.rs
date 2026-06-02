use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

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

/// models.dev API 响应
#[derive(Debug, Serialize, Deserialize)]
struct ModelsDevResponse(HashMap<String, ModelsDevProvider>);

#[derive(Debug, Serialize, Deserialize)]
struct ModelsDevProvider {
    #[serde(default)]
    models: HashMap<String, ModelsDevModel>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ModelsDevModel {
    id: String,
    #[serde(default)]
    cost: Option<ModelsDevCost>,
    #[serde(default)]
    limit: Option<ModelsDevLimit>,
    #[serde(default)]
    modalities: Option<ModelsDevModalities>,
    #[serde(default)]
    tool_call: Option<bool>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    temperature: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ModelsDevCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ModelsDevLimit {
    context: Option<i64>,
    output: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ModelsDevModalities {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    output: Vec<String>,
}

/// 成本计算器（DB 为 source of truth，内存缓存加速读取）
#[derive(Clone)]
pub struct ModelRegistry {
    pool: sqlx::SqlitePool,
    pricing: Arc<RwLock<HashMap<String, ModelInfo>>>,
    http_client: reqwest::Client,
}

impl ModelRegistry {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self {
            pool,
            pricing: Arc::new(RwLock::new(HashMap::new())),
            http_client: reqwest::Client::new(),
        }
    }

    /// 从 DB 加载到内存
    pub async fn load_from_db(&self) -> Result<(), sqlx::Error> {
        let rows = sqlx::query_as::<
            _,
            (
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
            ),
        >(
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

        let mut map = self.pricing.write().await;
        map.clear();
        for row in rows {
            let info = row_to_info(row);
            map.insert(info.model.clone(), info);
        }

        tracing::info!("从数据库加载了 {} 条模型信息", map.len());
        Ok(())
    }

    /// 从缓存文件加载
    pub async fn load_from_cache(&self, cache_path: &Path) -> Result<(), String> {
        if !cache_path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(cache_path)
            .await
            .map_err(|e| format!("读取缓存文件失败: {}", e))?;

        let models: HashMap<String, ModelInfo> =
            serde_json::from_str(&content).map_err(|e| format!("解析缓存文件失败: {}", e))?;

        self.upsert_models_to_db(models.values()).await?;
        self.load_from_db().await.map_err(|e| e.to_string())?;

        tracing::info!("从缓存文件加载模型信息: {}", cache_path.display());
        Ok(())
    }

    /// 从 models.dev API 拉取 → 写缓存 → upsert DB → 刷新内存
    pub async fn fetch_remote_pricing(
        &self,
        cache_path: &Path,
        providers: &[String],
    ) -> Result<(), String> {
        let response = self
            .http_client
            .get("https://models.dev/api.json")
            .send()
            .await
            .map_err(|e| format!("请求失败: {}", e))?
            .json::<ModelsDevResponse>()
            .await
            .map_err(|e| format!("解析失败: {}", e))?;

        // 过滤 + 转换
        let models = flatten_and_filter(response, providers);

        // 写缓存文件（扁平格式）
        if let Some(parent) = cache_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let json =
            serde_json::to_string_pretty(&models).map_err(|e| format!("序列化失败: {}", e))?;
        tokio::fs::write(cache_path, &json)
            .await
            .map_err(|e| format!("写入缓存失败: {}", e))?;

        tracing::info!(
            "模型信息缓存已更新: {} ({} 条)",
            cache_path.display(),
            models.len()
        );

        // upsert DB
        self.upsert_models_to_db(models.values()).await?;
        self.load_from_db().await.map_err(|e| e.to_string())?;

        Ok(())
    }

    /// upsert 模型信息到 DB
    async fn upsert_models_to_db(
        &self,
        models: impl Iterator<Item = &ModelInfo>,
    ) -> Result<(), String> {
        let mut count = 0u32;
        for info in models {
            let id = crate::api::response::generate_id();
            let result = sqlx::query(
                r#"INSERT INTO model_info (
                    id, model, provider, mode,
                    input_price, output_price, cache_read_price, cache_creation_price,
                    max_input_tokens, max_output_tokens,
                    supports_function_calling, supports_reasoning, supports_vision,
                    supports_pdf_input, supports_prompt_caching,
                    supports_system_messages, supports_tool_choice,
                    source
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'remote')
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
                    source = 'remote', updated_at = CURRENT_TIMESTAMP"#,
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
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;

            if result.rows_affected() > 0 {
                count += 1;
            }
        }

        tracing::info!("upsert {} 条模型信息到数据库", count);
        Ok(())
    }

    /// 计算成本（读内存，快路径）
    pub async fn calculate_cost(
        &self,
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        cache_creation_tokens: i32,
    ) -> f64 {
        let pricing = self.pricing.read().await;

        if let Some(info) = pricing.get(model) {
            return compute_cost(
                info,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            );
        }

        // 模糊匹配
        for (key, info) in pricing.iter() {
            if key == model || key.ends_with(&format!("/{}", model)) {
                return compute_cost(
                    info,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                );
            }
        }

        0.0
    }

    /// 获取模型信息
    pub async fn get_model_info(&self, model: &str) -> Option<ModelInfo> {
        let pricing = self.pricing.read().await;
        pricing.get(model).cloned()
    }

    /// 获取所有模型信息
    pub async fn get_all_models(&self) -> Vec<ModelInfo> {
        let pricing = self.pricing.read().await;
        let mut result: Vec<ModelInfo> = pricing.values().cloned().collect();
        result.sort_by(|a, b| a.model.cmp(&b.model));
        result
    }

    /// 手动设置模型信息
    pub async fn set_model_info(&self, info: ModelInfo) -> Result<(), String> {
        let id = crate::api::response::generate_id();
        sqlx::query(
            r#"INSERT INTO model_info (
                id, model, provider, mode,
                input_price, output_price, cache_read_price, cache_creation_price,
                max_input_tokens, max_output_tokens,
                supports_function_calling, supports_reasoning, supports_vision,
                supports_pdf_input, supports_prompt_caching,
                supports_system_messages, supports_tool_choice,
                source
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'manual')
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
                source = 'manual', updated_at = CURRENT_TIMESTAMP"#,
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
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let mut pricing = self.pricing.write().await;
        pricing.insert(info.model.clone(), info);

        Ok(())
    }
}

/// 将 models.dev 嵌套结构扁平化 + 按 providers 过滤
fn flatten_and_filter(
    response: ModelsDevResponse,
    providers: &[String],
) -> HashMap<String, ModelInfo> {
    let mut result = HashMap::new();

    for (provider_id, provider) in response.0 {
        // providers 白名单过滤
        if !providers.is_empty() && !providers.contains(&provider_id) {
            continue;
        }

        for (_, model) in provider.models {
            let cost = model.cost.as_ref();
            let limit = model.limit.as_ref();
            let modalities = model.modalities.as_ref();

            // 跳过没有定价的模型
            let Some(cost) = cost else { continue };

            let info = ModelInfo {
                model: model.id.clone(),
                provider: provider_id.clone(),
                mode: "chat".to_string(),
                input_price: cost.input,
                output_price: cost.output,
                cache_read_price: cost.cache_read,
                cache_creation_price: cost.cache_write,
                max_input_tokens: limit.and_then(|l| l.context),
                max_output_tokens: limit.and_then(|l| l.output),
                supports_function_calling: model.tool_call,
                supports_reasoning: model.reasoning,
                supports_vision: modalities.map(|m| m.input.contains(&"image".to_string())),
                supports_pdf_input: modalities.map(|m| m.input.contains(&"pdf".to_string())),
                supports_prompt_caching: cost.cache_read.map(|_| true),
                supports_system_messages: None,
                supports_tool_choice: model.tool_call,
            };

            result.insert(model.id, info);
        }
    }

    tracing::info!(
        "过滤后 {} 条模型 (providers: {:?})",
        result.len(),
        providers
    );
    result
}

fn row_to_info(
    (
        model,
        provider,
        mode,
        input_price,
        output_price,
        cache_read_price,
        cache_creation_price,
        max_input_tokens,
        max_output_tokens,
        supports_function_calling,
        supports_reasoning,
        supports_vision,
        supports_pdf_input,
        supports_prompt_caching,
        supports_system_messages,
        supports_tool_choice,
    ): ModelInfoRow,
) -> ModelInfo {
    ModelInfo {
        model,
        provider,
        mode,
        input_price,
        output_price,
        cache_read_price,
        cache_creation_price,
        max_input_tokens,
        max_output_tokens,
        supports_function_calling,
        supports_reasoning,
        supports_vision,
        supports_pdf_input,
        supports_prompt_caching,
        supports_system_messages,
        supports_tool_choice,
    }
}

fn compute_cost(
    info: &ModelInfo,
    input_tokens: i32,
    output_tokens: i32,
    cache_read_tokens: i32,
    cache_creation_tokens: i32,
) -> f64 {
    let input_cost = info
        .input_price
        .map(|p| (input_tokens as f64 / 1_000_000.0) * p)
        .unwrap_or(0.0);
    let output_cost = info
        .output_price
        .map(|p| (output_tokens as f64 / 1_000_000.0) * p)
        .unwrap_or(0.0);
    let cache_read_cost = info
        .cache_read_price
        .map(|p| (cache_read_tokens as f64 / 1_000_000.0) * p)
        .unwrap_or(0.0);
    let cache_creation_cost = info
        .cache_creation_price
        .map(|p| (cache_creation_tokens as f64 / 1_000_000.0) * p)
        .unwrap_or(0.0);

    input_cost + output_cost + cache_read_cost + cache_creation_cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn make_pool() -> sqlx::SqlitePool {
        let db_path = format!("/tmp/galaxy_stats_model_{}.db", uuid::Uuid::now_v7());
        let _ = std::fs::remove_file(&db_path);
        let db_url = format!("sqlite:{}?mode=rwc", db_path);
        Database::new(&db_url).await.unwrap().pool().clone()
    }

    fn info(model: &str, input: f64, output: f64) -> ModelInfo {
        ModelInfo {
            model: model.into(),
            provider: "test".into(),
            mode: "chat".into(),
            input_price: Some(input),
            output_price: Some(output),
            cache_read_price: None,
            cache_creation_price: None,
            max_input_tokens: None,
            max_output_tokens: None,
            supports_function_calling: None,
            supports_reasoning: None,
            supports_vision: None,
            supports_pdf_input: None,
            supports_prompt_caching: None,
            supports_system_messages: None,
            supports_tool_choice: None,
        }
    }

    #[tokio::test]
    async fn calculate_cost_basic_input_output() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        reg.set_model_info(info("gpt-4o", 2.5, 10.0)).await.unwrap();

        // 1M input + 0.5M output = 2.5 + 5.0 = 7.5
        let cost = reg.calculate_cost("gpt-4o", 1_000_000, 500_000, 0, 0).await;
        assert!((cost - 7.5).abs() < 1e-9);

        // 0 tokens -> 0
        let cost = reg.calculate_cost("gpt-4o", 0, 0, 0, 0).await;
        assert_eq!(cost, 0.0);
    }

    #[tokio::test]
    async fn calculate_cost_includes_cache_prices() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        let mut m = info("claude", 3.0, 15.0);
        m.cache_read_price = Some(0.3);
        m.cache_creation_price = Some(3.75);
        reg.set_model_info(m).await.unwrap();

        // input 1M (3.0) + output 1M (15.0) + cache_read 1M (0.3) + cache_creation 1M (3.75) = 22.05
        let cost = reg
            .calculate_cost("claude", 1_000_000, 1_000_000, 1_000_000, 1_000_000)
            .await;
        assert!((cost - 22.05).abs() < 1e-9);
    }

    #[tokio::test]
    async fn calculate_cost_missing_prices_default_zero() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        // 没有 input_price 的模型
        let mut m = info("mystery", 0.0, 0.0);
        m.input_price = None;
        reg.set_model_info(m).await.unwrap();

        let cost = reg.calculate_cost("mystery", 1_000_000, 1_000_000, 0, 0).await;
        assert_eq!(cost, 0.0);
    }

    #[tokio::test]
    async fn calculate_cost_unknown_model_returns_zero() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        let cost = reg.calculate_cost("nope", 1_000_000, 1_000_000, 0, 0).await;
        assert_eq!(cost, 0.0);
    }

    #[tokio::test]
    async fn calculate_cost_fuzzy_match_by_suffix() {
        // key 形如 "openai/gpt-4o" 时，传入 "gpt-4o" 也应命中
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        reg.set_model_info(info("openai/gpt-4o", 1.0, 2.0))
            .await
            .unwrap();

        let cost = reg.calculate_cost("gpt-4o", 1_000_000, 0, 0, 0).await;
        assert!((cost - 1.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn get_model_info_round_trip() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        assert!(reg.get_model_info("gpt-4o").await.is_none());

        reg.set_model_info(info("gpt-4o", 2.5, 10.0)).await.unwrap();
        let got = reg.get_model_info("gpt-4o").await.expect("应能取回");
        assert_eq!(got.model, "gpt-4o");
        assert_eq!(got.input_price, Some(2.5));
        assert_eq!(got.output_price, Some(10.0));
    }

    #[tokio::test]
    async fn get_all_models_returns_sorted() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        reg.set_model_info(info("z-model", 1.0, 2.0)).await.unwrap();
        reg.set_model_info(info("a-model", 3.0, 4.0)).await.unwrap();
        reg.set_model_info(info("m-model", 5.0, 6.0)).await.unwrap();

        let all = reg.get_all_models().await;
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].model, "a-model");
        assert_eq!(all[1].model, "m-model");
        assert_eq!(all[2].model, "z-model");
    }

    #[tokio::test]
    async fn load_from_db_empty_returns_no_models() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);
        reg.load_from_db().await.unwrap();
        assert!(reg.get_all_models().await.is_empty());
    }

    #[tokio::test]
    async fn load_from_db_reads_existing_rows() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        // 直接 SQL 写一行（模拟 remote source 的预置数据）
        let id = crate::api::response::generate_id();
        sqlx::query(
            r#"INSERT INTO model_info (id, model, provider, mode, input_price, output_price, source)
               VALUES (?, 'preloaded', 'openai', 'chat', 1.5, 6.0, 'remote')"#,
        )
        .bind(&id)
        .execute(&reg.pool)
        .await
        .unwrap();

        reg.load_from_db().await.unwrap();
        let got = reg.get_model_info("preloaded").await.expect("应能加载");
        assert_eq!(got.provider, "openai");
        assert_eq!(got.input_price, Some(1.5));
    }

    #[tokio::test]
    async fn set_model_info_upsert_updates_existing() {
        let pool = make_pool().await;
        let reg = ModelRegistry::new(pool);

        reg.set_model_info(info("dup", 1.0, 2.0)).await.unwrap();
        reg.set_model_info(info("dup", 5.0, 10.0)).await.unwrap();

        let got = reg.get_model_info("dup").await.unwrap();
        assert_eq!(got.input_price, Some(5.0));
        assert_eq!(got.output_price, Some(10.0));
        // 不应产生重复行
        let count: i32 =
            sqlx::query_scalar("SELECT COUNT(*) FROM model_info WHERE model = 'dup'")
                .fetch_one(&reg.pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }
}
