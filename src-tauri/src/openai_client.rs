//! OpenAI 兼容 HTTP 客户端(桥接层实现)。
//!
//! 兼容 `/v1/chat/completions` 接口的任何服务都可接入:DeepSeek、OpenAI、本地 Ollama 等。
//! reqwest blocking 在 `spawn_blocking` 内调用,不阻塞 Tauri 异步运行时。

use std::sync::Arc;

use endpoint_core::llm::{
    AdjudicationVerdict, LlmClient, LlmError, MetadataSuggestion, WatermarkCandidate,
};
use endpoint_core::rules::{Rule, RuleKind, RuleSource};
use endpoint_core::search::{SearchError, SearchResult, WebSearch};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use tracing::{debug, info, trace, warn};

pub struct OpenAiCompatibleClient {
    base_url: String,
    model: String,
    api_key: String,
    client: Client,
    /// 可选的 Web 搜索后端,用于 `suggest_metadata` 的 Pass B 兜底。
    /// `None` 或 [`endpoint_core::search::NoopWebSearch`] 表示禁用搜索。
    search: Option<Arc<dyn WebSearch>>,
}

impl OpenAiCompatibleClient {
    /// 携带搜索后端的构造:`suggest_metadata` 在 LLM 输出有空字段时会调它做 Pass B。
    pub fn with_search(
        base_url: String,
        model: String,
        api_key: String,
        search: Option<Arc<dyn WebSearch>>,
    ) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            search,
        }
    }

    fn chat(&self, system: &str, user: &str) -> Result<String, LlmError> {
        self.chat_inner(system, user, false)
    }

    /// 要求 LLM 以 JSON 对象格式输出。DeepSeek/OpenAI 兼容接口要求 system prompt
    /// 含 "JSON" 字样,否则 `response_format` 不会生效——本方法的调用方保证如此。
    fn chat_json(&self, system: &str, user: &str) -> Result<String, LlmError> {
        self.chat_inner(system, user, true)
    }

    fn chat_inner(&self, system: &str, user: &str, json_mode: bool) -> Result<String, LlmError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user",   "content": user   }
            ],
            "temperature": 0.1,
            "max_tokens": 800,
        });
        if json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }

        info!(
            url = %url,
            model = %self.model,
            system_chars = system.chars().count(),
            user_chars = user.chars().count(),
            "LLM 请求开始"
        );
        // 完整 prompt 仅在 trace 级输出(可能含用户文本)
        trace!(system_prompt = system, user_prompt = user, "完整 prompt");

        let started = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|e| {
                warn!(error = %e, elapsed_ms = started.elapsed().as_millis() as u64, "LLM 请求网络层失败");
                LlmError::Http(e.to_string())
            })?;

        let status = resp.status();
        let elapsed_ms = started.elapsed().as_millis() as u64;

        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            warn!(
                status = %status,
                elapsed_ms,
                body_preview = %text.chars().take(200).collect::<String>(),
                "LLM 请求非 2xx"
            );
            return Err(LlmError::Http(format!("HTTP {status}: {text}")));
        }

        let json: Value = resp.json().map_err(|e| {
            warn!(error = %e, elapsed_ms, "LLM 响应 JSON 解析失败");
            LlmError::Parse(e.to_string())
        })?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                warn!(elapsed_ms, "LLM 响应 choices[0].message.content 缺失");
                LlmError::Parse("响应中未找到 content 字段".into())
            })?
            .to_string();
        info!(
            status = %status,
            elapsed_ms,
            response_chars = content.chars().count(),
            "LLM 请求完成"
        );
        trace!(response = %content, "完整响应");
        Ok(content)
    }
}

impl LlmClient for OpenAiCompatibleClient {
    fn arbitrate_watermark(
        &self,
        candidates: &[WatermarkCandidate],
    ) -> Result<Vec<AdjudicationVerdict>, LlmError> {
        debug!(candidates = candidates.len(), "arbitrate_watermark");
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
        let watermark = verdicts
            .iter()
            .filter(|v| matches!(v, AdjudicationVerdict::IsWatermark { .. }))
            .count();
        let content = verdicts
            .iter()
            .filter(|v| matches!(v, AdjudicationVerdict::IsContent))
            .count();
        let uncertain = verdicts.len() - watermark - content;
        debug!(watermark, content, uncertain, "arbitrate_watermark 解析完成");
        Ok(verdicts)
    }

    fn induce_rule(&self, rejected_lines: &[&str]) -> Result<Option<Rule>, LlmError> {
        debug!(samples = rejected_lines.len(), "induce_rule");
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
            debug!("induce_rule: LLM 返回空字符串");
            return Ok(None);
        }

