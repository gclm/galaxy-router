//! 前置门禁：header 校验 + 预算检查 + 模型访问权限。

use sqlx::SqlitePool;

use crate::error::proxy::ProxyError;
use crate::repository::api_key_repository::{ApiKeyRepository, SqliteApiKeyRepository};
use crate::repository::budget_repository::{BudgetRepository, SqliteBudgetRepository};
use crate::repository::route_repository::{RouteRepository, SqliteRouteRepository};
use crate::repository::usage_repository::{SqliteUsageRepository, UsageRepository};

/// 验证字符串可作为 HTTP header value
/// 用于在保存上游 API Key 时一次性拦截含 CRLF / 控制字符的输入，
/// 避免转发时 `HeaderValue::from_str(...).unwrap()` panic。
pub(crate) fn validate_header_value(s: &str) -> Result<(), String> {
    reqwest::header::HeaderValue::from_str(s)
        .map(|_| ())
        .map_err(|e| format!("含非法 header 字符 ({e})"))
}

/// 检查 API Key 的预算额度（月/日）
pub(super) async fn check_budget(pool: &SqlitePool, key_id: &str) -> Result<(), String> {
    let limit = SqliteBudgetRepository::new(pool.clone(), 0)
        .get_limits(key_id)
        .await
        .map_err(|e| format!("查询预算失败: {}", e))?;

    let Some((monthly_limit, daily_limit)) = limit else {
        return Ok(()); // 无预算限制
    };

    // 累计消费（月/日，UTC）— tz 不影响（用 created_at + now）
    let (monthly_cost, daily_cost) = SqliteUsageRepository::new(pool.clone(), 0)
        .aggregate_cost(key_id)
        .await
        .map_err(|e| format!("查询消费失败: {}", e))?;

    if daily_limit > 0.0 && daily_cost >= daily_limit {
        return Err(format!(
            "日预算已耗尽: ${:.2}/${:.2}",
            daily_cost, daily_limit
        ));
    }
    if monthly_limit > 0.0 && monthly_cost >= monthly_limit {
        return Err(format!(
            "月预算已耗尽: ${:.2}/${:.2}",
            monthly_cost, monthly_limit
        ));
    }

    Ok(())
}

