//! 渠道 service：CRUD 编排（校验 + 约束分类 + 缓存失效）（D2 归位）。
//!
//! 复用 ChannelRepository + row_to_channel + validate_header_value + ProxyCache。
//! handler 只做 HTTP 适配（map ChannelError → ApiError）；端点探测试探见子模块 `probe`。

use std::sync::Arc;

use crate::domain::channel::{
    Channel, CreateChannelRequest, ListChannelsQuery, UpdateChannelRequest,
};
use crate::infra::cache::ProxyCache;
use crate::llm::relay::pipeline::validate_header_value;
use crate::repository::channel_repository::{ChannelRepository, row_to_channel};
use crate::repository::{ConstraintKind, classify_constraint};

pub mod probe;

#[derive(Clone)]
pub struct ChannelService {
    repo: Arc<dyn ChannelRepository>,
    cache: ProxyCache,
}

impl ChannelService {
    pub fn new(repo: Arc<dyn ChannelRepository>, cache: ProxyCache) -> Self {
        Self { repo, cache }
    }

    pub async fn list(
        &self,
        query: ListChannelsQuery,
    ) -> Result<(Vec<Channel>, i64), ChannelError> {
        let (rows, total) = self.repo.list(query).await.map_err(internal)?;
        let items = rows
            .into_iter()
            .map(row_to_channel)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        Ok((items, total))
    }

    pub async fn create(&self, req: CreateChannelRequest) -> Result<Channel, ChannelError> {
        validate_create(&req)?;
        let row = self.repo.create(req).await.map_err(map_repo_err)?;
        self.cache.invalidate_all_channels().await;
        row_to_channel(row).map_err(internal)
    }

    pub async fn get(&self, id: &str) -> Result<Channel, ChannelError> {
        let row = self
            .repo
            .get(id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ChannelError::NotFound("渠道不存在".into()))?;
        row_to_channel(row).map_err(internal)
    }

    pub async fn update(
        &self,
        id: &str,
        req: UpdateChannelRequest,
    ) -> Result<Channel, ChannelError> {
        validate_update(&req)?;
        let row = self
            .repo
            .update(id, req)
            .await
            .map_err(map_repo_err)?
            .ok_or_else(|| ChannelError::NotFound("渠道不存在".into()))?;
        // 修 bug（原 crud.rs update 无缓存失效）：渠道变更影响渠道缓存 + 引用它的路由缓存
        self.cache.invalidate_channel(id).await;
        self.cache.invalidate_all_routes().await;
        row_to_channel(row).map_err(internal)
    }

    pub async fn delete(&self, id: &str) -> Result<(), ChannelError> {
        let route_ids = self
            .repo
            .delete(id)
            .await
            .map_err(internal)?
            .ok_or_else(|| ChannelError::NotFound("渠道不存在".into()))?;
        self.cache.invalidate_channel(id).await;
        if !route_ids.is_empty() {
            self.cache.invalidate_all_routes().await;
        }
        Ok(())
    }
}

pub enum ChannelError {
    BadRequest(String),
    Conflict(String),
    NotFound(String),
    Internal(String),
}

fn internal<E: ToString>(e: E) -> ChannelError {
    ChannelError::Internal(e.to_string())
}

/// repository 错误 → ChannelError（channels 表无外键，仅 UNIQUE name → Conflict）
fn map_repo_err(e: sqlx::Error) -> ChannelError {
    match classify_constraint(&e) {
        Some(ConstraintKind::UniqueViolation) => ChannelError::Conflict("渠道名称已存在".into()),
        _ => internal(e),
    }
}

fn validate_create(req: &CreateChannelRequest) -> Result<(), ChannelError> {
    if req.name.is_empty() {
        return Err(ChannelError::BadRequest("渠道名称不能为空".into()));
    }
    if req.api_keys.is_empty() {
        return Err(ChannelError::BadRequest("至少需要一个 API Key".into()));
    }
    if req.endpoints.is_empty() {
        return Err(ChannelError::BadRequest("至少需要一个端点".into()));
    }
    for k in &req.api_keys {
        validate_header_value(&k.key)
            .map_err(|e| ChannelError::BadRequest(format!("API Key 非法: {e}")))?;
    }
    Ok(())
}

fn validate_update(req: &UpdateChannelRequest) -> Result<(), ChannelError> {
    if let Some(api_keys) = &req.api_keys {
        for k in api_keys {
            validate_header_value(&k.key)
                .map_err(|e| ChannelError::BadRequest(format!("API Key 非法: {e}")))?;
        }
    }
    let has_update = req.name.is_some()
        || req.api_keys.is_some()
        || req.endpoints.is_some()
        || req.models.is_some()
        || req.enabled.is_some()
        || req.rate_limit_rpm.is_some()
        || req.rate_limit_tpm.is_some()
        || req.failure_threshold.is_some()
        || req.blacklist_minutes.is_some()
        || req.concurrency.is_some()
        || req.timeout_secs.is_some()
        || req.max_concurrency.is_some();
    if !has_update {
        return Err(ChannelError::BadRequest("没有需要更新的字段".into()));
    }
    Ok(())
}
