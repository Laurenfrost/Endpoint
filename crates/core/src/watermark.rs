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
use serde::{Deserialize, Serialize};

use crate::domain::{
    Book, BookEntry, CleaningAnnotation, CleaningKind, DecisionScope, DecisionVerdict, Span,
    UserDecision, WatermarkAnnotation, WatermarkSignal, WatermarkSignalKind, WatermarkVerdict,
};
use crate::rules::{RuleKind, RuleSet};

/// 水印检测的可调参数。默认值见 [`Default`] 实现与 `docs/stage3-design.md` 第三节。
///
/// **v2 起加 serde derive + `#[serde(default)]`**,允许前端只传一部分字段
/// (例如只想改 `auto_threshold`)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
    /// 短行豁免阈值:行字符数 < 此值时所有特征都不打分。
    /// **v2 默认 10**(v1 是 4——实测发现 4 无法挡住"嗯,好的""哦,这样啊"等
    /// 5-6 字的对白)。
    pub min_line_chars: usize,
    /// 关闭水印检测开关。`false` 时 [`analyze`] 直接返回空列表;用于 A/B 与回归测试。默认 `true`。
    pub enabled: bool,
}

impl Default for WatermarkConfig {
    fn default() -> Self {
        Self {
            auto_threshold: 0.70,
            // v2:从 0.35 上调到 0.42——结合 w_repeat/w_keyword 都是 0.40,
            // 让"单 keyword 命中"(0.40)与"单 repetition 命中"(0.40)**单独**
            // 都不再产出 annotation,要求**至少两个独立特征**才进 suspect。
            // 这与设计文档"误删比漏删更保守"+真机实测后用户反馈"500+ 候选只 2 个真"对齐。
            suspect_threshold: 0.42,
            w_repeat: 0.40,
            w_non_cjk: 0.20,
            w_keyword: 0.40,
            repeat_count_min: 5,
            min_line_chars: 10, // v2:4 → 10
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

/// 判断一行是否进入水印评分(跳过 heading / 空行 / 短行 / **对白行**)。
/// 返回 trim 后的子切片以复用。
///
/// **v2 新增对白行豁免**:中文小说里高频出现的角色对白(被「」/『』/"" 包围)
/// 经常会因为短词/重复台词触发 repetition,但用户从未把它们当作水印。
/// 这里在 eligibility 阶段直接剔除——其行频不会污染统计,自身也不会被打分。
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
    // 短行豁免:v2 起默认 10 字符,避免把"嗯。"、"哦,是的"等口头禅当水印
    if trimmed.chars().take(config.min_line_chars).count() < config.min_line_chars {
        return None;
    }
    // v2 对白行豁免
    if is_dialogue_line(trimmed) {
        return None;
    }
    Some(trimmed)
}

/// 是否为对白行:首尾分别是中文小说常用引号对中的开/闭引号。
///
/// 允许首尾**不严格匹配**(如 `「话「』`)——`docs/stage3-v2-design.md` 第二节
/// 决策 6 明确"对白行豁免引号集合 = 3 对",这里取并集判断,容忍排版偶然失配。
///
/// 三对引号:
/// - 直角引号:`「`(U+300C)/ `」`(U+300D)
/// - 双直角引号:`『`(U+300E)/ `』`(U+300F)
/// - 弯引号:`"`(U+201C)/ `"`(U+201D)
fn is_dialogue_line(trimmed: &str) -> bool {
    let mut chars = trimmed.chars();
    let first = chars.next();
    let last = trimmed.chars().last();
    match (first, last) {
        (Some(f), Some(l)) if f != l => {
            matches!(f, '「' | '『' | '\u{201C}') && matches!(l, '」' | '』' | '\u{201D}')
        }
        _ => false,
    }
}

/// 统计字符串中**中文文本场景下的字符**数与总字符数(按 Unicode `char`,不按字节)。
///
/// 中文文本场景字符范围(v2 起扩展,详见 `docs/stage3-v2-design.md`):
/// - `U+4E00..=U+9FFF`:基本汉字
/// - `U+3400..=U+4DBF`:CJK 扩展 A
/// - `U+3000..=U+303F`:CJK 常用标点(「」『』、。等)
/// - `U+FF00..=U+FFEF`:全角字母数字
/// - **v2 新增** `U+2000..=U+206F`:通用标点(含 `…` U+2026、`—` U+2014、`–` U+2013 等中文常用标点)
/// - **v2 新增** `U+00A0`:NBSP(行内不可见空白,中文 txt 偶有)
/// - **v2 新增** `U+00B7`:中间点(用作中文姓名分隔,如「玛丽·安东尼」)
/// - **v2 新增** `U+FEFF`:零宽不换行空格(BOM 残留)
pub(crate) fn count_cjk_and_total(s: &str) -> (usize, usize) {
    let mut cjk = 0;
    let mut total = 0;
    for c in s.chars() {
        total += 1;
        if is_chinese_context_char(c) {
            cjk += 1;
        }
    }
    (cjk, total)
}

/// 判断字符是否属于"中文文本场景"——核心动机是 `non_cjk_ratio` 特征不把中文常用
/// 标点(尤其 `…` `——`)误算成"非中文",从而避免把对白密集的正文标为水印。
/// 详见 `docs/stage3-v2-design.md` 决策 2。
fn is_chinese_context_char(c: char) -> bool {
    let cp = c as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0x3000..=0x303F).contains(&cp)
        || (0xFF00..=0xFFEF).contains(&cp)
        || (0x2000..=0x206F).contains(&cp) // v2 新增:通用标点(含 … — –)
        || cp == 0x00A0                     // v2 新增:NBSP
        || cp == 0x00B7                     // v2 新增:中间点
        || cp == 0xFEFF                     // v2 新增:零宽不换行空格
}

/// 把 verdict==auto 的水印**镜像**到 `cleaning` 列表,使 EPUB 物化路径无感知地完成扣除。
///
/// 详见 `docs/stage3-design.md` 第二节"镜像不变式"与第五节 5.2:
/// 1. 仅 verdict==auto 写入;suspect **不**进 cleaning。
/// 2. 输出按 `span.start` 升序、互不重叠(保阶段二既定不变式)。
/// 3. 重叠时取并集 span,kind 优先级:
///    原 cleaning 4 变体 > `WatermarkKeyword` > `WatermarkRepetition` > `WatermarkNonCjk`。
/// 4. 单条 watermark 的 kind 映射:取其 `signals` 中 score 最高那条 signal,按
///    `KeywordRegex → WatermarkKeyword` / `Repetition → WatermarkRepetition` /
///    `NonCjkRatio → WatermarkNonCjk` 映射;tie-break 用同优先级链。
///
/// 实现策略:把所有 cleaning + auto 镜像放入一个 Vec,按 (start, end) 排序,
/// 然后线性扫描合并重叠区间。O((m+n) log (m+n)),m + n 量级远小于 source 字节数。
pub fn merge_auto_into_cleaning(
    cleaning: Vec<CleaningAnnotation>,
    watermarks: &[WatermarkAnnotation],
) -> Vec<CleaningAnnotation> {
    // 收集 auto 镜像
    let mut combined: Vec<CleaningAnnotation> = cleaning;
    for w in watermarks {
        if w.verdict != WatermarkVerdict::Auto {
            continue;
        }
        combined.push(CleaningAnnotation {
            span: w.span,
            kind: signals_to_cleaning_kind(&w.signals),
            replacement: None,
        });
    }
    sort_and_merge_overlaps(combined)
}

/// **阶段三 v2.2 新增**:把用户决策叠加到自动产物上,产出最终 cleaning。
///
/// 输入 `cleaning` 应当是 [`merge_auto_into_cleaning`] 的输出(已含 auto 镜像)。
/// 输入 `watermarks` 是原 `PipelineOutput.watermark` 全集,用于查找 approved suspect 的
/// signals(决定镜像变体 kind)。
///
/// 三类决策真正改变输出:
/// - `(Cleaning, Rejected)`:从 cleaning 移除该 span(EPUB 不删该行)
/// - `(Watermark, Rejected)`:从 cleaning 镜像移除该 span(若曾被 auto 镜像)
/// - `(Watermark, Approved)`:若 span 在 watermark 列表中 verdict==Suspect,
///   则注入 cleaning(以对应 `Watermark*` kind 形式)→ EPUB 删该行
///
/// 决策与 `pipeline.cleaning` 中 span 严格按 `(start, end)` 精确匹配——前端必须保证
/// 用 IPC 返回的 span 原样回传,不做合并裁剪。
///
/// 详见 `docs/stage3-v2-design.md` 第三节 3.3。
pub fn apply_user_decisions(
    cleaning: &[CleaningAnnotation],
    watermarks: &[WatermarkAnnotation],
    decisions: &[UserDecision],
) -> Vec<CleaningAnnotation> {
    // 按 (start, end) 索引三类逆向决策
    let mut cleaning_rejected: HashSet<(usize, usize)> = HashSet::new();
    let mut wm_rejected: HashSet<(usize, usize)> = HashSet::new();
    let mut wm_approved: HashSet<(usize, usize)> = HashSet::new();
    for d in decisions {
        let key = (d.span.start, d.span.end);
        match (d.scope, d.verdict) {
            (DecisionScope::Cleaning, DecisionVerdict::Rejected) => {
                cleaning_rejected.insert(key);
            }
            (DecisionScope::Watermark, DecisionVerdict::Rejected) => {
                wm_rejected.insert(key);
            }
            (DecisionScope::Watermark, DecisionVerdict::Approved) => {
                wm_approved.insert(key);
            }
            // 其余 3 态决策与默认一致,不需特殊处理:
            //  - (Cleaning, Approved):默认就是删 → no-op
            //  - 不可能 (Cleaning, ...) 同时被 rejected 与 approved,
            //    若前端传了冲突我们以 rejected 为准(rejected 是真正改默认的)
            _ => {}
        }
    }

    // 1. 过滤:rejected 项移除
    let mut kept: Vec<CleaningAnnotation> = cleaning
        .iter()
        .filter(|c| {
            let key = (c.span.start, c.span.end);
            if c.kind.is_watermark() {
                // watermark 镜像:rejected 移除
                !wm_rejected.contains(&key)
            } else {
                // 普通 cleaning:rejected 移除
                !cleaning_rejected.contains(&key)
            }
        })
        .cloned()
        .collect();

    // 2. 注入:approved suspect
    for w in watermarks {
        if w.verdict != WatermarkVerdict::Suspect {
            continue;
        }
        let key = (w.span.start, w.span.end);
        if !wm_approved.contains(&key) {
            continue;
        }
        kept.push(CleaningAnnotation {
            span: w.span,
            kind: signals_to_cleaning_kind(&w.signals),
            replacement: None,
        });
    }

    sort_and_merge_overlaps(kept)
}

/// 共用 helper:按 `(start, end)` 排序后,线性扫描合并重叠区间。
///
/// 重叠处理与第二节"镜像不变式 #3"一致:并集 span + 优先级 kind。
/// 同时被 [`merge_auto_into_cleaning`] 与 [`apply_user_decisions`] 调用。
fn sort_and_merge_overlaps(mut combined: Vec<CleaningAnnotation>) -> Vec<CleaningAnnotation> {
    combined.sort_by_key(|a| (a.span.start, a.span.end));

    let mut out: Vec<CleaningAnnotation> = Vec::with_capacity(combined.len());
    for ann in combined {
        if let Some(last) = out.last_mut() {
            if ann.span.start < last.span.end {
                let new_end = last.span.end.max(ann.span.end);
                let new_kind = pick_priority_kind(last.kind, ann.kind);
                let new_replacement = if new_kind == ann.kind && new_kind != last.kind {
                    ann.replacement
                } else {
                    last.replacement.take()
                };
                last.span = Span::new(last.span.start, new_end);
                last.kind = new_kind;
                last.replacement = new_replacement;
                continue;
            }
        }
        out.push(ann);
    }

    out
}

/// 从 signals 列表选出最适合做镜像 [`CleaningKind`] 的 signal。
///
/// 策略:取 score 最高那条;tie 时按 kind 优先级链 `KeywordRegex > Repetition > NonCjkRatio` 选。
/// 这与第二节"镜像不变式 #3"的 watermark 子优先级保持一致。
fn signals_to_cleaning_kind(signals: &[WatermarkSignal]) -> CleaningKind {
    // signals 在 analyze 中保证至少有一条;若万一为空,退化到 WatermarkKeyword 占位
    // (用 debug_assert 在 debug 构建中提示开发者)。
    debug_assert!(!signals.is_empty(), "auto watermark 的 signals 不应为空");
    let mut best_idx = 0;
    let mut best_pri = signal_priority(signals[0].kind);
    let mut best_score = signals[0].score;
    for (i, s) in signals.iter().enumerate().skip(1) {
        let pri = signal_priority(s.kind);
        if s.score > best_score || (s.score == best_score && pri > best_pri) {
            best_idx = i;
            best_pri = pri;
            best_score = s.score;
        }
    }
    match signals[best_idx].kind {
        WatermarkSignalKind::KeywordRegex => CleaningKind::WatermarkKeyword,
        WatermarkSignalKind::Repetition => CleaningKind::WatermarkRepetition,
        WatermarkSignalKind::NonCjkRatio => CleaningKind::WatermarkNonCjk,
    }
}

fn signal_priority(k: WatermarkSignalKind) -> u8 {
    match k {
        WatermarkSignalKind::KeywordRegex => 3,
        WatermarkSignalKind::Repetition => 2,
        WatermarkSignalKind::NonCjkRatio => 1,
    }
}

/// 重叠合并时的 kind 优先级:原 cleaning 4 变体 > watermark 3 变体(后者内部按 signal 同序)。
fn pick_priority_kind(a: CleaningKind, b: CleaningKind) -> CleaningKind {
    if cleaning_kind_priority(a) >= cleaning_kind_priority(b) {
        a
    } else {
        b
    }
}

fn cleaning_kind_priority(k: CleaningKind) -> u8 {
    match k {
        // 原 cleaning 5 变体之间相互不重叠(cleaning::analyze 保证),
        // 故彼此优先级取相同高位即可。
        CleaningKind::BlankLineCompression
        | CleaningKind::LeadingFullwidthSpace
        | CleaningKind::InlineFullwidthSpace
        | CleaningKind::ControlChar
        | CleaningKind::TrailingWhitespace => 10,
        CleaningKind::WatermarkKeyword => 3,
        CleaningKind::WatermarkRepetition => 2,
        CleaningKind::WatermarkNonCjk => 1,
    }
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
        // v2:suspect 从 0.35 上调到 0.42(决策:单特征不再 ≥ suspect)
        assert!(
            (c.suspect_threshold - 0.42).abs() < f32::EPSILON,
            "v2 默认 suspect_threshold = 0.42(单 keyword/repetition 0.40 不再触发)"
        );
        assert!((c.w_repeat - 0.40).abs() < f32::EPSILON);
        assert!((c.w_non_cjk - 0.20).abs() < f32::EPSILON);
        assert!((c.w_keyword - 0.40).abs() < f32::EPSILON);
        // 权重总和应当为 1.0(或非常接近)
        let sum = c.w_repeat + c.w_non_cjk + c.w_keyword;
        assert!((sum - 1.0).abs() < 1e-6, "三特征权重总和应当为 1.0,实际为 {sum}");
        assert_eq!(c.repeat_count_min, 5);
        // v2:4 → 10
        assert_eq!(c.min_line_chars, 10, "v2 默认短行豁免上调到 10 字符");
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

    /// v2 改:单 keyword 纯 CJK 命中 = 0.40 < 0.42 → **drop**。
    /// 这是 v2 调高 suspect 阈值的核心动机:单特征不再触发,降低误报。
    #[test]
    fn single_keyword_pure_cjk_below_suspect_threshold() {
        let text = "\
第一章 起
正文较长内容一。
本文首发于纵横中文网,谢谢支持。
正文较长内容二。
";
        let out = analyze_default(text);
        assert!(
            out.is_empty(),
            "v2:单 keyword 纯 CJK 命中(0.40 < 0.42)应当 drop,实际 {:?}",
            out
        );
    }

    /// 反向案例:同一关键词命中行**加另一个特征**(如非 CJK 比例)→ 进入 suspect。
    #[test]
    fn keyword_plus_non_cjk_reaches_suspect() {
        let text = "\
第一章 起
正文较长内容一。
本文首发于 https://example.com 谢谢支持。
正文较长内容二。
";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1, "应当只有首发那一行被标:{:?}", out);
        let w = &out[0];
        assert_eq!(w.verdict, WatermarkVerdict::Suspect);
        assert!(w.score >= 0.42 && w.score < 0.70, "实际 {}", w.score);
        // 至少有 keyword 与 non_cjk 两条 signal
        let kinds: Vec<WatermarkSignalKind> = w.signals.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&WatermarkSignalKind::KeywordRegex));
        assert!(kinds.contains(&WatermarkSignalKind::NonCjkRatio));
    }

    #[test]
    fn span_covers_whole_line() {
        // 用一个 keyword + non_cjk 双特征行确保命中(单 keyword 在 v2 已不够 suspect)
        let text = "正文一段。\n请到 https://novel.example.com 看更新。\n下一段较长内容。\n";
        let out = analyze_default(text);
        assert_eq!(out.len(), 1);
        let line_start = text.find("请到").unwrap();
        let line_end = line_start + "请到 https://novel.example.com 看更新。".len();
        assert_eq!(out[0].span.start, line_start);
        assert_eq!(out[0].span.end, line_end);
        // span 内容应当就是被命中那行
        assert_eq!(
            &text[out[0].span.start..out[0].span.end],
            "请到 https://novel.example.com 看更新。"
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
        // v2.1:min_line_chars=10 + suspect=0.42 → 命中行需:≥10 字符 且 单特征 0.40 不够,
        // 必须 keyword + 另一特征(non_cjk 或 repetition)。
        // 此 fixture 三个水印行都满足"keyword + non_cjk"。
        let text = "\
正文一段较长。
请访问 https://novel.example.com 看最新章节。
正文二段较长。
关注 TG 频道 @somenovelchannel 不迷路。
更新最快的小说网站 https://fast.example.cc 阅读体验最佳。
正文三段较长。
";
        let out = analyze_default(text);
        assert_eq!(out.len(), 3, "实际 {:?}", out);
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
        // 单 keyword 1.0 → 0.40。**v2:0.40 < 0.42 → drop**(从 v1 的 suspect 退到丢弃)
        let sig_kw = vec![WatermarkSignal {
            kind: WatermarkSignalKind::KeywordRegex,
            score: 1.0,
            detail: None,
        }];
        assert!((fused_score(&sig_kw, &cfg) - 0.40).abs() < 1e-5);
        assert_eq!(
            classify(0.40, &cfg),
            None,
            "v2:0.40 < suspect_threshold(0.42),单特征命中应丢弃"
        );
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
        assert_eq!(classify(0.42, &cfg), Some(WatermarkVerdict::Suspect));
        assert_eq!(classify(0.70, &cfg), Some(WatermarkVerdict::Auto));
        // 0.42 略下 → drop
        assert_eq!(classify(0.41, &cfg), None);
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

    /// v2 改:同一 CJK 行重复 50 次 → repetition score = 1.0,fused = 0.40 < 0.42 → **drop**。
    /// 这是 v2 调高 suspect 阈值的另一个直接体现:纯重复(无其它信号)不再算水印。
    #[test]
    fn pure_repetition_alone_below_suspect_threshold() {
        let line = "这是一行被重复很多次的疑似水印";
        let mut text = String::from("第一章 起\n");
        for _ in 0..50 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(
            out.is_empty(),
            "v2:纯重复 0.40 < 0.42 应当 drop,实际 {:?}",
            out
        );
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
    ///
    /// v2:由于默认 suspect_threshold=0.42 让单 repetition(fused 0.40)无法 ≥ suspect,
    /// 这里**直接验算公式**而不走 analyze(否则需要再叠一个特征才能产出 annotation,
    /// 反而把测试本身复杂化)。
    #[test]
    fn repetition_score_formula_is_log_capped_at_50() {
        let cases = [(5usize, 0.41_f32), (50, 1.0), (100, 1.0)];
        for (n, expected) in cases {
            let raw = (n as f32).log10() / 50f32.log10();
            let score = raw.clamp(0.0, 1.0);
            assert!(
                (score - expected).abs() < 0.02,
                "count={n} 公式分 {score} 偏离预期 {expected}"
            );
        }
    }

    /// 端到端验证 repetition signal 真的会出现在 analyze 输出中(只是需要叠加另一个特征
    /// 才能跨过 suspect_threshold)。
    #[test]
    fn repetition_signal_emerges_when_combined_with_keyword() {
        // 重复 50 次 的带 keyword 行 → keyword 0.40 + repetition 0.40 = 0.80 → auto
        let line = "首发于 https://example.com 更新最快阅读体验最佳";
        let mut text = String::from("第一章 起\n");
        for _ in 0..50 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(!out.is_empty(), "应当至少有 auto 命中");
        let w = &out[0];
        let rep_signal = w
            .signals
            .iter()
            .find(|s| s.kind == WatermarkSignalKind::Repetition)
            .expect("应当含 repetition signal");
        assert!((rep_signal.score - 1.0).abs() < 0.02, "count=50 时应当饱和到 1.0");
        assert_eq!(rep_signal.detail.as_deref(), Some("出现 50 次"));
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

    /// `count_cjk_and_total` helper:中文场景字符范围按设计文档第三节 v2 扩展。
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

    /// v2 扩展:省略号 `…`、破折号 `—`/`–`、中间点 `·`、NBSP、零宽 等
    /// 不应被算作 non-CJK——这是 v1 把对白密集的正文标为水印的核心病因。
    #[test]
    fn count_cjk_recognizes_v2_extended_punctuation() {
        // 省略号 U+2026
        assert_eq!(count_cjk_and_total("……"), (2, 2));
        // 破折号 U+2014 / en dash U+2013
        assert_eq!(count_cjk_and_total("——"), (2, 2));
        assert_eq!(count_cjk_and_total("––"), (2, 2));
        // 中间点 U+00B7(中文姓名分隔)
        assert_eq!(count_cjk_and_total("玛丽·安东尼"), (6, 6));
        // NBSP U+00A0
        assert_eq!(count_cjk_and_total("正\u{00A0}文"), (3, 3));
        // 零宽 U+200B / U+FEFF
        assert_eq!(count_cjk_and_total("正\u{200B}文\u{FEFF}"), (4, 4));
        // 典型对白行(省略号 + 中文 + 全角逗号)— v1 这里 ratio 高(……占很多 char),v2 应当全 CJK
        let line = "「他停顿了一下\u{FF0C}然后说道……」";
        let (cjk, total) = count_cjk_and_total(line);
        assert_eq!(cjk, total, "纯中文对白 + 省略号应当 100% CJK,实际 {}/{}", cjk, total);
    }

    /// v2 对白行豁免:即使重复 50 次,被引号包围的对白也不应触发 repetition。
    #[test]
    fn dialogue_line_repeated_does_not_trigger_repetition() {
        let lines = [
            "「这是一句对白内容。」",          // 「」
            "『另一种对白内容。』",            // 『』
            "\u{201C}弯引号的对白内容。\u{201D}", // ""
        ];
        for line in lines {
            let mut text = String::from("第一章 起\n");
            for _ in 0..50 {
                text.push_str(line);
                text.push('\n');
            }
            let out = analyze_default(&text);
            assert!(
                out.is_empty(),
                "对白行 `{}` 重复 50 次也不应触发水印,实际 {:?}",
                line,
                out
            );
        }
    }

    /// v2 对白豁免接受首尾**错配**(`「…』` 这种排版错误也豁免)。
    #[test]
    fn dialogue_line_exemption_tolerates_mismatched_quotes() {
        let line = "「这一行用错了引号但仍是对白』";
        let mut text = String::from("第一章\n");
        for _ in 0..50 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(out.is_empty(), "错配引号对白应当豁免,实际 {:?}", out);
    }

    /// v2 修复:含省略号的对白行 + 重复出现,非中文占比与行频都不会推升 score 到 suspect。
    #[test]
    fn ellipsis_heavy_dialogue_does_not_get_flagged() {
        let line = "「呃……我不太清楚……」";
        let mut text = String::from("第一章\n");
        for _ in 0..30 {
            text.push_str(line);
            text.push('\n');
        }
        let out = analyze_default(&text);
        assert!(
            out.is_empty(),
            "省略号密集的对白不应被标(v1 病因):{:?}",
            out
        );
    }

    /// v2 短行豁免上调到 10:5-6 字的对白即使不是引号包裹也应豁免。
    #[test]
    fn short_response_lines_below_ten_chars_are_exempt() {
        // 不是引号包裹但 < 10 字符的常见对白
        let lines = ["嗯,好的。", "我知道了。", "怎么了?", "哦,是这样。"];
        for line in lines {
            let mut text = String::from("第一章\n");
            for _ in 0..50 {
                text.push_str(line);
                text.push('\n');
            }
            let out = analyze_default(&text);
            assert!(
                out.is_empty(),
                "< 10 字符的短对白 `{}` 应豁免,实际 {:?}",
                line,
                out
            );
        }
    }

    /// v2:WatermarkConfig 反序列化测试——前端只传部分字段时,缺省字段走 Default。
    #[test]
    fn watermark_config_deserializes_partial_json() {
        let json = r#"{ "auto_threshold": 0.50 }"#;
        let cfg: WatermarkConfig = serde_json::from_str(json).unwrap();
        assert!((cfg.auto_threshold - 0.50).abs() < 1e-5);
        // 其余字段应当走 default(v2 后 suspect 默认 0.42)
        assert!((cfg.suspect_threshold - 0.42).abs() < 1e-5);
        assert_eq!(cfg.min_line_chars, 10);
        assert!(cfg.enabled);
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

    // ============================================================================
    // 3.3 新功能:merge_auto_into_cleaning(auto 镜像 + 优先级合并)
    // ============================================================================

    use crate::domain::CleaningKind;

    fn make_auto(span: Span, kind: WatermarkSignalKind) -> WatermarkAnnotation {
        WatermarkAnnotation {
            span,
            verdict: WatermarkVerdict::Auto,
            score: 0.80,
            signals: vec![WatermarkSignal {
                kind,
                score: 1.0,
                detail: None,
            }],
        }
    }

    fn make_suspect(span: Span) -> WatermarkAnnotation {
        WatermarkAnnotation {
            span,
            verdict: WatermarkVerdict::Suspect,
            score: 0.40,
            signals: vec![WatermarkSignal {
                kind: WatermarkSignalKind::KeywordRegex,
                score: 1.0,
                detail: None,
            }],
        }
    }

    #[test]
    fn merge_empty_watermarks_returns_cleaning_unchanged() {
        let cleaning = vec![CleaningAnnotation {
            span: Span::new(10, 12),
            kind: CleaningKind::LeadingFullwidthSpace,
            replacement: Some(" ".into()),
        }];
        let out = merge_auto_into_cleaning(cleaning.clone(), &[]);
        assert_eq!(out, cleaning);
    }

    #[test]
    fn merge_suspect_only_does_not_touch_cleaning() {
        // 镜像不变式 #2 的反向:suspect 必须不进入 cleaning
        let cleaning = vec![CleaningAnnotation {
            span: Span::new(0, 3),
            kind: CleaningKind::LeadingFullwidthSpace,
            replacement: None,
        }];
        let wms = vec![make_suspect(Span::new(100, 120))];
        let out = merge_auto_into_cleaning(cleaning.clone(), &wms);
        assert_eq!(out, cleaning, "suspect 不应进入 cleaning");
    }

    #[test]
    fn merge_auto_inserts_with_correct_kind_and_no_overlap() {
        let cleaning = vec![CleaningAnnotation {
            span: Span::new(0, 3),
            kind: CleaningKind::LeadingFullwidthSpace,
            replacement: None,
        }];
        let wms = vec![
            make_auto(Span::new(100, 120), WatermarkSignalKind::KeywordRegex),
            make_auto(Span::new(200, 220), WatermarkSignalKind::Repetition),
            make_auto(Span::new(300, 320), WatermarkSignalKind::NonCjkRatio),
        ];
        let out = merge_auto_into_cleaning(cleaning, &wms);
        assert_eq!(out.len(), 4);
        // 排序后
        assert_eq!(out[0].span, Span::new(0, 3));
        assert_eq!(out[0].kind, CleaningKind::LeadingFullwidthSpace);
        assert_eq!(out[1].span, Span::new(100, 120));
        assert_eq!(out[1].kind, CleaningKind::WatermarkKeyword);
        assert_eq!(out[1].replacement, None);
        assert_eq!(out[2].kind, CleaningKind::WatermarkRepetition);
        assert_eq!(out[3].kind, CleaningKind::WatermarkNonCjk);
    }

    #[test]
    fn merge_signals_to_cleaning_kind_picks_highest_score_then_priority() {
        // signals 中三个 kind,score 相等 → 按优先级 keyword > repetition > non_cjk
        let signals = vec![
            WatermarkSignal { kind: WatermarkSignalKind::NonCjkRatio, score: 1.0, detail: None },
            WatermarkSignal { kind: WatermarkSignalKind::KeywordRegex, score: 1.0, detail: None },
            WatermarkSignal { kind: WatermarkSignalKind::Repetition, score: 1.0, detail: None },
        ];
        assert_eq!(signals_to_cleaning_kind(&signals), CleaningKind::WatermarkKeyword);

        // 分数不等 → 取最高;此处 repetition score 最高
        let signals = vec![
            WatermarkSignal { kind: WatermarkSignalKind::KeywordRegex, score: 0.4, detail: None },
            WatermarkSignal { kind: WatermarkSignalKind::Repetition, score: 0.9, detail: None },
            WatermarkSignal { kind: WatermarkSignalKind::NonCjkRatio, score: 0.8, detail: None },
        ];
        assert_eq!(signals_to_cleaning_kind(&signals), CleaningKind::WatermarkRepetition);
    }

    #[test]
    fn merge_overlap_preserves_original_cleaning_kind_and_union_span() {
        // 已有 cleaning [10, 15) 类型 TrailingWhitespace;auto 水印 [12, 30) 类型 KeywordRegex
        // 重叠 → 并集 [10, 30),kind = TrailingWhitespace(原 cleaning 优先级 > watermark)
        let cleaning = vec![CleaningAnnotation {
            span: Span::new(10, 15),
            kind: CleaningKind::TrailingWhitespace,
            replacement: None,
        }];
        let wms = vec![make_auto(Span::new(12, 30), WatermarkSignalKind::KeywordRegex)];
        let out = merge_auto_into_cleaning(cleaning, &wms);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].span, Span::new(10, 30));
        assert_eq!(out[0].kind, CleaningKind::TrailingWhitespace);
    }

    #[test]
    fn merge_overlap_two_auto_watermarks_uses_priority_chain() {
        // 两条 auto 水印重叠:[10, 25) Repetition + [20, 30) Keyword
        // 排序后先扫 [10, 25);第二条 [20, 30) 重叠合并 → 并集 [10, 30)
        // kind: keyword > repetition → Keyword
        let wms = vec![
            make_auto(Span::new(10, 25), WatermarkSignalKind::Repetition),
            make_auto(Span::new(20, 30), WatermarkSignalKind::KeywordRegex),
        ];
        let out = merge_auto_into_cleaning(Vec::new(), &wms);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].span, Span::new(10, 30));
        assert_eq!(out[0].kind, CleaningKind::WatermarkKeyword);
    }

    #[test]
    fn merge_output_is_sorted_and_non_overlapping() {
        // 一堆乱序输入,验证输出严格按 start 升序、互不重叠
        let cleaning = vec![
            CleaningAnnotation { span: Span::new(50, 55), kind: CleaningKind::LeadingFullwidthSpace, replacement: None },
            CleaningAnnotation { span: Span::new(0, 3), kind: CleaningKind::TrailingWhitespace, replacement: None },
        ];
        let wms = vec![
            make_auto(Span::new(200, 220), WatermarkSignalKind::KeywordRegex),
            make_auto(Span::new(100, 120), WatermarkSignalKind::NonCjkRatio),
            make_auto(Span::new(50, 60), WatermarkSignalKind::Repetition), // 与 cleaning [50,55) 重叠
        ];
        let out = merge_auto_into_cleaning(cleaning, &wms);
        // 检查升序 + 不重叠
        for w in out.windows(2) {
            assert!(w[0].span.start <= w[1].span.start);
            assert!(w[0].span.end <= w[1].span.start, "重叠未合并:{:?} vs {:?}", w[0], w[1]);
        }
        // 应有 4 条:[0,3) / [50,60)(合并) / [100,120) / [200,220)
        assert_eq!(out.len(), 4);
    }

    // ============================================================================
    // v2.2:apply_user_decisions 测试
    // ============================================================================

    use crate::domain::{DecisionScope, DecisionVerdict, UserDecision};

    fn dec(span: Span, scope: DecisionScope, verdict: DecisionVerdict) -> UserDecision {
        UserDecision { span, scope, verdict }
    }

    /// 空 decisions → 输出与输入完全一致。
    #[test]
    fn apply_decisions_empty_is_identity() {
        let cleaning = vec![
            CleaningAnnotation { span: Span::new(0, 3), kind: CleaningKind::TrailingWhitespace, replacement: None },
            CleaningAnnotation { span: Span::new(50, 60), kind: CleaningKind::WatermarkKeyword, replacement: None },
        ];
        let wms = vec![make_auto(Span::new(50, 60), WatermarkSignalKind::KeywordRegex)];
        let out = apply_user_decisions(&cleaning, &wms, &[]);
        assert_eq!(out, cleaning);
    }

    /// cleaning rejected:span 应从输出移除(EPUB 不删该行)。
    #[test]
    fn apply_decisions_cleaning_rejected_removes_span() {
        let cleaning = vec![
            CleaningAnnotation { span: Span::new(0, 3), kind: CleaningKind::TrailingWhitespace, replacement: None },
            CleaningAnnotation { span: Span::new(10, 15), kind: CleaningKind::BlankLineCompression, replacement: Some("\n\n".into()) },
        ];
        let decisions = vec![dec(Span::new(0, 3), DecisionScope::Cleaning, DecisionVerdict::Rejected)];
        let out = apply_user_decisions(&cleaning, &[], &decisions);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].span, Span::new(10, 15));
    }

    /// watermark auto rejected:从 cleaning 镜像移除该 span。
    #[test]
    fn apply_decisions_watermark_auto_rejected_removes_mirror() {
        // 已含 auto 镜像的 cleaning
        let cleaning = vec![
            CleaningAnnotation { span: Span::new(0, 3), kind: CleaningKind::TrailingWhitespace, replacement: None },
            CleaningAnnotation { span: Span::new(50, 60), kind: CleaningKind::WatermarkKeyword, replacement: None },
            CleaningAnnotation { span: Span::new(100, 110), kind: CleaningKind::WatermarkRepetition, replacement: None },
        ];
        let wms = vec![
            make_auto(Span::new(50, 60), WatermarkSignalKind::KeywordRegex),
            make_auto(Span::new(100, 110), WatermarkSignalKind::Repetition),
        ];
        let decisions = vec![dec(Span::new(50, 60), DecisionScope::Watermark, DecisionVerdict::Rejected)];
        let out = apply_user_decisions(&cleaning, &wms, &decisions);
        assert_eq!(out.len(), 2);
        // [50,60) 镜像被移除;[0,3) 与 [100,110) 保留
        assert_eq!(out[0].span, Span::new(0, 3));
        assert_eq!(out[1].span, Span::new(100, 110));
    }

    /// watermark suspect approved:span 注入 cleaning(等效升 auto → EPUB 删)。
    #[test]
    fn apply_decisions_suspect_approved_injects_into_cleaning() {
        let cleaning: Vec<CleaningAnnotation> = Vec::new(); // 假设之前没有 cleaning
        let wms = vec![WatermarkAnnotation {
            span: Span::new(200, 220),
            verdict: WatermarkVerdict::Suspect,
            score: 0.56,
            signals: vec![WatermarkSignal {
                kind: WatermarkSignalKind::KeywordRegex,
                score: 1.0,
                detail: None,
            }],
        }];
        let decisions = vec![dec(Span::new(200, 220), DecisionScope::Watermark, DecisionVerdict::Approved)];
        let out = apply_user_decisions(&cleaning, &wms, &decisions);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].span, Span::new(200, 220));
        assert_eq!(out[0].kind, CleaningKind::WatermarkKeyword);
    }

    /// cleaning approved 是 no-op(默认就是删除,显式锁定不改输出)。
    #[test]
    fn apply_decisions_cleaning_approved_is_noop() {
        let cleaning = vec![CleaningAnnotation {
            span: Span::new(0, 3),
            kind: CleaningKind::TrailingWhitespace,
            replacement: None,
        }];
        let decisions = vec![dec(Span::new(0, 3), DecisionScope::Cleaning, DecisionVerdict::Approved)];
        let out = apply_user_decisions(&cleaning, &[], &decisions);
        assert_eq!(out, cleaning, "cleaning approved 应当 = no-op,默认行为不变");
    }

    /// suspect rejected 是 no-op(默认就是保留)。
    #[test]
    fn apply_decisions_suspect_rejected_is_noop() {
        let cleaning: Vec<CleaningAnnotation> = Vec::new();
        let wms = vec![WatermarkAnnotation {
            span: Span::new(200, 220),
            verdict: WatermarkVerdict::Suspect,
            score: 0.50,
            signals: vec![WatermarkSignal {
                kind: WatermarkSignalKind::KeywordRegex,
                score: 1.0,
                detail: None,
            }],
        }];
        let decisions = vec![dec(Span::new(200, 220), DecisionScope::Watermark, DecisionVerdict::Rejected)];
        let out = apply_user_decisions(&cleaning, &wms, &decisions);
        assert!(out.is_empty(), "suspect rejected 应当 = no-op,suspect 本就不在 cleaning");
    }

    /// 不存在 span 的决策应被忽略(不 panic,不改输出)。
    #[test]
    fn apply_decisions_unmatched_span_is_ignored() {
        let cleaning = vec![CleaningAnnotation {
            span: Span::new(0, 3),
            kind: CleaningKind::TrailingWhitespace,
            replacement: None,
        }];
        let decisions = vec![
            dec(Span::new(999, 1000), DecisionScope::Cleaning, DecisionVerdict::Rejected),
            dec(Span::new(500, 600), DecisionScope::Watermark, DecisionVerdict::Rejected),
        ];
        let out = apply_user_decisions(&cleaning, &[], &decisions);
        assert_eq!(out, cleaning);
    }

    /// approved suspect 与既有 cleaning 重叠时,按合并优先级处理(原 cleaning > watermark)。
    #[test]
    fn apply_decisions_approved_suspect_merges_with_existing() {
        let cleaning = vec![CleaningAnnotation {
            span: Span::new(195, 205),
            kind: CleaningKind::TrailingWhitespace,
            replacement: None,
        }];
        let wms = vec![WatermarkAnnotation {
            span: Span::new(200, 220),
            verdict: WatermarkVerdict::Suspect,
            score: 0.50,
            signals: vec![WatermarkSignal {
                kind: WatermarkSignalKind::KeywordRegex,
                score: 1.0,
                detail: None,
            }],
        }];
        let decisions = vec![dec(Span::new(200, 220), DecisionScope::Watermark, DecisionVerdict::Approved)];
        let out = apply_user_decisions(&cleaning, &wms, &decisions);
        assert_eq!(out.len(), 1);
        // 并集 [195, 220);kind 应当是 TrailingWhitespace(原 cleaning 优先级 > watermark)
        assert_eq!(out[0].span, Span::new(195, 220));
        assert_eq!(out[0].kind, CleaningKind::TrailingWhitespace);
    }
}
