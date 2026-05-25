//! 章节解析:核心库的心脏。
//!
//! 阶段一对应 CLAUDE.md 第六节的前两阶段:
//! 1. **候选行扫描**:用 [`rules::RuleSet`] 中的章节/卷规则逐行匹配,叠加结构约束。
//! 2. **层级归属**:把候选组织成卷章两级,每章归属前面最近的卷;卷之前的章(楔子/序章)挂书根。
//! 3. **超长区间检测**(阶段四 4.4):按中位数启发式找遗漏标题,本地、无 LLM。
//!
//! 整本未识别任何标题时沿用阶段零的「单章 Fallback」兜底。
//!
//! # 阶段三签名拆分(`docs/stage3-design.md` 第五节 5.2.a 决议)
//!
//! [`parse`] 只识别 chapter/volume 边界,产出的 [`Chapter::paragraphs`] 一律为空。
//! 段落物化由独立函数 [`materialize_paragraphs`] 完成。
//! 这样做的好处是:[`crate::watermark::analyze`] 在 [`parse`] 之后跑,
//! 把 auto 水印镜像写入 cleaning 后,再调 [`materialize_paragraphs`] 时就能
//! 一次性扣除"格式清洗 + 自动水印"两类删除,EPUB 输出路径单一不分支。
//!
//! # 关于 span 与坐标系
//!
//! 所有产出的 [`Chapter`] / [`Volume`] 的 `heading_span` / `body_span` 都指向**调用方传入的
//! `source` 字符串**(即 decoded source),单位是 UTF-8 字节偏移。详见 [`crate::domain`]
//! 模块文档「富标注输出契约」。

use regex::Regex;
use thiserror::Error;

use crate::cleaning;
use crate::domain::{
    Book, BookEntry, Chapter, ChapterOrigin, CleaningAnnotation, Metadata, Paragraph, Span, Volume,
};
use crate::rules::{RuleKind, RuleSet, RulesError};

#[derive(Debug, Error)]
pub enum ChapterError {
    #[error(transparent)]
    Rules(#[from] RulesError),
}

/// 候选行的等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadingLevel {
    Volume,
    Chapter,
}

/// 一条候选标题行的内部表示。
struct HeadingCandidate {
    level: HeadingLevel,
    heading_span: Span, // 指向 source 中的标题行(不含 \n)
    title: String,
    rule_id: String,
}

