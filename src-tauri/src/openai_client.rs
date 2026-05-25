//! OpenAI 兼容 HTTP 客户端(桥接层实现)。
//!
//! 兼容 `/v1/chat/completions` 接口的任何服务都可接入:DeepSeek、OpenAI、本地 Ollama 等。
//! reqwest blocking 在 `spawn_blocking` 内调用,不阻塞 Tauri 异步运行时。

use endpoint_core::llm::{
    AdjudicationVerdict, LlmClient, LlmError, MetadataSuggestion, WatermarkCandidate,
};
use endpoint_core::rules::{Rule, RuleKind, RuleSource};
use reqwest::blocking::Client;
use serde_json::{json, Value};

pub struct OpenAiCompatibleClient {
    base_url: String,
    model: String,
    api_key: String,
    client: Client,
}

impl OpenAiCompatibleClient {
    pub fn new(base_url: String, model: String, api_key: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    fn chat(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user",   "content": user   }
            ],
            "temperature": 0.1,
            "max_tokens": 512,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| LlmError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(LlmError::Http(format!("HTTP {status}: {text}")));
        }

        let json: Value = resp.json().map_err(|e| LlmError::Parse(e.to_string()))?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| LlmError::Parse("响应中未找到 content 字段".into()))?
            .to_string();
        Ok(content)
    }
}

impl LlmClient for OpenAiCompatibleClient {
    fn arbitrate_watermark(
        &self,
        candidates: &[WatermarkCandidate],
    ) -> Result<Vec<AdjudicationVerdict>, LlmError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let numbered: String = candidates
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}. {}", i + 1, c.text))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "你是中文网络小说电子书制作助手。判断以下各行是否为水印/推广内容。\
            对每一行输出「水印」或「正文」或「不确定」,格式严格如下(每行一个数字加判断):\n\
            1. 水印\n2. 正文\n以此类推。";
        let user = format!("请判断以下各行:\n{numbered}");

        let raw = self.chat(system, &user)?;

        let mut verdicts: Vec<AdjudicationVerdict> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(candidates.len())
            .map(|line| {
                if line.contains("水印") {
                    AdjudicationVerdict::IsWatermark
                } else if line.contains("正文") {
                    AdjudicationVerdict::IsContent
                } else {
                    AdjudicationVerdict::Uncertain
                }
            })
            .collect();

        while verdicts.len() < candidates.len() {
            verdicts.push(AdjudicationVerdict::Uncertain);
        }
        Ok(verdicts)
    }

    fn induce_rule(&self, rejected_lines: &[&str]) -> Result<Option<Rule>, LlmError> {
        if rejected_lines.is_empty() {
            return Ok(None);
        }

        let samples = rejected_lines
            .iter()
            .take(10)
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "你是中文网络小说电子书制作助手。根据给定的水印行样本,\
            归纳一条能匹配这类水印的 Rust 正则表达式(re2 语法)。\
            只输出正则表达式本身,不加任何解释或引号。";
        let user = format!("水印行样本:\n{samples}");

        let raw = self.chat(system, &user)?.trim().to_string();
        if raw.is_empty() {
            return Ok(None);
        }

        // 验证正则能编译,否则丢弃
        if regex::Regex::new(&raw).is_err() {
            return Ok(None);
        }

        let hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            raw.hash(&mut h);
            h.finish()
        };
        let id = format!("llm-watermark-{hash:x}");

        Ok(Some(Rule {
            id,
            pattern: raw,
            kind: RuleKind::Watermark,
            enabled: true,
            priority: 50,
            source: RuleSource::LlmGenerated,
            description: format!("LLM 归纳:从 {} 条样本生成", rejected_lines.len()),
        }))
    }

    fn suggest_metadata(&self, sample_text: &str) -> Result<Option<MetadataSuggestion>, LlmError> {
        let sample = &sample_text[..sample_text.len().min(1200)];
        let system = "你是中文网络小说电子书制作助手。从给定章节开头文本推断书名和作者。\
            输出格式(每行一项):\n书名: XXX\n作者: XXX\n如果无法推断请输出「未知」。";
        let user = format!("章节开头文本:\n{sample}");

        let raw = self.chat(system, &user)?;
        let mut title = None;
        let mut author = None;

        for line in raw.lines() {
            if let Some(v) = line
                .strip_prefix("书名:")
                .or_else(|| line.strip_prefix("书名："))
            {
                let v = v.trim();
                if v != "未知" {
                    title = Some(v.to_string());
                }
            }
            if let Some(v) = line
                .strip_prefix("作者:")
                .or_else(|| line.strip_prefix("作者："))
            {
                let v = v.trim();
                if v != "未知" {
                    author = Some(v.to_string());
                }
            }
        }

        if title.is_none() && author.is_none() {
            return Ok(None);
        }
        Ok(Some(MetadataSuggestion { title, author }))
    }
}
