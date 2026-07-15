//! 路由（分组）service：CRUD + 分组项编排（校验 + 约束分类 + 缓存失效）（D3 归位）。
//!
//! 复用 RouteRepository + ProxyCache。handler 只做 HTTP 适配（map RouteError → ApiError）。

use std::sync::Arc;

use crate::domain::route::{
    AddRouteItemRequest, CreateRouteRequest, ListRoutesQuery, Route, RouteItem, UpdateRouteRequest,
};
use crate::infra::cache::ProxyCache;
use crate::repository::route_repository::RouteRepository;
use crate::repository::{ConstraintKind, classify_constraint};

#[derive(Clone)]
pub struct RouteService {
    repo: Arc<dyn RouteRepository>,
    cache: ProxyCache,
}

impl RouteService {
    pub fn new(repo: Arc<dyn RouteRepository>, cache: ProxyCache) -> Self {
        Self { repo, cache }
    }

    pub async fn list(
        &self,
        query: ListRoutesQuery,
    ) -> Result<(Vec<Route>, i64), RouteError> {
        self.repo.list(query).await.map_err(internal)
    }

    pub async fn create(&self, req: CreateRouteRequest) -> Result<Route, RouteError> {
        if req.name.is_empty() {
            return Err(RouteError::BadRequest("分组名称不能为空".into()));
        }
        if req.items.is_empty() {
            return Err(RouteError::BadRequest("至少需要一个分组项".into()));
        }
        let route = self
            .repo
            .create(req)
            .await
            .map_err(|e| map_repo_err(e, "分组名称已存在"))?;
        self.cache.invalidate_all_routes().await;
        Ok(route)
    }

    pub async fn get(&self, id: &str) -> Result<Route, RouteError> {
        self.repo
            .get(id)
            .await
            .map_err(internal)?
            .ok_or_else(|| RouteError::NotFound("分组不存在".into()))
    }

    pub async fn update(
        &self,
        id: &str,
        req: UpdateRouteRequest,
    ) -> Result<Route, RouteError> {
        if let Some(items) = &req.items
            && items.is_empty()
        {
            return Err(RouteError::BadRequest("至少需要一个分组项".into()));
        }
        let route = self
            .repo
            .update(id, req)
            .await
            .map_err(|e| map_repo_err(e, "分组项重复"))?
            .ok_or_else(|| RouteError::NotFound("分组不存在".into()))?;
        self.cache.invalidate_all_routes().await;
        Ok(route)
    }

    pub async fn delete(&self, id: &str) -> Result<(), RouteError> {
        let deleted = self.repo.delete(id).await.map_err(internal)?;
        if !deleted {
            return Err(RouteError::NotFound("分组不存在".into()));
        }
        self.cache.invalidate_all_routes().await;
        Ok(())
    }

    pub async fn add_item(
        &self,
        route_id: &str,
        req: AddRouteItemRequest,
    ) -> Result<RouteItem, RouteError> {
        let item = self
            .repo
            .add_item(route_id, req)
            .await
            .map_err(map_fk_err)?
            .ok_or_else(|| RouteError::NotFound("分组不存在".into()))?;
        // 修 bug（原 routes.rs add_item 无缓存失效）
        self.cache.invalidate_all_routes().await;
        Ok(item)
    }

    pub async fn delete_item(&self, route_id: &str, item_id: &str) -> Result<(), RouteError> {
        let deleted = self
            .repo
            .delete_item(route_id, item_id)
            .await
            .map_err(internal)?;
        if !deleted {
            return Err(RouteError::NotFound("分组项不存在".into()));
        }
        self.cache.invalidate_all_routes().await;
        Ok(())
    }
}

pub enum RouteError {
    BadRequest(String),
    Conflict(String),
    NotFound(String),
    Internal(String),
}

fn internal<E: ToString>(e: E) -> RouteError {
    RouteError::Internal(e.to_string())
}

/// 写操作约束分类。`unique_msg` 区分 create（名称已存在）/ update（分组项重复）。
fn map_repo_err(e: sqlx::Error, unique_msg: &str) -> RouteError {
    match classify_constraint(&e) {
        Some(ConstraintKind::UniqueViolation) => RouteError::Conflict(unique_msg.into()),
        Some(ConstraintKind::ForeignKeyViolation) => RouteError::BadRequest("渠道不存在".into()),
        None => internal(e),
    }
}

/// add_item 仅 FOREIGN KEY（渠道不存在）；原 handler 未嗅探 UNIQUE，保持 internal。
fn map_fk_err(e: sqlx::Error) -> RouteError {
    match classify_constraint(&e) {
        Some(ConstraintKind::ForeignKeyViolation) => RouteError::BadRequest("渠道不存在".into()),
        _ => internal(e),
    }
}
