//! 水印检测:本地廉价、可解释、零 LLM 依赖。
//!
//! 阶段三的核心模块,对应 CLAUDE.md 第七节「文本智能策略」中
//! 「本地廉价计算 + 多特征打分漏斗」部分。LLM 完全不参与(那是阶段四)。
//!
//! # 实施进度
//!
//! - 3.0:模块骨架 + 类型定义 + 空 `analyze` 函数(已完成)。
//! - **3.1(本子阶段)**:关键词正则特征(`keyword_regex`)+ 在 [`crate::rules`] 加内置
//!   watermark 规则。单 keyword 命中默认权重融合 = `w_keyword * 1.0 = 0.40`,落 **suspect** 灰区。
//! - 3.2:行频(`repetition`)+ 非中文占比(`non_cjk_ratio`)+ 加权融合。
//! - 3.3:把 [`WatermarkConfig`] 经 [`crate::ConvertOptions`] 暴露 + auto 镜像到 cleaning。
//! - 3.4:前端 `Stage2Cleaning` 接入(本模块不参与)。
//!
//! # 不变式与契约
//!
//! 详见 [`crate::domain`] 模块文档第 6 节与 `docs/stage3-design.md` 第二节。
//! 简言之:本模块输出的 [`WatermarkAnnotation`] 列表按 `span.start` 升序,
//! 同一 span 至多一条 annotation(多特征命中合并 signals),
//! score ≥ `suspect_threshold`(低于灰区下阈值的不产出)。

use std::collections::HashSet;

use regex::Regex;

use crate::domain::{
    Book, BookEntry, CleaningAnnotation, Span, WatermarkAnnotation, WatermarkSignal,
    WatermarkSignalKind, WatermarkVerdict,
};
use crate::rules::{RuleKind, RuleSet};

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
/// 3.1 阶段实装 **关键词正则** 一个特征:逐行扫描 `source`,跳过空白行 + 章节/卷标题行,
/// 然后让每行经过所有启用的 [`RuleKind::Watermark`] 规则。命中即产生一条
/// `signals = [{ kind: keyword_regex, score: 1.0, detail: "命中规则 <id>" }]`,
/// 融合分 = `w_keyword * 1.0`(默认 0.40)→ 落 **suspect** 灰区(EPUB 保留,前端黄色)。
///
/// 3.2 子阶段会在此函数内追加 `repetition` / `non_cjk_ratio` 两个特征,合并 signals;
/// 因此本函数的输出结构(多 signal 合并、score 加权融合)从 3.1 起就按最终形态写。
///
/// # 参数(锁定签名)
///
/// - `source`:decoded source 文本。
/// - `book`:已识别的章节/卷边界。用于在扫描时跳过章节标题行(否则"第一卷 风云起"
///   出现在每卷开头会被行频特征误判;关键词特征本身也不应把章节标题误标)。
/// - `rules`:规则库;仅消费 [`RuleKind::Watermark`] 类规则。
/// - `cleaning_anns_base`:阶段二的基础清洗标注。3.2 之后可用于跳过已被清洗的区间;3.1 不消费。
/// - `config`:阈值与权重。
pub fn analyze(
    source: &str,
    book: &Book,
    rules: &RuleSet,
    _cleaning_anns_base: &[CleaningAnnotation],
    config: &WatermarkConfig,
) -> Vec<WatermarkAnnotation> {
    if !config.enabled {
        return Vec::new();
    }

    // 预编译启用的 watermark 规则。失败的规则(理论上不会发生——内置规则均有测试覆盖)直接跳过。
    let compiled: Vec<(String, Regex)> = rules
        .enabled_by_kind(RuleKind::Watermark)
        .into_iter()
        .filter_map(|r| r.compile().ok().map(|re| (r.id.clone(), re)))
        .collect();

    // 没有可用规则 → 关键词特征产不出任何 signal,3.1 阶段直接返回空。
    if compiled.is_empty() {
        return Vec::new();
    }

    // 收集所有 chapter/volume 标题行的起始字节,用于扫描时跳过。
    let heading_starts = collect_heading_starts(book);

    let mut out: Vec<WatermarkAnnotation> = Vec::new();
    // TODO(cancel): 接 ConvertOptions.cancel_token 后,每 N 行检查一次取消标志。
    for (line_start, line_end) in iter_lines(source) {
        // 跳过章节/卷标题行
        if heading_starts.contains(&line_start) {
            continue;
        }
        let line = &source[line_start..line_end];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 短行豁免(避免把"嗯。"等口头禅误标)
        if trimmed.chars().take(config.min_line_chars).count() < config.min_line_chars {
            continue;
        }

        // —— 关键词特征 ——
        // 多条规则同时命中时,只生成一条 signal(kind = keyword_regex),detail 列出命中规则 id 列表。
        // 这样上层 UI 可以展示"命中规则 A, B, C",同时分数语义清晰(命中即 1.0,不重复计权)。
        let mut hit_ids: Vec<&str> = Vec::new();
        for (id, re) in &compiled {
            if re.is_match(trimmed) {
                hit_ids.push(id);
            }
        }
        if hit_ids.is_empty() {
            continue;
        }

        let signal = WatermarkSignal {
            kind: WatermarkSignalKind::KeywordRegex,
            score: 1.0,
            detail: Some(format!("命中规则 {}", hit_ids.join(", "))),
        };
        let signals = vec![signal];
        let score = fused_score(&signals, config);
        let verdict = match classify(score, config) {
            Some(v) => v,
            None => continue,
        };

        out.push(WatermarkAnnotation {
            span: Span::new(line_start, line_end),
            verdict,
            score,
            signals,
        });
    }

    // out 已按 line_start 升序产出(iter_lines 顺序扫描)。
    out
}