/// 解析章节边界。**不**填 [`Chapter::paragraphs`]——段落物化由 [`materialize_paragraphs`]
/// 完成(阶段三签名拆分,详见模块文档与 `docs/stage3-design.md` 第五节)。
///
/// - `source`:解码后的源文本——所有 span 的坐标参照系。
/// - `rules`:规则库。仅消费 `Chapter` / `Volume` 两类规则。
pub fn parse(source: &str, rules: &RuleSet, metadata: Metadata) -> Result<Book, ChapterError> {
    // 预编译规则集,按优先级降序。
    let chapter_rules = compile_rules(rules, RuleKind::Chapter)?;
    let volume_rules = compile_rules(rules, RuleKind::Volume)?;

    // 第一阶段:候选行扫描。
    // TODO(cancel): 接 `ConvertOptions.cancel_token` 后,每 N 行检查一次取消标志,
    // 提前返回 `ChapterError::Cancelled`(待添加)。阶段二只预留接口,不实装。
    let mut candidates: Vec<HeadingCandidate> = Vec::new();
    for (line_start, line_end) in iter_lines(source) {
        let line = &source[line_start..line_end];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !passes_structural_constraints(trimmed) {
            continue;
        }

        // 卷规则优先于章规则尝试(卷标题更具体,典型如「第一卷 风起」中的「卷」位置
        // 不能被章规则误识别——实际上章规则用「章/回/话/节」,不会撞,但保证语义清楚)。
        if let Some(c) =
            try_match(line_start, line_end, trimmed, &volume_rules, HeadingLevel::Volume)
        {
            candidates.push(c);
            continue;
        }
        if let Some(c) = try_match(
            line_start,
            line_end,
            trimmed,
            &chapter_rules,
            HeadingLevel::Chapter,
        ) {
            candidates.push(c);
        }
    }

    // 兜底:整本未识别出任何候选。
    if candidates.is_empty() {
        return Ok(fallback_book(source, metadata));
    }

    // 第二阶段:卷章层级组织。
    let mut entries: Vec<BookEntry> = Vec::new();
    let mut current_volume: Option<Volume> = None;

    // 首个候选之前的实质内容 → 「楔子」单章挂书根(Fallback)。
    let first_start = candidates[0].heading_span.start;
    if first_start > 0 {
        let preface_span = Span::new(0, first_start);
        if !source[preface_span.start..preface_span.end]
            .trim()
            .is_empty()
        {
            entries.push(BookEntry::Chapter(Chapter {
                title: "楔子".into(),
                paragraphs: Vec::new(), // 由 materialize_paragraphs 填
                heading_span: Span::new(0, 0), // 无显式标题行
                body_span: preface_span,
                origin: ChapterOrigin::Fallback,
                matched_rule_id: None,
            }));
        }
    }

    for i in 0..candidates.len() {
        let cand = &candidates[i];
        // body 从本标题行之后到下一个候选起始(或文末)。
        let body_start = next_byte_after_line(source, cand.heading_span.end);
        let body_end = candidates
            .get(i + 1)
            .map(|n| n.heading_span.start)
            .unwrap_or(source.len());
        let body_span = Span::new(body_start.min(body_end), body_end);

        match cand.level {
            HeadingLevel::Volume => {
                // 收尾前一卷,开新卷。
                if let Some(v) = current_volume.take() {
                    entries.push(BookEntry::Volume(v));
                }
                current_volume = Some(Volume {
                    title: cand.title.clone(),
                    chapters: Vec::new(),
                    heading_span: cand.heading_span,
                    origin: ChapterOrigin::RegexMatch,
                    matched_rule_id: Some(cand.rule_id.clone()),
                });

                // 若卷头到下一候选之间有实质内容(非空白),作为该卷的「卷前」单章
                // (Fallback origin)塞进卷,避免文本丢失。
                if !body_span.is_empty()
                    && !source[body_span.start..body_span.end].trim().is_empty()
                {
                    let preface = Chapter {
                        title: "(卷前)".into(),
                        paragraphs: Vec::new(), // 由 materialize_paragraphs 填
                        heading_span: Span::new(cand.heading_span.end, cand.heading_span.end),
                        body_span,
                        origin: ChapterOrigin::Fallback,
                        matched_rule_id: None,
                    };
                    if let Some(v) = current_volume.as_mut() {
                        v.chapters.push(preface);
                    }
                }
            }
            HeadingLevel::Chapter => {
                let chapter = Chapter {
                    title: cand.title.clone(),
                    paragraphs: Vec::new(), // 由 materialize_paragraphs 填
                    heading_span: cand.heading_span,
                    body_span,
                    origin: ChapterOrigin::RegexMatch,
                    matched_rule_id: Some(cand.rule_id.clone()),
                };
                if let Some(v) = current_volume.as_mut() {
                    v.chapters.push(chapter);
                } else {
                    entries.push(BookEntry::Chapter(chapter));
                }
            }
        }
    }

    // 收尾末卷
    if let Some(v) = current_volume.take() {
        entries.push(BookEntry::Volume(v));
    }

    // 第三阶段:超长章节检测——用中位数启发式寻找遗漏的内嵌标题,本地、无 LLM。
    let mut book = Book { metadata, entries };
    detect_oversized_chapters(&mut book, source, rules, 2.5);
    Ok(book)
}

/// 把段落物化到 [`Book`] 上,填充每个 [`Chapter::paragraphs`]。
///
/// 调用方应当在 [`crate::watermark::analyze`] 之后、auto 水印已镜像到 `cleaning` 之后
/// 再调本函数,使一次物化同时扣除"格式清洗 + 自动水印"两类删除。
///
/// `cleaning` 必须按 `span.start` 升序、互不重叠(即 [`crate::cleaning::analyze`] 的
/// 输出形式,或经 `merge_auto_watermarks_into_cleaning` 合并后的形式)。
///
/// 重入安全:本函数会**覆盖**每章既有 paragraphs(若已被填过)。
pub fn materialize_paragraphs(book: &mut Book, source: &str, cleaning: &[CleaningAnnotation]) {
    for entry in &mut book.entries {
        match entry {
            BookEntry::Chapter(c) => {
                c.paragraphs = paragraphs_from(source, c.body_span, cleaning);
            }
            BookEntry::Volume(v) => {
                for c in &mut v.chapters {
                    c.paragraphs = paragraphs_from(source, c.body_span, cleaning);
                }
            }
        }
    }
}

