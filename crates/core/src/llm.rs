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
    /// LLM 判定为水印,应当删除。
    IsWatermark,
    /// LLM 判定为正文,应当保留。
    IsContent,
    /// LLM 无法确定,保留候选等待用户手动决策。
    Uncertain,
}

/// LLM 对书名/作者的推断结果。
#[derive(Debug, Clone)]
pub struct MetadataSuggestion {
    pub title: Option<String>,
    pub author: Option<String>,
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

    /// 给定章节开头样本文本,推断书名与作者。
    fn suggest_metadata(&self, sample_text: &str) -> Result<Option<MetadataSuggestion>, LlmError>;
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

    fn suggest_metadata(&self, _sample_text: &str) -> Result<Option<MetadataSuggestion>, LlmError> {
        Err(LlmError::NotConfigured)
    }
}
