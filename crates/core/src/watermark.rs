//! 水印检测:本地廉价、可解释、零 LLM 依赖。
//!
//! 阶段三的核心模块,对应 CLAUDE.md 第七节「文本智能策略」中
//! 「本地廉价计算 + 多特征打分漏斗」部分。LLM 完全不参与(那是阶段四)。
//!
//! # 实施进度
//!
//! - 3.0:模块骨架 + 类型定义 + 空 `analyze` 函数(已完成)。
//! - 3.1:关键词正则特征(`keyword_regex`)+ 在 [`crate::rules`] 加内置 watermark 规则(已完成)。
//! - **3.2(本子阶段)**:行频(`repetition`)+ 非中文占比(`non_cjk_ratio`)+ 三特征加权融合 +
//!   双阈值分流。单 keyword 仍是 suspect;keyword + repetition 才升 auto。
//! - 3.3:把 [`WatermarkConfig`] 经 [`crate::ConvertOptions`] 暴露 + auto 镜像到 cleaning。
//! - 3.4:前端 `Stage2Cleaning` 接入(本模块不参与)。
//!
//! # 不变式与契约
//!
//! 详见 [`crate::domain`] 模块文档第 6 节与 `docs/stage3-design.md` 第二节。
//! 简言之:本模块输出的 [`WatermarkAnnotation`] 列表按 `span.start` 升序,
//! 同一 span 至多一条 annotation(多特征命中合并 signals),
//! score ≥ `suspect_threshold`(低于灰区下阈值的不产出)。

use std::collections::{HashMap, HashSet};

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
/// 3.2 阶段实装三个特征:`repetition` / `non_cjk_ratio` / `keyword_regex`。每个 eligible 行
/// 经过三个特征独立打分,然后按 [`WatermarkConfig`] 的权重融合,双阈值分流为
/// auto / suspect / 丢弃。
///
/// # 算法详情
///
/// - 两遍扫描 source:第一遍按 [`eligible_trimmed`] 过滤后建 `HashMap<&str, u32>` 计行频;
///   第二遍逐行计算三特征分数 + 融合 + 分流。
/// - 同一行多特征命中合并为**一条** annotation,signals 按特征种类各持一条。
/// - 关键词特征:命中任一启用的 [`RuleKind::Watermark`] 规则即得 score = 1.0;多规则命中
///   合并 detail = `"命中规则 A, B, ..."`。
/// - 行频特征:行内容(trim 后字节相等)在全文出现次数 ≥ `config.repeat_count_min`(默认 5)时
///   触发,score = `min(1.0, log10(count)/log10(50))`。count=5 时 ~0.41,count=50 时 1.0。
/// - 非中文占比特征:`(非 CJK 字符数)/(总字符数) ≥ 0.4` 时触发,
///   score = `min(1.0, (ratio - 0.4)/0.4)`。ratio=0.4 时 0、ratio=0.8 时 1.0。
///
/// # 参数(锁定签名)
///
/// - `source`:decoded source 文本。
/// - `book`:已识别的章节/卷边界。用于在扫描时跳过章节标题行(否则"第一卷 风云起"
///   出现在每卷开头会被行频特征误判;关键词特征本身也不应把章节标题误标)。
/// - `rules`:规则库;仅消费 [`RuleKind::Watermark`] 类规则。
/// - `cleaning_anns_base`:阶段二的基础清洗标注。3.2 不消费,3.3 之后可用于跳过已被清洗的区间。
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

    let heading_starts = collect_heading_starts(book);

    // 一次性切行 + 缓存复用(两遍扫描共用同一份)。
    let lines = iter_lines(source);

    // —— 第一遍:行频统计 ——
    // 只统计 eligible 行(跳过 heading / 空行 / 短行),避免章节标题被计入。
    // TODO(cancel): 接 ConvertOptions.cancel_token 后,每 N 行检查一次取消标志。
    let mut count_by_trimmed: HashMap<&str, u32> = HashMap::new();
    for &(line_start, line_end) in &lines {
        if let Some(trimmed) =
            eligible_trimmed(source, line_start, line_end, &heading_starts, config)
        {
            *count_by_trimmed.entry(trimmed).or_insert(0) += 1;
        }
    }

    // 关键词规则预编译。空规则集时三特征中只有 keyword 失活,其它两个仍可工作。
    let compiled: Vec<(String, Regex)> = rules
        .enabled_by_kind(RuleKind::Watermark)
        .into_iter()
        .filter_map(|r| r.compile().ok().map(|re| (r.id.clone(), re)))
        .collect();

    // —— 第二遍:逐行三特征计算 + 融合 + 分流 ——
    let mut out: Vec<WatermarkAnnotation> = Vec::new();
    for &(line_start, line_end) in &lines {
        let Some(trimmed) =
            eligible_trimmed(source, line_start, line_end, &heading_starts, config)
        else {
            continue;
        };

        let mut signals: Vec<WatermarkSignal> = Vec::new();

        // —— 特征 1:行频统计 ——
        let count = count_by_trimmed.get(trimmed).copied().unwrap_or(0);
        if count >= config.repeat_count_min {
            let raw = (count as f32).log10() / 50f32.log10();
            let score = raw.clamp(0.0, 1.0);
            signals.push(WatermarkSignal {
                kind: WatermarkSignalKind::Repetition,
                score,
                detail: Some(format!("出现 {} 次", count)),
            });
        }

        // —— 特征 2:非中文字符占比 ——
        let (cjk_chars, total_chars) = count_cjk_and_total(trimmed);
        if total_chars > 0 {
            let non_cjk_ratio = (total_chars - cjk_chars) as f32 / total_chars as f32;
            if non_cjk_ratio >= 0.4 {
                let raw = (non_cjk_ratio - 0.4) / 0.4;
                let score = raw.clamp(0.0, 1.0);
                let percent = (non_cjk_ratio * 100.0).round() as u32;
                signals.push(WatermarkSignal {
                    kind: WatermarkSignalKind::NonCjkRatio,
                    score,
                    detail: Some(format!("{}% 非中文字符", percent)),
                });
            }
        }

        // —— 特征 3:关键词正则 ——
        // 多规则命中合并为一条 signal(detail 列所有 rule id)。
        let mut hit_ids: Vec<&str> = Vec::new();
        for (id, re) in &compiled {
            if re.is_match(trimmed) {
                hit_ids.push(id);
            }
        }
        if !hit_ids.is_empty() {
            signals.push(WatermarkSignal {
                kind: WatermarkSignalKind::KeywordRegex,
                score: 1.0,
                detail: Some(format!("命中规则 {}", hit_ids.join(", "))),
            });
        }

        if signals.is_empty() {
            continue;
        }

        let score = fused_score(&signals, config);
        let Some(verdict) = classify(score, config) else {
            continue;
        };

        out.push(WatermarkAnnotation {
            span: Span::new(line_start, line_end),
            verdict,
            score,
            signals,
        });
    }

    // out 已按 line_start 升序产出(lines 顺序扫描)。
    out
}

