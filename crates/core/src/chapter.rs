//! 章节切分。
//!
//! 阶段零只用一条「第X章」正则,在多行模式下按行首匹配,把命中位置之间的文本作为正文。
//! 后续阶段会扩展成 CLAUDE.md 第六节描述的四阶段流水线(规则库、结构补偿、灰区仲裁、兜底降级)。
//!
//! 输出的 Book 在阶段零里 `entries` 直接是一串 `Chapter`,不识别卷。

use std::sync::OnceLock;

use regex::Regex;
use thiserror::Error;

use crate::domain::{Book, BookEntry, Chapter, ChapterOrigin, Metadata, Paragraph};

#[derive(Debug, Error)]
pub enum ChapterError {
    #[error("章节正则编译失败: {0}")]
    Regex(#[from] regex::Error),
}

fn chapter_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        // 行首可有空白/全角空格;然后是「第」+ 中文数字或阿拉伯数字 + 「章」;
        // 章后可跟空格 + 标题文字;整行不超过 60 字(粗略约束,阻止把正文里出现的「第X章」误识别)。
        Regex::new(
            r"(?m)^[ \t\x{3000}]*第[0-9零一二三四五六七八九十百千万两]{1,15}章(?:[ \t\x{3000}].{0,60})?[ \t\x{3000}]*$",
        )
        .expect("内置章节正则应当总是合法")
    })
}

pub fn split(text: &str, metadata: Metadata) -> Result<Book, ChapterError> {
    let re = chapter_regex();

    let matches: Vec<(usize, usize, String)> = re
        .find_iter(text)
        .map(|m| (m.start(), m.end(), m.as_str().trim().to_string()))
        .collect();

    let mut entries: Vec<BookEntry> = Vec::new();

    if matches.is_empty() {
        // 兜底:整本作为一章。标明 `Fallback` 出处,让阶段二的预览界面提示用户手动干预。
        let char_count = text.chars().count();
        entries.push(BookEntry::Chapter(Chapter {
            title: metadata.title.clone(),
            paragraphs: paragraphs_of(text),
            source_start: 0,
            source_end: char_count,
            origin: ChapterOrigin::Fallback,
        }));
        return Ok(Book { metadata, entries });
    }

    // 首个章节标题之前如果有实质内容,作为「楔子」单独一章。
    let first_start = matches[0].0;
    if first_start > 0 {
        let preface = &text[..first_start];
        if !preface.trim().is_empty() {
            let char_count = preface.chars().count();
            entries.push(BookEntry::Chapter(Chapter {
                title: "楔子".into(),
                paragraphs: paragraphs_of(preface),
                source_start: 0,
                source_end: char_count,
                origin: ChapterOrigin::Fallback,
            }));
        }
    }

    for (i, m) in matches.iter().enumerate() {
        let (start, header_end, title) = (m.0, m.1, m.2.clone());
        let body_end = matches.get(i + 1).map(|n| n.0).unwrap_or(text.len());
        let body = &text[header_end..body_end];

        let source_start = text[..start].chars().count();
        let source_end = source_start + text[start..body_end].chars().count();

        entries.push(BookEntry::Chapter(Chapter {
            title,
            paragraphs: paragraphs_of(body),
            source_start,
            source_end,
            origin: ChapterOrigin::RegexMatch,
        }));
    }

    Ok(Book { metadata, entries })
}

fn paragraphs_of(s: &str) -> Vec<Paragraph> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(Paragraph::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter_titles(book: &Book) -> Vec<&str> {
        book.entries
            .iter()
            .map(|e| match e {
                BookEntry::Chapter(c) => c.title.as_str(),
                BookEntry::Volume(v) => v.title.as_str(),
            })
            .collect()
    }

    fn chapters(book: &Book) -> Vec<&Chapter> {
        book.entries
            .iter()
            .filter_map(|e| match e {
                BookEntry::Chapter(c) => Some(c),
                BookEntry::Volume(_) => None,
            })
            .collect()
    }

    #[test]
    fn splits_three_chapters() {
        let text = "第一章 起\n正文1\n\n第二章 承\n正文2a\n正文2b\n第三章 转\n正文3";
        let book = split(text, Metadata::new("测试书", "测试作者")).unwrap();
        let titles = chapter_titles(&book);
        assert_eq!(titles, vec!["第一章 起", "第二章 承", "第三章 转"]);

        let chs = chapters(&book);
        assert_eq!(chs[0].paragraphs.len(), 1);
        assert_eq!(chs[1].paragraphs.len(), 2);
        assert_eq!(chs[1].paragraphs[0].as_str(), "正文2a");
    }

    #[test]
    fn captures_preface_before_first_heading() {
        let text = "这是楔子,正文还没开始\n\n第一章\n正文";
        let book = split(text, Metadata::new("书", "作者")).unwrap();
        let titles = chapter_titles(&book);
        assert_eq!(titles, vec!["楔子", "第一章"]);
        assert_eq!(
            chapters(&book)[0].origin,
            ChapterOrigin::Fallback,
            "preface 应当标记为 Fallback"
        );
    }

    #[test]
    fn fallbacks_to_single_chapter_when_no_heading() {
        let text = "完全没有章节标题的一段文字\n第二行\n第三行";
        let book = split(text, Metadata::new("无章节书", "作者")).unwrap();
        assert_eq!(chapter_titles(&book), vec!["无章节书"]);
        assert_eq!(chapters(&book)[0].origin, ChapterOrigin::Fallback);
        assert_eq!(chapters(&book)[0].paragraphs.len(), 3);
    }

    #[test]
    fn ignores_chapter_keyword_inside_body() {
        // 正文里提到「第一章」不应当被当成章节标题
        let text = "第一章\n他正在读第一章 的内容\n第二章\n后续";
        let book = split(text, Metadata::new("书", "作者")).unwrap();
        assert_eq!(chapter_titles(&book), vec!["第一章", "第二章"]);
        assert_eq!(chapters(&book)[0].paragraphs.len(), 1);
    }

    #[test]
    fn accepts_arabic_numerals() {
        let text = "第1章 阿拉伯数字\n正文\n第2章 第二\n正文";
        let book = split(text, Metadata::new("书", "作者")).unwrap();
        assert_eq!(chapter_titles(&book), vec!["第1章 阿拉伯数字", "第2章 第二"]);
    }
}
