//! 领域模型:贯穿全部模块的「通用语言」。
//!
//! 本文件只放类型定义,不放逻辑。CLAUDE.md 第五节描述了完整契约;阶段零先把字段立起来,
//! 后续阶段会在不改字段名的前提下补全行为(出处标记、source 偏移、卷识别等)。

/// 一本完整的书。
#[derive(Debug, Clone)]
pub struct Book {
    pub metadata: Metadata,
    /// 顶层条目有序列表:卷或章。无卷小说就是一串 `Chapter`,有卷小说混入 `Volume`。
    pub entries: Vec<BookEntry>,
}

#[derive(Debug, Clone)]
pub enum BookEntry {
    Volume(Volume),
    Chapter(Chapter),
}

#[derive(Debug, Clone)]
pub struct Volume {
    pub title: String,
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    pub paragraphs: Vec<Paragraph>,
    /// 在原始 txt 文本中的起止字符偏移(以 `char` 为单位,而非字节)。阶段零粗略填,
    /// 阶段二前会冻结契约。
    pub source_start: usize,
    pub source_end: usize,
    pub origin: ChapterOrigin,
}

/// 段落:纯文本,**不包含任何 XHTML**。段落 → XHTML 的转换在 EPUB 构建阶段进行。
#[derive(Debug, Clone)]
pub struct Paragraph(pub String);

impl Paragraph {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 「这一章是怎么来的」的出处标记。阶段零只会用到 `RegexMatch` 与 `Fallback`,
/// 其他变体留给后续阶段填充。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChapterOrigin {
    /// 由章节正则规则匹配得到。
    RegexMatch,
    /// 结构分析(超长章二次切分)补出的章。
    Structural,
    /// LLM 灰区仲裁产出。
    LlmAdjudicated,
    /// 整本未识别出任何章节标题,按空行/字数兜底切分。
    Fallback,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub language: String,
    pub cover: Option<Vec<u8>>,
    pub description: Option<String>,
}

impl Metadata {
    /// 最简构造:只指定书名作者,其他取默认。
    pub fn new(title: impl Into<String>, author: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            author: author.into(),
            language: "zh-CN".into(),
            cover: None,
            description: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_defaults_language_to_zh_cn() {
        let m = Metadata::new("斗破苍穹", "天蚕土豆");
        assert_eq!(m.language, "zh-CN");
        assert!(m.cover.is_none());
    }

    #[test]
    fn book_entry_enum_round_trips() {
        let ch = Chapter {
            title: "第一章 风云起".into(),
            paragraphs: vec![Paragraph::new("test")],
            source_start: 0,
            source_end: 4,
            origin: ChapterOrigin::RegexMatch,
        };
        let entry = BookEntry::Chapter(ch);
        match entry {
            BookEntry::Chapter(c) => assert_eq!(c.title, "第一章 风云起"),
            BookEntry::Volume(_) => panic!("expected Chapter"),
        }
    }
}