fn compile_rules(rules: &RuleSet, kind: RuleKind) -> Result<Vec<(String, Regex)>, RulesError> {
    rules
        .enabled_by_kind(kind)
        .into_iter()
        .map(|r| r.compile().map(|re| (r.id.clone(), re)))
        .collect()
}

fn try_match(
    line_start: usize,
    line_end: usize,
    trimmed: &str,
    compiled: &[(String, Regex)],
    level: HeadingLevel,
) -> Option<HeadingCandidate> {
    for (id, re) in compiled {
        if re.is_match(trimmed) {
            return Some(HeadingCandidate {
                level,
                heading_span: Span::new(line_start, line_end),
                title: trimmed.to_string(),
                rule_id: id.clone(),
            });
        }
    }
    None
}

/// 结构约束:行长度上限。规则正则本身已通过「`(?:...)?` 标题尾巴 + 各部分长度上限」
/// 限制了匹配范围,这里只做最后一道防线——避免某些病态正则把超长行误判。
fn passes_structural_constraints(trimmed: &str) -> bool {
    // 大约 100 字符上限。中文 1 字 = 3 字节,100 字符 ≈ 300 字节。
    trimmed.chars().take(101).count() <= 100
}

/// 给定一个标题行(end 指向行尾,不含 `\n`),返回**下一行起始**的字节偏移。
fn next_byte_after_line(source: &str, line_end: usize) -> usize {
    let bytes = source.as_bytes();
    if line_end < bytes.len() && bytes[line_end] == b'\n' {
        line_end + 1
    } else {
        line_end
    }
}

/// 把 source 中 `span` 范围内的内容,应用 cleaning 标注,然后按行切成段落。
fn paragraphs_from(source: &str, span: Span, cleaning_anns: &[CleaningAnnotation]) -> Vec<Paragraph> {
    if span.is_empty() {
        return Vec::new();
    }
    let body = &source[span.start..span.end];
    // 把全局 cleaning 标注裁剪到 body 区间,并平移坐标。
    let local: Vec<CleaningAnnotation> = cleaning_anns
        .iter()
        .filter(|a| a.span.start >= span.start && a.span.end <= span.end)
        .map(|a| CleaningAnnotation {
            span: Span::new(a.span.start - span.start, a.span.end - span.start),
            kind: a.kind,
            replacement: a.replacement.clone(),
        })
        .collect();
    let cleaned = cleaning::apply(body, &local);
    cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(Paragraph::new)
        .collect()
}

fn fallback_book(source: &str, metadata: Metadata) -> Book {
    let span = Span::new(0, source.len());
    let entries = vec![BookEntry::Chapter(Chapter {
        title: metadata.title.clone(),
        paragraphs: Vec::new(), // 由 materialize_paragraphs 填
        heading_span: Span::new(0, 0),
        body_span: span,
        origin: ChapterOrigin::Fallback,
        matched_rule_id: None,
    })];
    Book { metadata, entries }
}

// ── 阶段四 4.4:超长章节检测 ─────────────────────────────────────────────────

