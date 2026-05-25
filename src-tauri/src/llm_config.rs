//! LLM 配置:读写 `%APPDATA%\Endpoint\config.toml`。
//!
//! 配置文件格式:
//! ```toml
//! [llm]
//! base_url = "https://api.deepseek.com"
//! model = "deepseek-chat"
//! api_key = "sk-..."
//! ```
//!
//! API key 存放在用户的 AppData,仅本机可读。v1 明文存储 + 警告;后期可改用 OS keychain。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    llm: LlmConfig,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Endpoint").join("config.toml"))
}

/// `%APPDATA%\Endpoint\rules.json` — LLM 归纳规则的持久化路径。
pub fn user_rules_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Endpoint").join("rules.json"))
}

pub fn load() -> LlmConfig {
    let Some(path) = config_path() else {
        return LlmConfig::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return LlmConfig::default();
    };
    toml::from_str::<ConfigFile>(&text)
        .unwrap_or_default()
        .llm
}

pub fn save(cfg: &LlmConfig) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("无法获取配置目录".into());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let file = ConfigFile { llm: cfg.clone() };
    let text = toml::to_string_pretty(&file).map_err(|e| format!("配置序列化失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入配置失败: {e}"))
}

/// 根据配置构造 LLM 客户端。api_key 或 base_url 为空时返回 `NoopLlmClient`。
pub fn create_client(cfg: &LlmConfig) -> Box<dyn endpoint_core::llm::LlmClient> {
    if cfg.api_key.is_empty() || cfg.base_url.is_empty() {
        Box::new(endpoint_core::llm::NoopLlmClient)
    } else {
        Box::new(crate::openai_client::OpenAiCompatibleClient::new(
            cfg.base_url.clone(),
            cfg.model.clone(),
            cfg.api_key.clone(),
        ))
    }
}
