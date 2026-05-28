//! Web 搜索客户端抽象。
//!
//! 与 [`crate::llm`] 同构:核心库只暴露 trait + Noop 实现;具体 HTTP 客户端
//! (Brave/Tavily/etc) 放在桥接层。LLM 元数据建议的 Pass B 流程会从此 trait
//! 拿搜索结果作为 LLM 的额外上下文。
//!
//! 任何功能都不得以「必须有搜索」为前提:`NoopWebSearch` 永远可用,返回
//! `NotConfigured`,调用方静默退化即可。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("HTTP 请求失败: {0}")]
    Http(String),
    #[error("响应解析失败: {0}")]
    Parse(String),
    #[error("搜索后端未配置(API key 或 provider 为空)")]
    NotConfigured,
}

/// 单条搜索结果。字段命名遵循通用搜索引擎语义,与具体 provider 无关。
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    /// 摘要片段(provider 通常已截短,无需自己再截)。
    pub snippet: String,
}

/// Web 搜索接口。同 [`crate::llm::LlmClient`] 一样要求 Send + Sync,
/// 桥接层用 `Arc<dyn WebSearch>` 持有,LLM 客户端可注入引用做两段式编排。
pub trait WebSearch: Send + Sync {
    /// 执行一次搜索,返回前若干条结果(provider 内部决定数量,通常 5-10)。
    ///
    /// 未配置时返回 `Err(SearchError::NotConfigured)`,调用方应当静默跳过。
    fn search(&self, query: &str) -> Result<Vec<SearchResult>, SearchError>;
}

/// 空实现:总是返回 `NotConfigured`。
pub struct NoopWebSearch;

impl WebSearch for NoopWebSearch {
    fn search(&self, _query: &str) -> Result<Vec<SearchResult>, SearchError> {
        Err(SearchError::NotConfigured)
    }
}
