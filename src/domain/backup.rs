//! 备份/恢复数据契约（纯 serde 结构，跨 repository/service/handler）。
//!
//! BackupFile 为导出/导入的 JSON 顶层格式；BackupData 为四类配置数据的载荷。

use serde::{Deserialize, Serialize};

use crate::domain::channel::Channel;

pub const BACKUP_FORMAT: &str = "galaxy-router-backup";
pub const BACKUP_VERSION: i32 = 1;

/// 导出文件格式
#[derive(Debug, Serialize, Deserialize)]
pub struct BackupFile {
    pub format: String,
    pub version: i32,
    pub exported_at: String,
    pub app_version: String,
    pub data: BackupData,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupData {
    pub channels: Vec<Channel>,
    #[serde(alias = "groups")]
    pub routes: Vec<RouteExport>,
    pub api_keys: Vec<ApiKeyExport>,
    pub settings: Vec<SettingExport>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteExport {
    pub name: String,
    pub match_regex: Option<String>,
    pub retry_enabled: bool,
    pub first_token_timeout_secs: i32,
    pub enabled: bool,
    pub items: Vec<RouteItemExport>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteItemExport {
    pub channel_name: String,
    pub model_name: String,
    pub priority: i32,
    pub weight: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyExport {
    pub name: String,
    pub api_key: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingExport {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Default)]
pub struct ImportResult {
    pub channels_imported: u32,
    pub routes_imported: u32,
    pub api_keys_imported: u32,
    pub settings_imported: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ResetResult {
    pub channels_deleted: u64,
    pub routes_deleted: u64,
    pub api_keys_deleted: u64,
    pub settings_reset: u64,
}