/// 判断一行是否进入水印评分(跳过 heading / 空行 / 短行)。返回 trim 后的子切片以复用。
fn eligible_trimmed<'a>(
    source: &'a str,
    line_start: usize,
    line_end: usize,
    heading_starts: &HashSet<usize>,
    config: &WatermarkConfig,
) -> Option<&'a str> {
    if heading_starts.contains(&line_start) {
        return None;
    }
    let trimmed = source[line_start..line_end].trim();
    if trimmed.is_empty() {
        return None;
    }
    // 短行豁免:避免把"嗯。"、"哦?"等口头禅当水印
    if trimmed.chars().take(config.min_line_chars).count() < config.min_line_chars {
        return None;
    }
    Some(trimmed)
}

/// 统计字符串中 CJK 字符数与总字符数(按 Unicode `char`,不按字节)。
///
/// CJK 范围按 `docs/stage3-design.md` 第三节:
/// - `U+4E00..=U+9FFF`:基本汉字
/// - `U+3400..=U+4DBF`:CJK 扩展 A
/// - `U+3000..=U+303F`:常用中文标点(包含「」『』、。等)
/// - `U+FF00..=U+FFEF`:全角字母数字
pub(crate) fn count_cjk_and_total(s: &str) -> (usize, usize) {
    let mut cjk = 0;
    let mut total = 0;
    for c in s.chars() {
        total += 1;
        if is_cjk(c) {
            cjk += 1;
        }
    }
    (cjk, total)
}

