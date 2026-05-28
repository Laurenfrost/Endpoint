//! LLM + 搜索配置:读写 `%APPDATA%\Endpoint\config.toml`。
//!
//! 配置文件格式:
//! ```toml
//! [llm]
//! base_url = "https://api.deepseek.com"
//! model = "deepseek-chat"
//! api_key = "sk-..."
//!
//! [search]
//! provider = "brave"        # 目前仅 brave;为空 = 禁用搜索
//! api_key = "BSA..."
//! ```
//!
//! API key 存放在用户的 AppData,仅本机可读。v1 明文存储 + 警告;后期可改用 OS keychain。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchConfig {
    /// 搜索后端标识。目前支持:`"brave"`。为空 / 未识别的值 = 禁用搜索。
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
}

/// kepubify 可执行文件路径 + 是否启用。
/// `path` 为空时 `enabled` 无意义,等同于不跑 kepubify;前端 UI 会因路径缺失禁用勾选框。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KepubifyConfig {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    llm: LlmConfig,
    #[serde(default)]
    search: SearchConfig,
    #[serde(default)]
    kepubify: KepubifyConfig,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Endpoint").join("config.toml"))
}

/// `%APPDATA%\Endpoint\rules.json` — LLM 归纳规则的持久化路径。
pub fn user_rules_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Endpoint").join("rules.json"))
}

pub fn load() -> LlmConfig {
    load_all().llm
}

pub fn load_search() -> SearchConfig {
    load_all().search
}

pub fn load_kepubify() -> KepubifyConfig {
    load_all().kepubify
}

fn load_all() -> ConfigFile {
    let Some(path) = config_path() else {
        return ConfigFile::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return ConfigFile::default();
    };
    toml::from_str::<ConfigFile>(&text).unwrap_or_default()
}

pub fn save(cfg: &LlmConfig) -> Result<(), String> {
    let mut all = load_all();
    all.llm = cfg.clone();
    save_all(&all)
}

pub fn save_search(cfg: &SearchConfig) -> Result<(), String> {
    let mut all = load_all();
    all.search = cfg.clone();
    save_all(&all)
}

pub fn save_kepubify(cfg: &KepubifyConfig) -> Result<(), String> {
    let mut all = load_all();
    all.kepubify = cfg.clone();
    save_all(&all)
}

fn save_all(file: &ConfigFile) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("无法获取配置目录".into());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let text = toml::to_string_pretty(file).map_err(|e| format!("配置序列化失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入配置失败: {e}"))
}

/// 根据配置构造搜索后端。provider/api_key 任一为空 = 返回 [`NoopWebSearch`]。
pub fn create_search(cfg: &SearchConfig) -> std::sync::Arc<dyn endpoint_core::search::WebSearch> {
    if cfg.api_key.is_empty() || cfg.provider.is_empty() {
        info!(
            provider = %cfg.provider,
            key_set = !cfg.api_key.is_empty(),
            "搜索未完整配置,使用 NoopWebSearch"
        );
        return std::sync::Arc::new(endpoint_core::search::NoopWebSearch);
    }
    match cfg.provider.to_ascii_lowercase().as_str() {
        "brave" => {
            info!(provider = "brave", "构造 Brave Search 客户端");
            std::sync::Arc::new(crate::brave_client::BraveSearchClient::new(
                cfg.api_key.clone(),
            ))
        }
        other => {
            info!(provider = %other, "未知搜索 provider,使用 NoopWebSearch");
            std::sync::Arc::new(endpoint_core::search::NoopWebSearch)
        }
    }
}

/// 根据配置构造 LLM 客户端。api_key 或 base_url 为空时返回 `NoopLlmClient`。
///
/// 返回 `Arc` 而非 `Box`:供 `spawn_blocking` 调用方 clone 引用进 blocking 线程,
/// 见 [`crate::state::AppState::llm_client`] 注释。
///
/// `search_cfg` 用于把搜索后端注入 LLM 客户端,使 `suggest_metadata` 可触发 Pass B。
/// 搜索未配置时 LLM 客户端持有 `NoopWebSearch`,suggest_metadata 会自动跳过 Pass B。
pub fn create_client(
    cfg: &LlmConfig,
    search_cfg: &SearchConfig,
) -> std::sync::Arc<dyn endpoint_core::llm::LlmClient> {
    if cfg.api_key.is_empty() || cfg.base_url.is_empty() {
        info!(
            base_url_set = !cfg.base_url.is_empty(),
            key_set = !cfg.api_key.is_empty(),
            "LLM 未完整配置,使用 NoopLlmClient"
        );
        std::sync::Arc::new(endpoint_core::llm::NoopLlmClient)
    } else {
        let search = create_search(search_cfg);
        // 搜索为 Noop 时也可注入:OpenAiCompatibleClient 内部判 SearchError::NotConfigured
        // 即跳过 Pass B,行为与「不注入搜索」等价。
        info!(
            base_url = %cfg.base_url,
            model = %cfg.model,
            search_provider = %search_cfg.provider,
            "构造 OpenAI 兼容 LLM 客户端"
        );
        std::sync::Arc::new(crate::openai_client::OpenAiCompatibleClient::with_search(
            cfg.base_url.clone(),
            cfg.model.clone(),
            cfg.api_key.clone(),
            Some(search),
        ))
    }
}
