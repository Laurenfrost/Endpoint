//! 文本清洗:**确定性**部分。
//!
//! 阶段一只处理低风险的格式整理。水印检测、行频统计、困惑度等"智能"内容属阶段三,
//! 不写进此模块。
//!
//! # 设计要点
//!
//! 1. **不修改原文**:[`analyze`] 仅产出标注列表,不返回任何字符串。原文是 UI 主显示的
//!    锚点(参见 [`crate::domain`] 模块文档的契约第 3 条),清洗结果以红色高亮覆盖在原文上,
//!    所以"清洗后文本"应当作为按需 derive 的视图存在。
//! 2. **annotation 的 span 在 decoded source 坐标系**(UTF-8 字节偏移)。
//! 3. **保证非重叠**:输出列表按 `span.start` 升序、互不重叠。这样 [`apply`] 可以单次
//!    线性扫描完成。
//!
//! # 阶段一覆盖的清洗类型
//!
//! | 类型 | 范围 | 策略 |
//! |------|------|------|
//! | [`CleaningKind::BlankLineCompression`] | 连续 3 个及以上换行符(允许之间夹空白) | 全部用 `\n\n` 替换(保留段落分隔) |
//! | [`CleaningKind::TrailingWhitespace`] | 非空行的行尾 ` ` / `\t` / `\u{3000}` 串 | 删除 |
//! | [`CleaningKind::FullwidthSpace`] | 非空行行首的 `\u{3000}` 串 | 删除(EPUB CSS 控制缩进) |
//! | [`CleaningKind::ControlChar`] | `\u{0000}`-`\u{001F}` 除 `\t`/`\n` 之外的字符 + `\u{007F}` | 删除 |
//!
//! 注意:**不**剥离行首半角空格(可能是诗歌等有意排版),**不**触碰行内的全角空格
//! (可能是排版的分隔符)。所有规则都保守、可解释。

use std::sync::OnceLock;

use regex::Regex;

use crate::domain::{CleaningAnnotation, CleaningKind, Span};

/// 扫描文本,产出按 `span.start` 升序排列、互不重叠的清洗标注列表。
pub fn analyze(text: &str) -> Vec<CleaningAnnotation> {
    let mut anns: Vec<CleaningAnnotation> = Vec::new();

    // —— 第一遍:逐行分析。 ——
    // 行的定义:由 `\n` 分隔的字节区间(不含 `\n`)。
    // TODO(cancel): 接 `ConvertOptions.cancel_token` 后,每 N 行检查一次取消标志,
    // 提前返回。阶段二只预留接口,不实装。
    let mut line_start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            analyze_line(text, line_start, i, &mut anns);
            line_start = i + 1;
        }
    }
    // 处理最后一行(可能没有以 `\n` 结尾)。
    if line_start < text.len() {
        analyze_line(text, line_start, text.len(), &mut anns);
    }

    // —— 第二遍:连续空行压缩。 ——
    // 「\n + (含可选空白的换行) * 2 次以上」=「3+ 个换行」,意味着至少 2 个空白行。
    // 全部替换为 `\n\n`(保留段落分隔的一行空)。
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\r?\n(?:[ \t\x{3000}]*\r?\n){2,}").expect("内置空行正则应当总是合法")
    });
    for m in re.find_iter(text) {
        anns.push(CleaningAnnotation {
            span: Span::new(m.start(), m.end()),
            kind: CleaningKind::BlankLineCompression,
            replacement: Some("\n\n".to_string()),
        });
    }

    // 排序后去除被 BlankLineCompression 吞掉的内部 annotation
    //(对空白行的每行分析本就不会产出 annotation,所以理论上不会发生;
    // 但保留兜底以维持「非重叠」不变式)。
    anns.sort_by_key(|a| (a.span.start, a.span.end));
    dedup_contained(&mut anns);
    anns
}

