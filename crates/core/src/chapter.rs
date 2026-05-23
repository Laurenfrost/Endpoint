//! 章节解析:核心库的心脏。
//!
//! 阶段一对应 CLAUDE.md 第六节的前两阶段:
//! 1. **候选行扫描**:用 [`rules::RuleSet`] 中的章节/卷规则逐行匹配,叠加结构约束。
//! 2. **层级归属**:把候选组织成卷章两级,每章归属前面最近的卷;卷之前的章(楔子/序章)挂书根。
//!
//! 阶段三(超长区间检测)、阶段四(LLM 兜底)不在本阶段实现。整本未识别任何标题时
//! 沿用阶段零的「单章 Fallback」兜底。
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

    Ok(Book { metadata, entries })
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

    /// 测试辅助:跑完整"边界识别 + 物化"两步,与阶段二 `parse(.., cleaning, ..)` 行为等价。
    fn parse_default(text: &str) -> Book {
        let cleaning_anns = cleaning::analyze(text);
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
        let cleaning_anns = cleaning::analyze(text);

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
