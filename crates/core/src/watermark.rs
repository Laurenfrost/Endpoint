//! 水印检测:本地廉价、可解释、零 LLM 依赖。
//!
//! 阶段三的核心模块,对应 CLAUDE.md 第七节「文本智能策略」中
//! 「本地廉价计算 + 多特征打分漏斗」部分。LLM 完全不参与(那是阶段四)。
//!
//! # 实施进度
//!
//! - **3.0(本子阶段)**:仅模块骨架 + 类型定义 + 空 `analyze` 函数返回空列表。
//!   留出与 [`crate::lib::run_pipeline`] 的接入点,使 3.1/3.2/3.3 可以平滑填入。
//! - 3.1:实装关键词正则特征(`keyword_regex`)+ 在 [`crate::rules`] 加内置 watermark 规则。
//! - 3.2:实装行频(`repetition`)+ 非中文占比(`non_cjk_ratio`)+ 加权融合 + 双阈值分流。
//! - 3.3:把 [`WatermarkConfig`] 经 [`crate::ConvertOptions`] 暴露 + auto 镜像到 cleaning。
//! - 3.4:前端 `Stage2Cleaning` 接入(本模块不参与)。
//!
//! # 不变式与契约
//!
//! 详见 [`crate::domain`] 模块文档第 6 节与 `docs/stage3-design.md` 第二节。
//! 简言之:本模块输出的 [`WatermarkAnnotation`] 列表按 `span.start` 升序,
//! 同一 span 至多一条 annotation(多特征命中合并 signals),
//! score ≥ `suspect_threshold`(低于灰区下阈值的不产出)。

use crate::domain::WatermarkAnnotation;
use crate::domain::Book;
use crate::domain::CleaningAnnotation;
use crate::rules::RuleSet;

/// 水印检测的可调参数。默认值见 [`Default`] 实现与 `docs/stage3-design.md` 第三节。
#[derive(Debug, Clone)]
pub struct WatermarkConfig {
    /// `score >= auto_threshold` → verdict = `auto`,镜像到 cleaning。默认 0.70。
    pub auto_threshold: f32,
    /// `suspect_threshold <= score < auto_threshold` → verdict = `suspect`,仅前端列表。默认 0.35。
    pub suspect_threshold: f32,
    /// 行频特征(`repetition`)的权重。默认 0.40。
    pub w_repeat: f32,
    /// 非中文占比特征(`non_cjk_ratio`)的权重。默认 0.20。
    pub w_non_cjk: f32,
    /// 关键词正则特征(`keyword_regex`)的权重。默认 0.40。
    pub w_keyword: f32,
    /// 行频统计触发的最小重复次数。低于此值的重复行不计 `repetition` 分。默认 5。
    pub repeat_count_min: u32,
    /// 短行豁免阈值:行字符数 < 此值时所有特征都不打分(避免把"嗯。"误标)。默认 4。
    pub min_line_chars: usize,
    /// 关闭水印检测开关。`false` 时 [`analyze`] 直接返回空列表;用于 A/B 与回归测试。默认 `true`。
    pub enabled: bool,
}

impl Default for WatermarkConfig {
    fn default() -> Self {
        Self {
            auto_threshold: 0.70,
            suspect_threshold: 0.35,
            w_repeat: 0.40,
            w_non_cjk: 0.20,
            w_keyword: 0.40,
            repeat_count_min: 5,
            min_line_chars: 4,
            enabled: true,
        }
    }
}

/// 扫描文本,产出水印标注列表。
///
/// 3.0 骨架阶段:无论参数如何,固定返回空列表——让上层 [`crate::lib::run_pipeline`]
/// 走通"无水印"路径,与阶段二行为等价。3.1/3.2 会填入真实实现。
///
/// # 参数(锁定签名)
///
/// - `source`:decoded source 文本。
/// - `book`:已识别的章节/卷边界。用于在扫描时跳过章节标题行(否则"第一卷 风云起"
///   出现在每卷开头会被行频特征误判)。
/// - `rules`:规则库;仅消费 [`crate::rules::RuleKind::Watermark`] 类规则。
/// - `cleaning_anns_base`:阶段二的基础清洗标注。3.2 之后可用于跳过已被清洗的区间。
/// - `config`:阈值与权重。
pub fn analyze(
    _source: &str,
    _book: &Book,
    _rules: &RuleSet,
    _cleaning_anns_base: &[CleaningAnnotation],
    _config: &WatermarkConfig,
) -> Vec<WatermarkAnnotation> {
    // 3.0 骨架:返回空列表。
    // 3.1 起会逐步填入关键词 / 行频 / 非中文占比三个特征。
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Book, Metadata};

    #[test]
    fn default_config_uses_documented_thresholds() {
        let c = WatermarkConfig::default();
        assert!((c.auto_threshold - 0.70).abs() < f32::EPSILON);
        assert!((c.suspect_threshold - 0.35).abs() < f32::EPSILON);
        assert!((c.w_repeat - 0.40).abs() < f32::EPSILON);
        assert!((c.w_non_cjk - 0.20).abs() < f32::EPSILON);
        assert!((c.w_keyword - 0.40).abs() < f32::EPSILON);
        // 权重总和应当为 1.0(或非常接近)
        let sum = c.w_repeat + c.w_non_cjk + c.w_keyword;
        assert!((sum - 1.0).abs() < 1e-6, "三特征权重总和应当为 1.0,实际为 {sum}");
        assert_eq!(c.repeat_count_min, 5);
        assert_eq!(c.min_line_chars, 4);
        assert!(c.enabled);
    }

    #[test]
    fn skeleton_analyze_returns_empty() {
        let book = Book {
            metadata: Metadata::new("测试", "作者"),
            entries: vec![],
        };
        let rules = RuleSet::builtin();
        let out = analyze("任何文本", &book, &rules, &[], &WatermarkConfig::default());
        assert!(out.is_empty(), "3.0 骨架阶段 analyze 必须返回空列表");
    }
}