/// 按 [`analyze`] 的输出把清洗应用到原文,得到清洗后字符串。
///
/// 调用方传入的 `anns` 必须按 `span.start` 升序且互不重叠(即 [`analyze`] 的输出形式)。
pub fn apply(text: &str, anns: &[CleaningAnnotation]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for a in anns {
        if a.span.start < cursor {
            // 防御:理论上不会触发(analyze 保证非重叠)。
            continue;
        }
        out.push_str(&text[cursor..a.span.start]);
        if let Some(rep) = &a.replacement {
            out.push_str(rep);
        }
        cursor = a.span.end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn analyze_line(text: &str, line_start: usize, line_end: usize, anns: &mut Vec<CleaningAnnotation>) {
    if line_start == line_end {
        return; // 空行 → 由空行压缩处理
    }
    let line = &text[line_start..line_end];

    // 整行是否全空白?是 → 跳过逐行分析(交给空行压缩)。
    if line.bytes().all(is_ws_byte) && line.chars().all(is_line_whitespace) {
        return;
    }

    // 1. 行尾尾随空白
    let trimmed_end_relative = line.trim_end_matches(is_line_whitespace).len();
    if trimmed_end_relative < line.len() {
        anns.push(CleaningAnnotation {
            span: Span::new(line_start + trimmed_end_relative, line_end),
            kind: CleaningKind::TrailingWhitespace,
            replacement: None,
        });
    }

    // 取去尾后的有效行,用于查行首全角空格与行内控制字符
    let effective_line = &line[..trimmed_end_relative];

    // 2. 行首全角空格(连续的)
    let mut leading_fw_end_rel = 0usize;
    for (idx, c) in effective_line.char_indices() {
        if c == '\u{3000}' {
            leading_fw_end_rel = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if leading_fw_end_rel > 0 {
        anns.push(CleaningAnnotation {
            span: Span::new(line_start, line_start + leading_fw_end_rel),
            kind: CleaningKind::FullwidthSpace,
            replacement: None,
        });
    }

    // 3. 控制字符(在行的有效区域内、且不与行首全角空格重叠)
    for (idx, c) in effective_line.char_indices() {
        if idx < leading_fw_end_rel {
            continue;
        }
        if is_strippable_control(c) {
            let abs = line_start + idx;
            anns.push(CleaningAnnotation {
                span: Span::new(abs, abs + c.len_utf8()),
                kind: CleaningKind::ControlChar,
                replacement: None,
            });
        }
    }
}

fn is_ws_byte(b: u8) -> bool {
    // 快速路径:ASCII 空白
    matches!(b, b' ' | b'\t' | b'\r')
        || b >= 0x80 // 非 ASCII,留给 `is_line_whitespace` 复核
}

fn is_line_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\u{3000}')
}

fn is_strippable_control(c: char) -> bool {
    match c {
        '\t' | '\n' | '\r' => false, // 保留(\r 由 trailing whitespace 处理)
        c if (c as u32) < 0x20 => true,
        '\u{007F}' => true,
        _ => false,
    }
}

fn dedup_contained(anns: &mut Vec<CleaningAnnotation>) {
    let mut i = 0;
    while i + 1 < anns.len() {
        let cur_end = anns[i].span.end;
        // 删除所有完全被 anns[i] 包含的后续 annotation。
        let j = i + 1;
        while j < anns.len() && anns[j].span.end <= cur_end {
            anns.remove(j);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_yields_no_annotations() {
        let text = "第一章\n正文内容\n";
        assert!(analyze(text).is_empty());
    }

    #[test]
    fn detects_trailing_whitespace() {
        let text = "正文1   \n正文2\t\t\n正文3\u{3000}\n";
        let anns = analyze(text);
        assert_eq!(anns.len(), 3);
        for a in &anns {
            assert_eq!(a.kind, CleaningKind::TrailingWhitespace);
            assert!(a.replacement.is_none());
        }
        // 第一处尾随空白覆盖 "   "(3 个 ASCII 空格)
        assert_eq!(&text[anns[0].span.start..anns[0].span.end], "   ");
    }

    #[test]
    fn detects_leading_fullwidth_space() {
        let text = "\u{3000}\u{3000}正文\n";
        let anns = analyze(text);
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].kind, CleaningKind::FullwidthSpace);
        assert_eq!(
            &text[anns[0].span.start..anns[0].span.end],
            "\u{3000}\u{3000}"
        );
    }

    #[test]
    fn leaves_inline_fullwidth_space_alone() {
        // 行内的全角空格不剥离(可能是有意排版)
        let text = "前文\u{3000}后文\n";
        let anns = analyze(text);
        assert!(anns.is_empty());
    }

    #[test]
    fn strips_control_characters() {
        // 行中夹杂 \u{0001} \u{0007} \u{007F}
        let text = "前\u{0001}中\u{0007}后\u{007F}\n";
        let anns = analyze(text);
        assert_eq!(anns.len(), 3);
        for a in &anns {
            assert_eq!(a.kind, CleaningKind::ControlChar);
        }
    }

    #[test]
    fn preserves_tab_and_newline() {
        // \t 不应当被识别为控制字符;\n 作为换行符也不被剥离
        let text = "前\t中\n后\n";
        let anns = analyze(text);
        assert!(
            anns.is_empty(),
            "\\t 与 \\n 不应触发 ControlChar,实际产出 {:?}",
            anns
        );
    }

    #[test]
    fn compresses_blank_line_runs() {
        let text = "para A\n\n\n\npara B";
        let anns = analyze(text);
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].kind, CleaningKind::BlankLineCompression);
        assert_eq!(anns[0].replacement.as_deref(), Some("\n\n"));
        // span 应覆盖全部 4 个 \n
        assert_eq!(&text[anns[0].span.start..anns[0].span.end], "\n\n\n\n");
    }

    #[test]
    fn does_not_compress_single_blank_line() {
        // 两个 \n(=一个空白行)是合法段落分隔,不动
        let text = "para A\n\npara B";
        let anns = analyze(text);
        assert!(anns.is_empty(), "{:?}", anns);
    }

    #[test]
    fn blank_lines_with_interior_whitespace_compress() {
        let text = "A\n  \n\t\n  \nB";
        let anns = analyze(text);
        // 整体匹配为一段空行压缩,内部含 4 个 \n
        assert!(anns
            .iter()
            .any(|a| a.kind == CleaningKind::BlankLineCompression));
    }

    #[test]
    fn apply_round_trip_full_example() {
        let text = "\u{3000}\u{3000}第一章 起   \n\n\n\n  正文1\t\t\n";
        let anns = analyze(text);
        let cleaned = apply(text, &anns);
        assert_eq!(cleaned, "第一章 起\n\n  正文1\n");
    }

    #[test]
    fn apply_no_annotations_is_identity() {
        let text = "干净的文本\n第二行\n";
        let cleaned = apply(text, &[]);
        assert_eq!(cleaned, text);
    }

    #[test]
    fn annotations_are_sorted_and_non_overlapping() {
        let text = "\u{3000}前\u{0001}文   \n\n\n\n下一行\n";
        let anns = analyze(text);
        for w in anns.windows(2) {
            assert!(
                w[0].span.end <= w[1].span.start,
                "annotations overlap: {:?} {:?}",
                w[0],
                w[1]
            );
        }
    }
}
