//! Database 已迁 infra/db（B3-C1），此处 re-export 保 crate::db::Database 兼容。
//! SQL 迁移文件留 src/db/migrations（约束 #5，sqlx::migrate! 路径常量，编译期解析）。
pub use crate::infra::db::Database;