/// 在每个超长章节的 body 内寻找遗漏的标题行,就地拆分。
///
/// 判定阈值:所有章节字符数的中位数 × `median_factor`(默认 2.5)。
/// 拆出的子章节 origin 标记为 [`ChapterOrigin::Structural`]。
///
/// 条件不满足时(章节数 < 2,或中位数为 0)静默返回,不修改 book。
fn detect_oversized_chapters(book: &mut Book, source: &str, rules: &RuleSet, median_factor: f32) {
    // 收集所有章节的 body 字符数
    let mut char_counts: Vec<usize> = Vec::new();
    for entry in &book.entries {
        match entry {
            BookEntry::Chapter(c) => {
                char_counts.push(source[c.body_span.start..c.body_span.end].chars().count());
            }
            BookEntry::Volume(v) => {
                for c in &v.chapters {
                    char_counts.push(source[c.body_span.start..c.body_span.end].chars().count());
                }
            }
        }
    }
    // 少于 2 章时中位数无意义
    if char_counts.len() < 2 {
        return;
    }
    let median = compute_median(&mut char_counts);
    if median == 0 {
        return;
    }
    let threshold = (median as f32 * median_factor) as usize;

    let chapter_rules = compile_rules(rules, RuleKind::Chapter).unwrap_or_default();

    let old_entries = std::mem::take(&mut book.entries);
    let mut new_entries = Vec::with_capacity(old_entries.len());

    for entry in old_entries {
        match entry {
            BookEntry::Chapter(c) => {
                let len = source[c.body_span.start..c.body_span.end].chars().count();
                if len > threshold {
                    let split = split_oversized_chapter(c, source, &chapter_rules);
                    new_entries.extend(split.into_iter().map(BookEntry::Chapter));
                } else {
                    new_entries.push(BookEntry::Chapter(c));
                }
            }
            BookEntry::Volume(v) => {
                let mut new_chapters = Vec::with_capacity(v.chapters.len());
                for c in v.chapters {
                    let len = source[c.body_span.start..c.body_span.end].chars().count();
                    if len > threshold {
                        new_chapters.extend(split_oversized_chapter(c, source, &chapter_rules));
                    } else {
                        new_chapters.push(c);
                    }
                }
                new_entries.push(BookEntry::Volume(Volume {
                    title: v.title,
                    chapters: new_chapters,
                    heading_span: v.heading_span,
                    origin: v.origin,
                    matched_rule_id: v.matched_rule_id,
                }));
            }
        }
    }
    book.entries = new_entries;
}

/// 在章节 body 内找空行包围的短行(≤ 30 字)或命中章节规则的行,按这些行拆分。
/// 找不到拆分点时返回只含原章节的 `Vec`。
fn split_oversized_chapter(
    chapter: Chapter,
    source: &str,
    chapter_rules: &[(String, Regex)],
) -> Vec<Chapter> {
    let body_start = chapter.body_span.start;
    let body_end = chapter.body_span.end;
    if body_start >= body_end {
        return vec![chapter];
    }
    let body = &source[body_start..body_end];
    let lines = iter_lines(body);
    let n = lines.len();

    // 候选拆分点:(body 内 line_start, body 内 line_end, 标题文本)
    let mut splits: Vec<(usize, usize, String)> = Vec::new();

    for i in 0..n {
        let (ls, le) = lines[i];
        let trimmed = body[ls..le].trim();
        if trimmed.is_empty() {
            continue;
        }
        // 超过 30 字则不是标题候选
        if trimmed.chars().count() > 30 {
            continue;
        }
        // 两侧是否为空行(或 body 边界)
        let prev_blank = i == 0 || body[lines[i - 1].0..lines[i - 1].1].trim().is_empty();
        let next_blank = i + 1 >= n || body[lines[i + 1].0..lines[i + 1].1].trim().is_empty();
        // 命中章节规则
        let rule_match = chapter_rules.iter().any(|(_, re)| re.is_match(trimmed));

        if (prev_blank && next_blank) || rule_match {
            splits.push((ls, le, trimmed.to_string()));
        }
    }

    if splits.is_empty() {
        return vec![chapter];
    }

    let mut result: Vec<Chapter> = Vec::with_capacity(splits.len() + 1);

    // 第一段:原始标题行 + 第一个拆分点之前的 body
    let first_body_end = body_start + splits[0].0;
    result.push(Chapter {
        title: chapter.title.clone(),
        paragraphs: Vec::new(),
        heading_span: chapter.heading_span,
        body_span: Span::new(chapter.body_span.start, first_body_end),
        origin: chapter.origin,
        matched_rule_id: chapter.matched_rule_id.clone(),
    });

    // 后续每段:以找到的标题行为 heading,body 到下一个拆分点(或 body 末尾)
    for j in 0..splits.len() {
        let (sp_start, sp_end, ref title) = splits[j];
        let heading_start = body_start + sp_start;
        let heading_end = body_start + sp_end;
        // body 从该标题行后的第一行起
        let sub_body_start = body_start + next_byte_after_line(body, sp_end);
        let sub_body_end = if j + 1 < splits.len() {
            body_start + splits[j + 1].0
        } else {
            body_end
        };
        result.push(Chapter {
            title: title.clone(),
            paragraphs: Vec::new(),
            heading_span: Span::new(heading_start, heading_end),
            body_span: Span::new(sub_body_start.min(sub_body_end), sub_body_end),
            origin: ChapterOrigin::Structural,
            matched_rule_id: None,
        });
    }

    result
}

