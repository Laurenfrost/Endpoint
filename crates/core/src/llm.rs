//! LLM 客户端抽象。
//!
//! 核心库只暴露 trait + 空实现(NoopLlmClient);发 HTTP 请求的具体实现放在桥接层。
//! 任何功能都不得以「必须有 LLM」为前提(CLAUDE.md 第三节第 4 条)。
//!
//! # 设计原则
//! - `NoopLlmClient`:API key 未配置时的默认实现;所有方法返回 `LlmError::NotConfigured`。
//!   调用方对此错误的正确响应是**静默跳过**(降级路径),而非向用户报错。
//! - `LlmClient` 是 object-safe trait(`Box<dyn LlmClient>` 合法),便于桥接层
//!   在运行时替换实现而无需重启进程。

use crate::rules::Rule;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("HTTP 请求失败: {0}")]
    Http(String),
    #[error("响应解析失败: {0}")]
    Parse(String),
    #[error("LLM 未配置(API key 或 base_url 为空)")]
    NotConfigured,
}

/// 被 LLM 仲裁的水印候选行。
#[derive(Debug, Clone)]
pub struct WatermarkCandidate {
    /// 候选行原文(UTF-8,不含行尾换行符)。
    pub text: String,
    /// 前一行上下文(可选,用于让 LLM 看上下文)。
    pub context_before: Option<String>,
    /// 后一行上下文(可选)。
    pub context_after: Option<String>,
}

/// LLM 对单条水印候选的裁定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdjudicationVerdict {
    /// LLM 判定为水印,应当删除。附带理由供前端 signal 展示。
    IsWatermark { reason: String },
    /// LLM 判定为正文,应当保留。
    IsContent,
    /// LLM 无法确定,保留候选等待用户手动决策。
    Uncertain,
}

/// LLM 对元数据的推断结果。所有字段都可为空——LLM 应当对未知的字段留空而非编造。
#[derive(Debug, Clone, Default)]
pub struct MetadataSuggestion {
    pub title: Option<String>,
    pub author: Option<String>,
    /// 作品简介(可选,给前端"简介"字段填写参考)。
    pub description: Option<String>,
    /// 封面关键词建议(可选,供用户手动搜图参考;不自动搜图)。
    pub cover_keywords: Option<String>,
    /// 分类/标签(0..=3 个),写入 EPUB 的 `dc:subject`,供 Kobo / Calibre 书库分组。
    /// 例如:`["玄幻", "无限流"]`。
    pub subjects: Vec<String>,
    /// 系列名称(如「斗罗大陆」),写入 EPUB 的 `belongs-to-collection` + `calibre:series`。
    pub series: Option<String>,
    /// 系列内序号(从 1 起;EPUB `group-position` + `calibre:series_index`)。
    /// 仅当 [`series`] 非空时有意义。
    pub series_index: Option<u32>,
}

impl MetadataSuggestion {
    /// 至少有一个字段非空才算「LLM 给出了建议」。供调用方判断是否要触发 Pass B 搜索。
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.author.is_none()
            && self.description.is_none()
            && self.cover_keywords.is_none()
            && self.subjects.is_empty()
            && self.series.is_none()
            && self.series_index.is_none()
    }

    /// 任何主要字段为空都视为「需要搜索补全」。`cover_keywords` 不算主要字段。
    pub fn needs_fillin(&self) -> bool {
        self.title.is_none()
            || self.author.is_none()
            || self.description.is_none()
            || self.subjects.is_empty()
            || self.series.is_none()
    }

    /// 把另一个建议中的字段填到本建议的空缺处(只填空,不覆盖)。
    pub fn fill_from(&mut self, other: MetadataSuggestion) {
        if self.title.is_none() {
            self.title = other.title;
        }
        if self.author.is_none() {
            self.author = other.author;
        }
        if self.description.is_none() {
            self.description = other.description;
        }
        if self.cover_keywords.is_none() {
            self.cover_keywords = other.cover_keywords;
        }
        if self.subjects.is_empty() {
            self.subjects = other.subjects;
        }
        if self.series.is_none() {
            self.series = other.series;
        }
        if self.series_index.is_none() {
            self.series_index = other.series_index;
        }
    }
}

/// LLM 客户端接口。核心库依赖此 trait 抽象,桥接层提供具体 HTTP 实现。
///
/// 所有方法均可在 `spawn_blocking` 内同步调用。
pub trait LlmClient: Send + Sync {
    /// 批量仲裁水印候选。返回与 `candidates` 等长的裁定列表。
    ///
    /// 返回 `Err(LlmError::NotConfigured)` 时调用方应静默跳过,不向用户报错。
    fn arbitrate_watermark(
        &self,
        candidates: &[WatermarkCandidate],
    ) -> Result<Vec<AdjudicationVerdict>, LlmError>;

    /// 给定若干被用户「拒绝」的行样本,归纳一条可复用的正则规则。
    /// 若无法归纳则返回 `Ok(None)`。
    fn induce_rule(&self, rejected_lines: &[&str]) -> Result<Option<Rule>, LlmError>;

    /// 给定章节开头样本文本(以及可选的源文件名作为提示),推断书名与作者。
    ///
    /// `file_name` 是不含扩展名的源文件主名(如 `《蛊真人》精校版`),网文 txt 的文件名
    /// 通常已包含书名,作为额外线索喂给 LLM 能显著提高短样本下的命中率。
    fn suggest_metadata(
        &self,
        sample_text: &str,
        file_name: Option<&str>,
    ) -> Result<Option<MetadataSuggestion>, LlmError>;
}

/// 空实现:所有方法返回 `NotConfigured`。
/// 用于 API key 未配置时与单元测试场景。
pub struct NoopLlmClient;

impl LlmClient for NoopLlmClient {
    fn arbitrate_watermark(
        &self,
        _candidates: &[WatermarkCandidate],
    ) -> Result<Vec<AdjudicationVerdict>, LlmError> {
        Err(LlmError::NotConfigured)
    }

    fn induce_rule(&self, _rejected_lines: &[&str]) -> Result<Option<Rule>, LlmError> {
        Err(LlmError::NotConfigured)
    }

    fn suggest_metadata(
        &self,
        _sample_text: &str,
        _file_name: Option<&str>,
    ) -> Result<Option<MetadataSuggestion>, LlmError> {
        Err(LlmError::NotConfigured)
    }
}