/// 按当前 [`WatermarkConfig`] 把 signals 列表融合为总分 `[0, 1]`。
///
/// 公式见 `docs/stage3-design.md` 第三节 3.3:
/// `score = w_repeat * s_repeat + w_non_cjk * s_non_cjk + w_keyword * s_keyword`,
/// 每个特征只取一次最大值(同 kind 多 signal 取 max),最终结果 clamp 到 `[0, 1]`。
fn fused_score(signals: &[WatermarkSignal], config: &WatermarkConfig) -> f32 {
    let mut s_repeat: f32 = 0.0;
    let mut s_non_cjk: f32 = 0.0;
    let mut s_keyword: f32 = 0.0;
    for s in signals {
        let target = match s.kind {
            WatermarkSignalKind::Repetition => &mut s_repeat,
            WatermarkSignalKind::NonCjkRatio => &mut s_non_cjk,
            WatermarkSignalKind::KeywordRegex => &mut s_keyword,
        };
        if s.score > *target {
            *target = s.score;
        }
    }
    let raw =
        config.w_repeat * s_repeat + config.w_non_cjk * s_non_cjk + config.w_keyword * s_keyword;
    raw.clamp(0.0, 1.0)
}

/// 按双阈值分流;低于 suspect_threshold 返回 None(不产出 annotation)。
fn classify(score: f32, config: &WatermarkConfig) -> Option<WatermarkVerdict> {
    if score >= config.auto_threshold {
        Some(WatermarkVerdict::Auto)
    } else if score >= config.suspect_threshold {
        Some(WatermarkVerdict::Suspect)
    } else {
        None
    }
}

/// 收集所有 chapter/volume 真实标题行的起始字节。
///
/// Fallback 章(`heading_span` 为空,如"楔子"前缀章或"(卷前)"占位章)的标题不对应真实行,
/// 不写入集合;此类 span 的 start 经常落在第一个真实标题行之前或紧贴卷头,放进集合会误伤。
fn collect_heading_starts(book: &Book) -> HashSet<usize> {
    let mut set = HashSet::new();
    for entry in &book.entries {
        match entry {
            BookEntry::Chapter(c) => {
                if !c.heading_span.is_empty() {
                    set.insert(c.heading_span.start);
                }
            }
            BookEntry::Volume(v) => {
                if !v.heading_span.is_empty() {
                    set.insert(v.heading_span.start);
                }
                for c in &v.chapters {
                    if !c.heading_span.is_empty() {
                        set.insert(c.heading_span.start);
                    }
                }
            }
        }
    }
    set
}

