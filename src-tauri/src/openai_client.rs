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
            对每一行输出判断和一句简短理由,格式:\n\
            1. 水印: 含推广链接\n\
            2. 正文: 正常情节描写\n\
            3. 不确定: 无法判断\n以此类推。";
        let user = format!("请判断以下各行:\n{numbered}");

        let raw = self.chat(system, &user)?;

        let mut verdicts: Vec<AdjudicationVerdict> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(candidates.len())
            .map(|line| {
                // Strip leading "N. " numbering
                let content = line.splitn(2, ". ").nth(1).unwrap_or(line).trim();
                let (verdict_word, reason) = if let Some(idx) = content.find(':') {
                    let v = content[..idx].trim();
                    let r = content[idx + 1..].trim();
                    (v, r.to_string())
                } else {
                    (content, String::new())
                };
                if verdict_word.contains("水印") {
                    let detail = if reason.is_empty() {
                        "LLM 判定为水印".to_string()
                    } else {
                        reason
                    };
                    AdjudicationVerdict::IsWatermark { reason: detail }
                } else if verdict_word.contains("正文") {
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
        let char_limit = 10000;
        let sample: String = sample_text.chars().take(char_limit).collect();
        let system = "你是中文网络小说电子书制作助手。从给定章节文本推断书名、作者、简介和封面关键词。\
            输出格式(每行一项,未知的项写「未知」):\n\
            书名: XXX\n\
            作者: XXX\n\
            简介: XXX\n\
            封面关键词: XXX";
        let user = format!("章节文本:\n{sample}");

        let raw = self.chat(system, &user)?;
        let mut title = None;
        let mut author = None;
        let mut description = None;
        let mut cover_keywords = None;

        for line in raw.lines() {
            fn extract<'a>(line: &'a str, prefix_cn: &str, prefix_cn2: &str) -> Option<&'a str> {
                line.strip_prefix(prefix_cn)
                    .or_else(|| line.strip_prefix(prefix_cn2))
                    .map(str::trim)
                    .filter(|v| !v.is_empty() && *v != "未知")
            }
            if let Some(v) = extract(line, "书名:", "书名：") {
                title = Some(v.to_string());
            } else if let Some(v) = extract(line, "作者:", "作者：") {
                author = Some(v.to_string());
            } else if let Some(v) = extract(line, "简介:", "简介：") {
                description = Some(v.to_string());
            } else if let Some(v) = extract(line, "封面关键词:", "封面关键词：") {
                cover_keywords = Some(v.to_string());
            }
        }

        if title.is_none() && author.is_none() && description.is_none() {
            return Ok(None);
        }
        Ok(Some(MetadataSuggestion {
            title,
            author,
            description,
            cover_keywords,
        }))
    }
}
