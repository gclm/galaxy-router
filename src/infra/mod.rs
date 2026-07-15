//! 基础设施层（连接池、配置、缓存）。
//! B3-C1 起 db 归入；SQL 迁移文件仍留 src/db/migrations（约束 #5，sqlx::migrate! 编译期常量）。

pub mod db;
pub mod config;