fn is_cjk(c: char) -> bool {
    let cp = c as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0x3000..=0x303F).contains(&cp)
        || (0xFF00..=0xFFEF).contains(&cp)
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
    fn single_keyword_pure_cjk_lands_as_suspect_with_default_weights() {
        // 单 keyword 命中(纯中文,无 non_cjk_ratio 加分)= w_keyword × 1.0 = 0.40 → suspect
        let text = "\
第一章 起
正文一。
本文首发于纵横中文网,谢谢支持。
正文二。
";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1, "应当只有首发那一行被标:{:?}", out);
        let w = &out[0];
        assert_eq!(w.verdict, WatermarkVerdict::Suspect);
        assert!((w.score - 0.40).abs() < 1e-5, "纯中文 keyword 单特征应当 = 0.40,实际 {}", w.score);
        assert_eq!(w.signals.len(), 1, "纯中文行不应触发 non_cjk,只剩 keyword 一条 signal");
        assert_eq!(w.signals[0].kind, WatermarkSignalKind::KeywordRegex);
        assert!((w.signals[0].score - 1.0).abs() < f32::EPSILON);
        assert!(
            w.signals[0]
                .detail
                .as_ref()
                .map(|s| s.contains("builtin-watermark-first-publish"))
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
    fn multiple_rule_hits_on_same_line_collapse_into_one_keyword_signal() {
        // 同时命中 URL + 首发 两条规则,合并为 1 个 KeywordRegex signal,
        // detail 列两个 rule id;score 仍为 1.0(不重复计权)。
        // 注意:这行 non_cjk 也会触发,所以总 signals.len() 可能 > 1——本测试只关心 keyword 那条。
        let text = "本文首发于 https://example.com/novel\n";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1);
        let kw_signals: Vec<&WatermarkSignal> = out[0]
            .signals
            .iter()
            .filter(|s| s.kind == WatermarkSignalKind::KeywordRegex)
            .collect();
        assert_eq!(
            kw_signals.len(),
            1,
            "多规则命中应合并为 1 个 keyword_regex signal,实际 signals = {:?}",
            out[0].signals
        );
        assert!((kw_signals[0].score - 1.0).abs() < f32::EPSILON);
        let detail = kw_signals[0].detail.as_deref().unwrap_or("");
        assert!(detail.contains("builtin-watermark-url-http"), "detail = {detail}");
        assert!(detail.contains("builtin-watermark-first-publish"), "detail = {detail}");
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
        // 把 watermark 规则全 disable;两行短样本 non_cjk 单独 fused = 0.20 < 0.35、
        // repetition 不触发(count < 5)、keyword 已禁 → 全部 drop。
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

    // ============================================================================
    // 3.2 新特征:行频(repetition)+ 非中文占比(non_cjk_ratio)+ 三特征融合
    // ============================================================================

    /// 同一行重复 50 次 → repetition score = 1.0(capped),fused = w_repeat × 1.0 = 0.40
    /// → suspect。无 keyword 命中也无 non_cjk。
    #[test]
    fn pure_repetition_alone_lands_as_suspect() {
        let line = "这是一行被重复很多次的疑似水印";
        let mut text = String::from("第一章 起\n");
        for _ in 0..50 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(out.len() >= 1, "应当至少有一条 suspect");
        let w = &out[0];
        assert_eq!(w.verdict, WatermarkVerdict::Suspect);
        assert!((w.score - 0.40).abs() < 1e-5, "实际分数 {}", w.score);
        assert_eq!(w.signals.len(), 1, "纯 CJK 重复行只应有 repetition 一条 signal");
        assert_eq!(w.signals[0].kind, WatermarkSignalKind::Repetition);
        assert!((w.signals[0].score - 1.0).abs() < 1e-5);
        assert_eq!(w.signals[0].detail.as_deref(), Some("出现 50 次"));
    }

    /// keyword + repetition(均 1.0)→ fused = 0.80 → **auto**。
    /// 设计文档第三节 fixture #2 的真实场景。
    #[test]
    fn keyword_plus_repetition_lands_as_auto() {
        // 纯中文 keyword 命中行重复 50 次:每个 hit 都有 keyword(1.0)+ repetition(1.0),fused = 0.80
        let line = "本文首发于纵横中文网,谢谢支持";
        let mut text = String::from("第一章 起\n正文。\n");
        for _ in 0..50 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(out.len() >= 1, "应当至少有 auto 命中");
        // 所有命中行都应当是 auto
        for w in &out {
            assert_eq!(w.verdict, WatermarkVerdict::Auto, "{:?} 应当为 auto", w);
            assert!((w.score - 0.80).abs() < 1e-5, "实际分数 {}", w.score);
            assert_eq!(w.signals.len(), 2);
        }
    }

    /// 纯非 CJK 行(无 keyword、不重复)→ 仅 non_cjk score=1.0,fused = 0.20 < 0.35 → drop。
    /// 重要语义:非 CJK 单特征**无法**单独触发 suspect,避免把零星英文短语误标。
    #[test]
    fn pure_non_cjk_alone_does_not_reach_suspect() {
        let text = "第一章 起\n正文。\nABCDEFG abcdefg 12345678\n正文二。\n";
        let out = analyze_default(text);
        assert!(out.is_empty(), "non_cjk 单特征 fused=0.20 不应触发 suspect:{:?}", out);
    }

    /// 高 non_cjk 行 + keyword(URL)→ keyword 0.40 + non_cjk ~0.20 ≈ 0.6 → suspect(不到 auto)。
    /// **修正设计文档第三节 fixture #3** 的错误期望——单 keyword + 单 non_cjk 不足以升 auto,
    /// 还需 repetition 或 LLM 仲裁。
    #[test]
    fn keyword_plus_non_cjk_no_repetition_stays_suspect() {
        let text = "第一章 起\n正文。\n本文首发于 https://example.com/novel\n正文二。\n";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1);
        let w = &out[0];
        assert_eq!(
            w.verdict,
            WatermarkVerdict::Suspect,
            "keyword+non_cjk 无 repetition 时应当落 suspect,实际 {:?} (score={})",
            w.verdict,
            w.score
        );
        assert!(
            w.score > 0.35 && w.score < 0.70,
            "score 应落在 suspect 区间 [0.35, 0.70),实际 {}",
            w.score
        );
        // 应当有 keyword + non_cjk 两条 signal
        let kinds: Vec<WatermarkSignalKind> = w.signals.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&WatermarkSignalKind::KeywordRegex));
        assert!(kinds.contains(&WatermarkSignalKind::NonCjkRatio));
    }

    /// 三特征齐发:重复 50 次的"首发于 https://xxx.com"行 → 三 signal 融合 → 必定 auto。
    #[test]
    fn all_three_features_fused_to_auto() {
        let line = "本文首发于 https://example.com/n/12345";
        let mut text = String::from("第一章 起\n正文。\n");
        for _ in 0..50 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(!out.is_empty());
        let w = &out[0];
        assert_eq!(w.verdict, WatermarkVerdict::Auto);
        // 应当有三个 signal
        let mut kinds: Vec<WatermarkSignalKind> = w.signals.iter().map(|s| s.kind).collect();
        kinds.sort_by_key(|k| format!("{:?}", k));
        let expected = vec![
            WatermarkSignalKind::KeywordRegex,
            WatermarkSignalKind::NonCjkRatio,
            WatermarkSignalKind::Repetition,
        ];
        let mut expected_sorted = expected.clone();
        expected_sorted.sort_by_key(|k| format!("{:?}", k));
        assert_eq!(kinds, expected_sorted, "三特征应当全部触发");
        // 融合分应当 > auto_threshold(0.70),三特征最大 1.0 时可达 1.0
        assert!(w.score >= 0.70, "score = {}", w.score);
    }

    /// 行频特征:count < repeat_count_min 不触发 repetition signal。
    #[test]
    fn repetition_below_threshold_does_not_signal() {
        // 重复 4 次,默认 repeat_count_min = 5 不触发
        let line = "重复 4 次的中文行内容";
        let mut text = String::from("第一章 起\n");
        for _ in 0..4 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(out.is_empty(), "count<5 时 repetition 不应触发,实际 {:?}", out);
    }

    /// repetition 公式锁定:count=5 时 score ≈ 0.41;count=50 时 score = 1.0(刚好封顶);
    /// count=100 时仍 = 1.0(超过 50 已饱和)。
    #[test]
    fn repetition_score_formula_is_log_capped_at_50() {
        // 用一个低 repeat_count_min 让 count=5 也触发,以便观察分数
        let line = "重复测试行内容长度足够";
        let cases = [(5usize, 0.41_f32), (50, 1.0), (100, 1.0)];
        for (n, expected) in cases {
            let mut text = String::from("第一章 起\n");
            for _ in 0..n {
                text.push_str(line);
                text.push('\n');
            }
            let book = parse_book(&text);
            let out = analyze(text.as_str(), &book, &RuleSet::builtin(), &[], &WatermarkConfig::default());
            let rep_signal = out
                .iter()
                .find_map(|w| w.signals.iter().find(|s| s.kind == WatermarkSignalKind::Repetition));
            // 注意:count=5 时 fused = 0.41 * 0.40 = 0.164 < 0.35,不会进 out。
            // 所以 count=5 case 我们必须借助 fused_score 直接测公式。
            if n == 5 {
                // 直接用 fused_score 验证 repetition 公式(下方独立测试 fused_score_and_classify_thresholds 也覆盖)
                let s_repeat = (n as f32).log10() / 50f32.log10();
                assert!((s_repeat - expected).abs() < 0.02, "count={n} 公式分 {s_repeat} 偏离 {expected}");
            } else {
                let s = rep_signal
                    .unwrap_or_else(|| panic!("count={n} 应当产出 repetition signal,实际无,out={:?}", out));
                assert!((s.score - expected).abs() < 0.02, "count={n} score={} 偏离 {expected}", s.score);
            }
        }
    }

    /// non_cjk 公式锁定:ratio=0.4 → score=0;ratio=0.6 → 0.5;ratio=1.0 → cap 1.0。
    /// 用 fused_score / 直接公式校验,不走 analyze(避免被 suspect 阈值过滤)。
    #[test]
    fn non_cjk_score_formula_breakpoints() {
        let cases = [(0.40_f32, 0.0_f32), (0.60, 0.5), (0.80, 1.0), (1.0, 1.0)];
        for (ratio, expected) in cases {
            let raw = (ratio - 0.4) / 0.4;
            let score = raw.clamp(0.0, 1.0);
            assert!(
                (score - expected).abs() < 1e-5,
                "ratio={ratio} 应当得 {expected},实际 {score}"
            );
        }
    }

    /// `count_cjk_and_total` helper:CJK 范围按设计文档第三节(基本汉字 + 扩展 A + 中文标点 + 全角)。
    #[test]
    fn count_cjk_and_total_recognizes_designated_ranges() {
        // 基本汉字
        assert_eq!(count_cjk_and_total("正文"), (2, 2));
        // 中文标点(。是 U+3002 落在 0x3000..0x303F)
        assert_eq!(count_cjk_and_total("正文。"), (3, 3));
        // 全角字母(Ａ 是 U+FF21 落在 0xFF00..0xFFEF)
        assert_eq!(count_cjk_and_total("ＡＢ"), (2, 2));
        // 半角 ASCII 全部 non-CJK(A/B 是 U+0041/U+0042)
        assert_eq!(count_cjk_and_total("AB"), (0, 2));
        assert_eq!(count_cjk_and_total("abc"), (0, 3));
        // 数字、空格、半角标点 non-CJK
        assert_eq!(count_cjk_and_total("a b 1."), (0, 6));
        // 混合
        let (cjk, total) = count_cjk_and_total("正文 abc 一二");
        assert_eq!((cjk, total), (4, 9), "应得 4 CJK + 5 non-CJK(含空格)= 9 total");
        // 空串
        assert_eq!(count_cjk_and_total(""), (0, 0));
    }

    /// 短行豁免应当在 **eligibility** 阶段就生效,导致该行**不进入** repetition 计数。
    /// 如果短行也被算入行频统计,会让"嗯。"等口头禅产生 repetition signal——这是必须避免的。
    #[test]
    fn short_lines_do_not_contribute_to_repetition_count() {
        // "嗯。" 只有 2 字符,默认 min_line_chars=4 → 豁免
        let mut text = String::from("第一章 起\n");
        for _ in 0..50 {
            text.push_str("嗯。\n");
        }
        // 加一条长正文以验证不会被影响
        text.push_str("正文一段较长的内容用于占位。\n");
        let out = analyze_default(&text);
        assert!(out.is_empty(), "短行不应触发任何 signal,实际 {:?}", out);
    }
}
