//! Brave Search API 客户端(桥接层实现 [`endpoint_core::search::WebSearch`])。
//!
//! 文档:https://api.search.brave.com/app/documentation/web-search/get-started
//! 免费额度:2000 次/月。请求方式:GET /res/v1/web/search,header 带
//! `X-Subscription-Token: <api_key>`,query 参数 `q` + `count` 等。
//!
//! 响应 JSON 结构(关心的部分):
//! ```json
//! { "web": { "results": [ { "title": "...", "url": "...", "description": "..." } ] } }
//! ```
//! `description` 字段即给 LLM 用的 snippet(已是纯文本,无 HTML)。

use endpoint_core::search::{SearchError, SearchResult, WebSearch};
use reqwest::blocking::Client;
use serde_json::Value;
use tracing::{info, trace, warn};

pub struct BraveSearchClient {
    api_key: String,
    client: Client,
    /// 每次请求返回的结果条数。默认 5,够 LLM 拼上下文又不超 prompt 预算。
    count: u32,
}

impl BraveSearchClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            count: 5,
        }
    }
}

impl WebSearch for BraveSearchClient {
    fn search(&self, query: &str) -> Result<Vec<SearchResult>, SearchError> {
        if self.api_key.is_empty() {
            return Err(SearchError::NotConfigured);
        }
        let url = "https://api.search.brave.com/res/v1/web/search";
        info!(
            url = %url,
            query_chars = query.chars().count(),
            count = self.count,
            "Brave 搜索请求开始"
        );
        trace!(query = %query, "Brave 搜索 query 原文");

        let started = std::time::Instant::now();
        let resp = self
            .client
            .get(url)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .query(&[("q", query), ("count", &self.count.to_string())])
            .send()
            .map_err(|e| {
                warn!(error = %e, elapsed_ms = started.elapsed().as_millis() as u64, "Brave 搜索网络层失败");
                SearchError::Http(e.to_string())
            })?;

        let status = resp.status();
        let elapsed_ms = started.elapsed().as_millis() as u64;

        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            warn!(
                status = %status,
                elapsed_ms,
                body_preview = %text.chars().take(200).collect::<String>(),
                "Brave 搜索非 2xx"
            );
            return Err(SearchError::Http(format!("HTTP {status}: {text}")));
        }

        let json: Value = resp.json().map_err(|e| {
            warn!(error = %e, elapsed_ms, "Brave 搜索响应 JSON 解析失败");
            SearchError::Parse(e.to_string())
        })?;

        let results = json["web"]["results"].as_array().cloned().unwrap_or_default();
        let parsed: Vec<SearchResult> = results
            .iter()
            .filter_map(|r| {
                Some(SearchResult {
                    title: r["title"].as_str()?.to_string(),
                    url: r["url"].as_str()?.to_string(),
                    snippet: r["description"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect();

        info!(
            status = %status,
            elapsed_ms,
            result_count = parsed.len(),
            "Brave 搜索请求完成"
        );
        Ok(parsed)
    }
}
