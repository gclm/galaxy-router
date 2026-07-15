//! 基础设施层（连接池、配置、缓存）。
//! db 归入本层：连接池 + 迁移执行器 + 迁移 SQL（db/migrations/，约束 #5，sqlx::migrate! 编译期常量）。

pub mod db;
pub mod config;
pub mod cache;