        // 验证正则能编译,否则丢弃
        if let Err(e) = regex::Regex::new(&raw) {
            warn!(pattern = %raw, error = %e, "induce_rule: LLM 返回的正则无法编译,丢弃");
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

    fn suggest_metadata(
        &self,
        sample_text: &str,
        file_name: Option<&str>,
    ) -> Result<Option<MetadataSuggestion>, LlmError> {
        let char_limit = 1000;
        let sample: String = sample_text.chars().take(char_limit).collect();
        debug!(
            sample_chars = sample.chars().count(),
            file_name = file_name.unwrap_or("<none>"),
            has_search = self.search.is_some(),
            "suggest_metadata"
        );

        // ===== Pass A:仅凭文件名 + 正文样本,让 LLM 输出 JSON =====
        let mut sug = match self.suggest_metadata_pass_a(&sample, file_name)? {
            Some(s) => s,
            None => MetadataSuggestion::default(),
        };
        debug!(
            has_title = sug.title.is_some(),
            has_author = sug.author.is_some(),
            has_description = sug.description.is_some(),
            subjects_n = sug.subjects.len(),
            has_series = sug.series.is_some(),
            "suggest_metadata Pass A 完成"
        );

        // ===== Pass B(可选):有搜索且 A 留有空缺,跑一次搜索补齐 =====
        if sug.needs_fillin() && self.search.is_some() {
            let search = self.search.as_ref().unwrap().clone();
            let query = build_search_query(&sug, file_name);
            info!(query = %query, "suggest_metadata: 触发 Pass B 搜索");
            match search.search(&query) {
                Ok(results) if !results.is_empty() => {
                    match self.suggest_metadata_pass_b(&sample, file_name, &sug, &results) {
                        Ok(Some(b)) => {
                            debug!(
                                b_subjects_n = b.subjects.len(),
                                b_has_series = b.series.is_some(),
                                "Pass B 返回建议"
                            );
                            sug.fill_from(b);
                        }
                        Ok(None) => {
                            warn!("Pass B: LLM 仍未给出可用字段");
                        }
                        Err(e) => {
                            // 搜索成功但 LLM 失败:Pass A 结果照常返回,不向上抛错。
                            warn!(error = %e, "Pass B LLM 调用失败,沿用 Pass A 结果");
                        }
                    }
                }
                Ok(_) => {
                    info!("Pass B 搜索无结果");
                }
                Err(SearchError::NotConfigured) => {
                    debug!("Pass B 跳过:搜索后端未配置");
                }
                Err(e) => {
                    warn!(error = %e, "Pass B 搜索失败,沿用 Pass A 结果");
                }
            }
        }

        if sug.is_empty() {
            Ok(None)
        } else {
            Ok(Some(sug))
        }
    }
}

// ============ suggest_metadata 内部实现 ============

impl OpenAiCompatibleClient {
    /// Pass A:零搜索上下文,纯靠文件名 + 正文样本 + LLM 训练知识。
    fn suggest_metadata_pass_a(
        &self,
        sample: &str,
        file_name: Option<&str>,
    ) -> Result<Option<MetadataSuggestion>, LlmError> {
        let system = METADATA_JSON_SYSTEM_PROMPT;
        let user = match file_name {
            Some(name) => format!(
                "文件名:{name}\n\n章节开头文本:\n{sample}\n\n\
                 如果你从训练数据中认得这本小说,请尽量基于已有知识完整填写所有字段;\
                 如果你不认得,只填能在上述文件名/正文中直接得到的字段,其他留空。\
                 请按规定 JSON 格式输出。"
            ),
            None => format!(
                "章节开头文本:\n{sample}\n\n\
                 请按规定 JSON 格式输出,仅填能从正文确认的字段,未知的留空。"
            ),
        };
        let raw = self.chat_json(system, &user)?;
        parse_metadata_json(&raw, "Pass A")
    }