/// 行迭代:按 `\n` 分隔,返回 `(line_start, line_end)` 半开字节区间(不含 `\n`)。
/// 与 `chapter::iter_lines` / `cleaning.rs` 中的行迭代逻辑一致——刻意各处分别保留一份,
/// 避免把这种 3 行的小工具提升为跨模块公共 API。
fn iter_lines(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut line_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            out.push((line_start, i));
            line_start = i + 1;
        }
        i += 1;
    }
    if line_start < bytes.len() {
        out.push((line_start, bytes.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chapter;
    use crate::domain::Metadata;

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

    /// 用本模块的最小构造跑一次 chapter::parse,产出 book 给 analyze 用。
    /// 不调 chapter::materialize_paragraphs(本模块测试不依赖段落物化)。
    fn parse_book(text: &str) -> Book {
        chapter::parse(text, &RuleSet::builtin(), Metadata::new("测试", "作者")).unwrap()
    }

    fn analyze_default(text: &str) -> Vec<WatermarkAnnotation> {
        let book = parse_book(text);
        analyze(
            text,
            &book,
            &RuleSet::builtin(),
            &[],
            &WatermarkConfig::default(),
        )
    }

    #[test]
    fn keyword_url_lands_as_suspect_with_default_weights() {
        // 单 keyword 命中 = w_keyword × 1.0 = 0.40 → suspect(>= 0.35,< 0.70)
        let text = "\
第一章 起
正文一。
请访问 https://novel.example.com/123 查看更新。
正文二。
";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1, "应当只有 URL 这一行被标");
        let w = &out[0];
        assert_eq!(w.verdict, WatermarkVerdict::Suspect);
        assert!((w.score - 0.40).abs() < 1e-5, "实际分数 {}", w.score);
        assert_eq!(w.signals.len(), 1);
        assert_eq!(w.signals[0].kind, WatermarkSignalKind::KeywordRegex);
        assert!((w.signals[0].score - 1.0).abs() < f32::EPSILON);
        assert!(
            w.signals[0]
                .detail
                .as_ref()
                .map(|s| s.contains("builtin-watermark-url-http"))
                .unwrap_or(false),
            "detail 应当包含命中规则 id,实际为 {:?}",
            w.signals[0].detail
        );
    }

    #[test]
    fn span_covers_whole_line() {
        let text = "正文。\n本文首发于纵横中文网。\n下一段。\n";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1);
        let line_start = text.find("本文首发").unwrap();
        let line_end = line_start + "本文首发于纵横中文网。".len();
        assert_eq!(out[0].span.start, line_start);
        assert_eq!(out[0].span.end, line_end);
        // span 内容应当就是被命中那行
        assert_eq!(
            &text[out[0].span.start..out[0].span.end],
            "本文首发于纵横中文网。"
        );
    }

    #[test]
    fn skips_chapter_and_volume_heading_lines() {
        // 章节/卷标题里**故意**塞一段会被 watermark 关键词匹配的文字,但因为这行是 heading,
        // 必须被跳过——确认本模块按 book.entries 过滤 heading_span。
        // 用 `首发于` 作为关键词触发(builtin-watermark-first-publish),
        // 同时仍能被 chapter 的「第一章」格式识别。
        let text = "\
第一章 首发于
正文一。
第二章 起
正文二。
";
        let book = parse_book(text);
        // sanity:章节确实识别出来了
        assert_eq!(book.entries.len(), 2);
        let out = analyze(
            text,
            &book,
            &RuleSet::builtin(),
            &[],
            &WatermarkConfig::default(),
        );
        assert!(
            out.is_empty(),
            "章节标题行不应被 watermark 标:{:?}",
            out
        );
    }

    #[test]
    fn skips_empty_and_short_lines() {
        // 短行豁免:`min_line_chars = 4`,所以「@abc」不会被检测(即使能匹配 TG 规则,但太短)。
        // 实际 @abc 也太短不命中(TG 规则要求 ≥5 字符);用「免费」试 —— 太短直接豁免。
        let text = "\n\n免费\n正文有效内容。\n";
        let out = analyze_default(text);
        assert!(out.is_empty());
    }

    #[test]
    fn disabled_config_returns_empty_even_with_obvious_watermark() {
        let text = "首发于纵横中文网。\n请访问 https://example.com\n";
        let book = parse_book(text);
        let mut config = WatermarkConfig::default();
        config.enabled = false;
        let out = analyze(text, &book, &RuleSet::builtin(), &[], &config);
        assert!(out.is_empty(), "关闭开关后应当固定返回空");
    }

    #[test]
    fn multiple_rule_hits_on_same_line_produce_one_signal_with_combined_detail() {
        // 同时命中 URL + 首发 两条规则,只产 1 条 signal,detail 列两个 rule id
        let text = "本文首发于 https://example.com/novel\n";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].signals.len(), 1, "多规则命中应合并为 1 个 keyword_regex signal");
        let detail = out[0].signals[0].detail.as_deref().unwrap_or("");
        assert!(detail.contains("builtin-watermark-url-http"));
        assert!(detail.contains("builtin-watermark-first-publish"));
        // 分数仍为 w_keyword × 1.0(不会因为多规则而 > 0.40)
        assert!((out[0].score - 0.40).abs() < 1e-5);
    }

    #[test]
    fn normal_prose_does_not_get_flagged() {
        // 完整一小段网文正文,不应有任何水印命中
        let text = "\
第一章 起
他抬起头,望着远方的天空。
夜色渐浓,街上的人也少了。
「你怎么了?」她轻声问道。
他没有回答,只是叹了口气。
第二章 承
故事继续。
";
        let out = analyze_default(text);
        assert!(out.is_empty(), "正文不应被误标:{:?}", out);
    }

    #[test]
    fn analyze_result_is_sorted_by_span_start() {
        let text = "\
正文。
首发于纵横中文网。
正文二。
请访问 https://example.com 阅读。
更新最快,无广告阅读。
正文三。
";
        let out = analyze_default(text);
        assert_eq!(out.len(), 3);
        for w in out.windows(2) {
            assert!(
                w[0].span.start < w[1].span.start,
                "annotations 必须按 span.start 升序"
            );
        }
    }

    #[test]
    fn fused_score_and_classify_thresholds() {
        let cfg = WatermarkConfig::default();
        // 单 keyword 1.0 → 0.40 → suspect
        let sig_kw = vec![WatermarkSignal {
            kind: WatermarkSignalKind::KeywordRegex,
            score: 1.0,
            detail: None,
        }];
        assert!((fused_score(&sig_kw, &cfg) - 0.40).abs() < 1e-5);
        assert_eq!(classify(0.40, &cfg), Some(WatermarkVerdict::Suspect));
        // keyword + repetition(均 1.0)→ 0.80 → auto
        let sig_two = vec![
            WatermarkSignal {
                kind: WatermarkSignalKind::KeywordRegex,
                score: 1.0,
                detail: None,
            },
            WatermarkSignal {
                kind: WatermarkSignalKind::Repetition,
                score: 1.0,
                detail: None,
            },
        ];
        assert!((fused_score(&sig_two, &cfg) - 0.80).abs() < 1e-5);
        assert_eq!(classify(0.80, &cfg), Some(WatermarkVerdict::Auto));
        // 全 0 → drop
        assert_eq!(classify(0.0, &cfg), None);
        // 边界:正好等于 suspect_threshold → suspect;正好等于 auto_threshold → auto
        assert_eq!(classify(0.35, &cfg), Some(WatermarkVerdict::Suspect));
        assert_eq!(classify(0.70, &cfg), Some(WatermarkVerdict::Auto));
    }

    #[test]
    fn empty_rule_set_returns_empty() {
        // 把 watermark 规则全 disable,analyze 应返回空(关键词分支无法触发,3.2 才有其他特征)
        let mut rules = RuleSet::builtin();
        let mut to_disable: Vec<String> = rules
            .rules
            .iter()
            .filter(|r| r.kind == RuleKind::Watermark)
            .map(|r| r.id.clone())
            .collect();
        for id in to_disable.drain(..) {
            let mut r = rules.find(&id).cloned().unwrap();
            r.enabled = false;
            rules.upsert(r);
        }
        let text = "请访问 https://example.com\n首发于纵横\n";
        let book = parse_book(text);
        let out = analyze(text, &book, &rules, &[], &WatermarkConfig::default());
        assert!(out.is_empty(), "无启用 watermark 规则时应返回空");
    }
}