/// 中位数:对离群值稳健,优于平均值。偶数个元素取中间两者的平均(整数除法)。
fn compute_median(vals: &mut Vec<usize>) -> usize {
    if vals.is_empty() {
        return 0;
    }
    vals.sort_unstable();
    let n = vals.len();
    if n % 2 == 1 {
        vals[n / 2]
    } else {
        (vals[n / 2 - 1] + vals[n / 2]) / 2
    }
}

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
    use crate::cleaning;

    /// 测试辅助:跑完整"边界识别 + 物化"两步。
    ///
    /// v2 起 `CleaningConfig::default()` 全关,默认行为是"不清洗"。
    /// 本 helper **显式全开** 5 个 kind 保留 v1 行为,让本模块旧测试的 paragraphs
    /// 期望(段首剥全角、行尾去空白、空行压缩等)继续成立。
    fn parse_default(text: &str) -> Book {
        let cfg = cleaning::CleaningConfig {
            blank_line_compression: true,
            leading_fullwidth_space: true,
            inline_fullwidth_space: true,
            control_char: true,
            trailing_whitespace: true,
        };
        let cleaning_anns = cleaning::analyze(text, &cfg);
        let mut book = parse(text, &RuleSet::builtin(), Metadata::new("测试书", "测试作者")).unwrap();
        materialize_paragraphs(&mut book, text, &cleaning_anns);
        book
    }

    fn flatten_titles(book: &Book) -> Vec<&str> {
        let mut out = Vec::new();
        for e in &book.entries {
            match e {
                BookEntry::Chapter(c) => out.push(c.title.as_str()),
                BookEntry::Volume(v) => {
                    out.push(v.title.as_str());
                    for c in &v.chapters {
                        out.push(c.title.as_str());
                    }
                }
            }
        }
        out
    }

    #[test]
    fn splits_three_flat_chapters() {
        let text = "第一章 起\n正文1\n\n第二章 承\n正文2a\n正文2b\n第三章 转\n正文3";
        let book = parse_default(text);
        let titles = flatten_titles(&book);
        assert_eq!(titles, vec!["第一章 起", "第二章 承", "第三章 转"]);
    }

    #[test]
    fn captures_preface_before_first_heading() {
        let text = "这是楔子,正文还没开始\n\n第一章\n正文";
        let book = parse_default(text);
        let titles = flatten_titles(&book);
        assert_eq!(titles, vec!["楔子", "第一章"]);
        match &book.entries[0] {
            BookEntry::Chapter(c) => assert_eq!(c.origin, ChapterOrigin::Fallback),
            _ => panic!(),
        }
    }

    #[test]
    fn fallback_when_no_heading() {
        let text = "完全没有章节标题的一段文字\n第二行\n第三行";
        let book = parse_default(text);
        assert_eq!(flatten_titles(&book), vec!["测试书"]);
        match &book.entries[0] {
            BookEntry::Chapter(c) => {
                assert_eq!(c.origin, ChapterOrigin::Fallback);
                assert_eq!(c.paragraphs.len(), 3);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ignores_chapter_keyword_in_body() {
        let text = "第一章\n他正在读第一章 的内容\n第二章\n后续";
        let book = parse_default(text);
        assert_eq!(flatten_titles(&book), vec!["第一章", "第二章"]);
    }

    #[test]
    fn arabic_numerals_work() {
        let text = "第1章 阿拉伯数字\n正文\n第2章 第二\n正文";
        let book = parse_default(text);
        assert_eq!(flatten_titles(&book), vec!["第1章 阿拉伯数字", "第2章 第二"]);
    }

    #[test]
    fn organizes_into_volumes() {
        let text = "\
第一卷 风起
第一章 起
正文1
第二章 承
正文2
第二卷 云涌
第三章 转
正文3
";
        let book = parse_default(text);
        // 应当是两个卷,每卷下挂章
        assert_eq!(book.entries.len(), 2);
        match &book.entries[0] {
            BookEntry::Volume(v) => {
                assert_eq!(v.title, "第一卷 风起");
                assert_eq!(v.chapters.len(), 2);
                assert_eq!(v.chapters[0].title, "第一章 起");
                assert_eq!(v.chapters[1].title, "第二章 承");
                assert!(v.matched_rule_id.is_some());
            }
            _ => panic!("expected Volume"),
        }
        match &book.entries[1] {
            BookEntry::Volume(v) => {
                assert_eq!(v.title, "第二卷 云涌");
                assert_eq!(v.chapters.len(), 1);
                assert_eq!(v.chapters[0].title, "第三章 转");
            }
            _ => panic!("expected Volume"),
        }
    }

    #[test]
    fn pre_volume_chapter_goes_to_book_root() {
        let text = "\
楔子
楔子的内容
第一卷 风起
第一章 起
正文1
";
        let book = parse_default(text);
        // 「楔子」是章规则命中(序章/楔子模式),挂书根;然后才是「第一卷」。
        assert_eq!(book.entries.len(), 2);
        match &book.entries[0] {
            BookEntry::Chapter(c) => {
                assert_eq!(c.title, "楔子");
                assert_eq!(c.origin, ChapterOrigin::RegexMatch);
            }
            _ => panic!("expected Chapter at root"),
        }
        match &book.entries[1] {
            BookEntry::Volume(v) => {
                assert_eq!(v.title, "第一卷 风起");
                assert_eq!(v.chapters.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn heading_span_points_at_correct_bytes() {
        let text = "第一章 起\n正文1\n第二章 承\n正文2";
        let book = parse_default(text);
        let c1 = match &book.entries[0] {
            BookEntry::Chapter(c) => c,
            _ => panic!(),
        };
        // 第一章 起 在 source 中的起始字节应当是 0,end 应当等于该行末尾(不含 \n)
        assert_eq!(c1.heading_span.start, 0);
        let expected_end = "第一章 起".len();
        assert_eq!(c1.heading_span.end, expected_end);
        // body_span 应当紧接 \n 之后开始
        assert_eq!(c1.body_span.start, expected_end + 1);
        // body 应当包含 "正文1"
        let body = &text[c1.body_span.start..c1.body_span.end];
        assert!(body.starts_with("正文1"));
    }

    #[test]
    fn paragraphs_apply_cleaning_within_body() {
        // 行尾尾随空白应被清洗掉,反映在 paragraph 的文本中
        let text = "第一章\n正文1   \n正文2\u{3000}\n";
        let book = parse_default(text);
        let c = match &book.entries[0] {
            BookEntry::Chapter(c) => c,
            _ => panic!(),
        };
        let texts: Vec<&str> = c.paragraphs.iter().map(|p| p.as_str()).collect();
        assert_eq!(texts, vec!["正文1", "正文2"]);
    }

    #[test]
    fn long_lines_are_not_treated_as_headings() {
        // 100 个 "第一章" 拼一起就太长,不应被认为是标题
        let line = "第一章 ".to_string() + &"很长的标题".repeat(30);
        let text = format!("{line}\n正文");
        let book = parse_default(&text);
        // 没识别出章节 → fallback
        assert_eq!(flatten_titles(&book), vec!["测试书"]);
    }

    #[test]
    fn volume_only_then_chapters() {
        let text = "第一卷\n第一章\n正文1\n";
        let book = parse_default(text);
        assert_eq!(book.entries.len(), 1);
        match &book.entries[0] {
            BookEntry::Volume(v) => {
                assert_eq!(v.chapters.len(), 1);
                assert_eq!(v.chapters[0].title, "第一章");
            }
            _ => panic!(),
        }
    }

    // ── 阶段四 4.4 测试 ──────────────────────────────────────────────────────

    /// 超长章节内的「空行包围短行」被识别为拆分点,拆出的子章 origin = Structural。
    #[test]
    fn oversized_chapter_split_at_structural_headings() {
        // 5 个普通章节,body 各约 52 字
        let normal_body = "正文内容\n".repeat(13); // 13 × 4 chars = 52 chars
        // 1 个超长章节:含两个「空行包围的短行」作为结构标题,共约 164 chars
        let section = "正文内容\n".repeat(13);
        let giant_body = format!("{section}\n独立小节\n\n{section}\n又一小节\n\n{section}");

        let text = format!(
            "第一章 一\n{normal_body}\n\
             第二章 二\n{normal_body}\n\
             第三章 三\n{normal_body}\n\
             第四章 四\n{normal_body}\n\
             第五章 五（超长）\n{giant_body}"
        );

        let book = parse(&text, &RuleSet::builtin(), Metadata::new("测试", "作者")).unwrap();

        // 超长章被拆成 3 段:第五章 + 独立小节 + 又一小节
        let mut found_structural = 0usize;
        let mut found_title = false;
        for entry in &book.entries {
            if let BookEntry::Chapter(c) = entry {
                if c.origin == ChapterOrigin::Structural {
                    found_structural += 1;
                }
                if c.title == "独立小节" || c.title == "又一小节" {
                    found_title = true;
                }
            }
        }
        assert!(found_structural >= 2, "应当至少拆出 2 个 Structural 子章,实际 {found_structural}");
        assert!(found_title, "应当在 entries 中找到拆分出的子章标题");
    }

    /// 所有章节长度相近时,不应发生任何拆分。
    #[test]
    fn normal_chapters_are_not_split() {
        let body = "正文内容\n".repeat(20); // 均匀,不存在离群值
        let text = format!(
            "第一章 一\n{body}第二章 二\n{body}第三章 三\n{body}第四章 四\n{body}"
        );
        let book = parse(&text, &RuleSet::builtin(), Metadata::new("测试", "作者")).unwrap();
        // 4 个章节,无一被拆分
        assert_eq!(book.entries.len(), 4, "均匀章节不应被拆分,实际 entries = {}", book.entries.len());
        for e in &book.entries {
            if let BookEntry::Chapter(c) = e {
                assert_ne!(
                    c.origin,
                    ChapterOrigin::Structural,
                    "均匀章节不应出现 Structural origin"
                );
            }
        }
    }

    /// 阶段三签名拆分:`parse` 单独调用时,所有 chapter 的 paragraphs 必须为空,
    /// 物化只能由 [`materialize_paragraphs`] 完成。
    #[test]
    fn parse_alone_leaves_paragraphs_empty() {
        let text = "\
第一卷 风起
第一章 起
正文1
第二章 承
正文2
第二卷 云涌
第三章 转
正文3
";
        let book = parse(text, &RuleSet::builtin(), Metadata::new("测试", "作者")).unwrap();
        for entry in &book.entries {
            match entry {
                BookEntry::Chapter(c) => assert!(
                    c.paragraphs.is_empty(),
                    "parse 单独调用时 Chapter.paragraphs 必须为空"
                ),
                BookEntry::Volume(v) => {
                    for c in &v.chapters {
                        assert!(
                            c.paragraphs.is_empty(),
                            "parse 单独调用时卷内 Chapter.paragraphs 必须为空"
                        );
                    }
                }
            }
        }
    }

    /// `materialize_paragraphs` 应当填充所有章(包括卷内章)的 paragraphs,且重入覆盖。
    #[test]
    fn materialize_paragraphs_fills_volumes_and_is_idempotent() {
        let text = "第一卷 风起\n第一章 起\n正文1\n正文2\n第二卷 云涌\n第三章 转\n正文3\n";
        let mut book = parse(text, &RuleSet::builtin(), Metadata::new("测试", "作者")).unwrap();
        let cleaning_anns = cleaning::analyze(text, &cleaning::CleaningConfig::default());

        materialize_paragraphs(&mut book, text, &cleaning_anns);
        // 卷 1 第一章应有 2 段;卷 2 第三章应有 1 段。
        let first_vol = match &book.entries[0] {
            BookEntry::Volume(v) => v,
            _ => panic!(),
        };
        assert_eq!(first_vol.chapters[0].paragraphs.len(), 2);
        let second_vol = match &book.entries[1] {
            BookEntry::Volume(v) => v,
            _ => panic!(),
        };
        assert_eq!(second_vol.chapters[0].paragraphs.len(), 1);

        // 重入:再调一次应当覆盖,而不是累加
        materialize_paragraphs(&mut book, text, &cleaning_anns);
        let first_vol2 = match &book.entries[0] {
            BookEntry::Volume(v) => v,
            _ => panic!(),
        };
        assert_eq!(first_vol2.chapters[0].paragraphs.len(), 2, "重入应覆盖,不应累加");
    }
}