    /// Pass B:把搜索结果片段塞进 prompt,让 LLM 补齐 Pass A 留下的空缺。
    fn suggest_metadata_pass_b(
        &self,
        sample: &str,
        file_name: Option<&str>,
        pass_a: &MetadataSuggestion,
        results: &[SearchResult],
    ) -> Result<Option<MetadataSuggestion>, LlmError> {
        let system = METADATA_JSON_SYSTEM_PROMPT;

        let snippets = results
            .iter()
            .take(5)
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "{n}. 标题:{t}\n   URL:{u}\n   摘要:{s}",
                    n = i + 1,
                    t = r.title,
                    u = r.url,
                    s = r.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        // 把 Pass A 已知字段拼出来,提示 LLM 这些不要改,只补空白。
        let mut known_lines = Vec::new();
        if let Some(t) = &pass_a.title {
            known_lines.push(format!("- 已知书名:{t}"));
        }
        if let Some(a) = &pass_a.author {
            known_lines.push(format!("- 已知作者:{a}"));
        }
        if let Some(d) = &pass_a.description {
            known_lines.push(format!("- 已知简介:{d}"));
        }
        if !pass_a.subjects.is_empty() {
            known_lines.push(format!("- 已知分类:{}", pass_a.subjects.join("、")));
        }
        if let Some(s) = &pass_a.series {
            known_lines.push(format!("- 已知系列:{s}"));
        }
        let known = if known_lines.is_empty() {
            "(尚无任何已知字段)".to_string()
        } else {
            known_lines.join("\n")
        };

        let user = format!(
            "下面是我从网上搜来的关于这本小说的资料:\n\n\
             {snippets}\n\n\
             我已经知道的字段(请不要否定它们,只补齐空缺):\n{known}\n\n\
             文件名:{file}\n\n\
             章节开头样本(供交叉验证):\n{sample}\n\n\
             请综合以上信息按规定 JSON 格式输出元数据,着重补齐分类、系列、简介等之前没法确认的字段。\
             如果搜索资料里没有可靠依据,该字段就留空,不要硬猜。",
            file = file_name.unwrap_or("(未知)"),
        );

        let raw = self.chat_json(system, &user)?;
        parse_metadata_json(&raw, "Pass B")
    }
}

/// 系统 prompt:JSON 输出契约。两个 Pass 共用,降低 LLM 行为漂移。
/// 注意必须含 "JSON" 字样,DeepSeek 才会启用 `response_format=json_object`。
const METADATA_JSON_SYSTEM_PROMPT: &str = r#"你是中文网络小说电子书制作助手。从用户提供的信息中推断电子书元数据,以 JSON 对象输出。

输出 JSON 结构(所有字段都可省略或为 null;不要编造,不知道就空):
{
  "title": "书名(字符串或 null)",
  "author": "作者笔名(字符串或 null)",
  "description": "100-300 字的简介(字符串或 null)",
  "subjects": ["分类1", "分类2"],
  "series": "所属系列名,如「斗破苍穹」(字符串或 null;独立作品则 null)",
  "series_index": 1,
  "cover_keywords": "若做封面可参考的视觉关键词,逗号分隔(字符串或 null)"
}

规则:
1. 网文 txt 的文件名通常已包含书名,可能带「精校版」「完结」「全本」等后缀;请去掉这些后缀后采用。
2. subjects 给 1-3 个中文标签,如「玄幻」「都市」「无限流」「修真」「轻小说」;没把握就给空数组。
3. series 仅当确实是续作/系列时填;单本独立作品填 null。
4. series_index 是该书在系列中的顺序(从 1 起),无系列或不确定时填 null。
5. 不要输出 Markdown、不要解释,只输出一个合法 JSON 对象。"#;

/// 解析 LLM 输出的 JSON 元数据。容忍以下偏差:
/// - 整个响应被 ```json ... ``` 围栏包裹
/// - 字段值为字符串 "null" 或 "未知"
/// - subjects 是字符串(逗号/顿号分隔)而非数组
fn parse_metadata_json(raw: &str, pass_label: &str) -> Result<Option<MetadataSuggestion>, LlmError> {
    let stripped = strip_code_fence(raw);
    let val: Value = match serde_json::from_str(stripped) {
        Ok(v) => v,
        Err(e) => {
            let preview: String = raw.chars().take(200).collect();
            warn!(
                pass = pass_label,
                error = %e,
                response_chars = raw.chars().count(),
                response_preview = %preview,
                "suggest_metadata: LLM 响应不是合法 JSON"
            );
            return Ok(None);
        }
    };

    let str_field = |key: &str| -> Option<String> {
        val.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "未知" && *s != "null")
            .map(str::to_string)
    };

    let subjects = match val.get("subjects") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "未知")
            .collect::<Vec<_>>(),
        Some(Value::String(s)) => s
            .split([',', '，', '、', '/', '|'])
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty() && x != "未知")
            .collect(),
        _ => Vec::new(),
    };

    let series_index = val
        .get("series_index")
        .and_then(|v| match v {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        })
        .and_then(|n| u32::try_from(n).ok());

    let sug = MetadataSuggestion {
        title: str_field("title"),
        author: str_field("author"),
        description: str_field("description"),
        cover_keywords: str_field("cover_keywords"),
        subjects,
        series: str_field("series"),
        series_index,
    };

    if sug.is_empty() {
        let preview: String = raw.chars().take(200).collect();
        warn!(
            pass = pass_label,
            response_preview = %preview,
            "suggest_metadata: JSON 解析成功但所有字段为空"
        );
        return Ok(None);
    }
    Ok(Some(sug))
}

fn strip_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        return rest.trim().trim_end_matches("```").trim();
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return rest.trim().trim_end_matches("```").trim();
    }
    trimmed
}

/// 构造 Pass B 的搜索 query。优先用 Pass A 已知书名;否则回退到文件名 stem。
fn build_search_query(pass_a: &MetadataSuggestion, file_name: Option<&str>) -> String {
    let base = pass_a
        .title
        .as_deref()
        .or(file_name)
        .unwrap_or("中文网络小说")
        .to_string();
    // 去掉常见的版本后缀,提高检索命中率
    let cleaned = base
        .replace("精校版", "")
        .replace("完结版", "")
        .replace("全本", "")
        .replace("完结", "")
        .replace('《', "")
        .replace('》', "")
        .trim()
        .to_string();
    // 用「网络小说 作者 简介」等关键词引导,中文搜索引擎更容易命中百度百科/书评页
    format!("{cleaned} 网络小说 作者 简介")
}