/// 验证 API Key 是否有权访问目标模型（三段式：403→404→503）
pub(super) async fn validate_model_access(
    pool: &SqlitePool,
    key_id: &str,
    model: &str,
    allowed_routes: &str,
) -> Result<(), ProxyError> {
    // === 第一关：检查 allowed_routes / supported_models ===
    // allowed_routes 非空时，检查请求的 model 是否在允许的分组列表中
    if !allowed_routes.is_empty() {
        let allowed: Vec<&str> = allowed_routes
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !allowed.is_empty() && !allowed.contains(&model) {
            return Err(ProxyError::ModelNotSupported(format!(
                "API Key 无权访问模型: {}",
                model
            )));
        }
    } else {
        // allowed_routes 为空时回退到 supported_models（兼容旧逻辑）
        // key 已鉴权（enabled 由 middleware 保证），tz 不影响此查询
        let supported = SqliteApiKeyRepository::new(pool.clone(), 0)
            .get_supported_models(key_id)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if let Some(models_str) = supported
            && !models_str.is_empty()
        {
            let allowed =
                crate::api::handlers::admin::api_keys::parse_supported_models(&models_str);
            if !allowed.iter().any(|m| m == model) {
                return Err(ProxyError::ModelNotSupported(format!(
                    "API Key 无权访问模型: {}",
                    model
                )));
            }
        }
    }

    // === 第二关：检查分组是否存在 ===
    // 仅在 allowed_routes 非空时严格检查（Octopus 策略：明确指定了分组才校验）
    // allowed_routes 为空时跳过，由后续 Relay candidate 构建检查渠道可用性
    if !allowed_routes.is_empty() {
        let group_exists = SqliteRouteRepository::new(pool.clone(), 0)
            .find_enabled_by_name(model)
            .await
            .map(|opt| opt.is_some())
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        if !group_exists {
            return Err(ProxyError::ModelNotFound(format!("模型不存在: {}", model)));
        }
    }

    // === 第三关：分组内是否有可用渠道（延迟到 Relay candidate 构建时检查）===
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_header_value_accepts_normal_and_rejects_crlf() {
        assert!(validate_header_value("sk-abc.123_OK-9").is_ok());
        assert!(validate_header_value("sk-abc/def+ghi=").is_ok());
        assert!(validate_header_value("sk-abc").is_ok());
        assert!(validate_header_value("sk-abc\r\nfoo").is_err());
        assert!(validate_header_value("sk-abc\0").is_err());
        assert!(validate_header_value("sk-abc\x7f").is_err());
    }

    /// 回归:月预算只算当月消费,历史月份不计入(修复"一设就被拦")
    #[tokio::test]
    async fn check_budget_monthly_ignores_previous_month() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("src/db/migrations").run(&pool).await.unwrap();
        let key_id = "019ecf15-0000-7000-8000-000000000001";

        sqlx::query("INSERT INTO api_keys (id, name, api_key) VALUES (?, 't', 'sk-t')")
            .bind(key_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO budget_limits (id, api_key_id, monthly_limit_usd) VALUES ('b1', ?, 10.0)",
        )
        .bind(key_id)
        .execute(&pool)
        .await
        .unwrap();
        // 上月消费 100(全部历史超 10,但不在当月)
        sqlx::query(
            "INSERT INTO usage_logs (id, api_key_id, requested_model, cost, created_at) VALUES ('u1', ?, 'm', 100.0, datetime('now','-1 month'))",
        )
        .bind(key_id)
        .execute(&pool)
        .await
        .unwrap();
        // 当月消费 1(当月未超 10)
        sqlx::query(
            "INSERT INTO usage_logs (id, api_key_id, requested_model, cost) VALUES ('u2', ?, 'm', 1.0)",
        )
        .bind(key_id)
        .execute(&pool)
        .await
        .unwrap();
        // 修复前 monthly_cost=101>=10 误拦;修复后只算当月 1<10 → Ok
        assert!(
            check_budget(&pool, key_id).await.is_ok(),
            "月预算应只算当月消费(1),不应把上月历史(100)计入"
        );
    }

    /// 当月消费超额仍应拦截(确认修复没破坏正常拦截)
    #[tokio::test]
    async fn check_budget_blocks_when_current_month_exceeds() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("src/db/migrations").run(&pool).await.unwrap();
        let key_id = "019ecf15-0000-7000-8000-000000000002";

        sqlx::query("INSERT INTO api_keys (id, name, api_key) VALUES (?, 't', 'sk-t2')")
            .bind(key_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO budget_limits (id, api_key_id, monthly_limit_usd) VALUES ('b2', ?, 10.0)",
        )
        .bind(key_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO usage_logs (id, api_key_id, requested_model, cost) VALUES ('u3', ?, 'm', 15.0)",
        )
        .bind(key_id)
        .execute(&pool)
        .await
        .unwrap();
        // 当月 15 >= 10 → 拦截
        assert!(check_budget(&pool, key_id).await.is_err());
    }

    /// 回归:无消费记录时 SUM 返回 INTEGER 0,须能正确 decode 为 f64
    /// (曾因漏 CAST AS REAL 报 "Rust f64 not compatible with SQL INTEGER")
    #[tokio::test]
    async fn check_budget_no_usage_decodes_cleanly() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("src/db/migrations").run(&pool).await.unwrap();
        let key_id = "019ecf15-0000-7000-8000-000000000003";

        sqlx::query("INSERT INTO api_keys (id, name, api_key) VALUES (?, 't', 'sk-t3')")
            .bind(key_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO budget_limits (id, api_key_id, monthly_limit_usd) VALUES ('b3', ?, 10.0)",
        )
        .bind(key_id)
        .execute(&pool)
        .await
        .unwrap();
        // 无 usage_logs → SUM 为 INTEGER 0,修复前 sqlx 按 f64 解码失败
        assert!(check_budget(&pool, key_id).await.is_ok());
    }
}
