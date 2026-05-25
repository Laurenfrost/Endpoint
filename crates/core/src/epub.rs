//! EPUB 3 构建。
//!
//! 阶段零最小可用版本基础上,阶段四 4.0 扩展:
//! - **CSS 覆盖**:通过 [`EpubOptions::css_override`] 替换内嵌的 `DEFAULT_CSS`。
//! - **封面嵌入**:通过 [`EpubOptions::cover`] 写入封面图片 + 封面 XHTML 页 +
//!   OPF manifest `properties="cover-image"` + spine 首位。
//! - **字体嵌入**([`EpubOptions::font_bytes`]):接口已定义,4.1 子阶段填实逻辑。

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;
use zip::write::{FileOptions, ZipWriter};
use zip::CompressionMethod;

use crate::domain::{Book, BookEntry, Chapter, Volume};

// ==================== 公开类型 ====================

/// 封面图片的 MIME 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverMime {
    Jpeg,
    Png,
    Svg,
}

impl CoverMime {
    pub fn as_str(self) -> &'static str {
        match self {
            CoverMime::Jpeg => "image/jpeg",
            CoverMime::Png => "image/png",
            CoverMime::Svg => "image/svg+xml",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            CoverMime::Jpeg => "jpg",
            CoverMime::Png => "png",
            CoverMime::Svg => "svg",
        }
    }

    /// 从文件路径扩展名推断 MIME 类型。不认识的扩展名返回 `None`。
    pub fn from_path(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
            Some(CoverMime::Jpeg)
        } else if lower.ends_with(".png") {
            Some(CoverMime::Png)
        } else if lower.ends_with(".svg") {
            Some(CoverMime::Svg)
        } else {
            None
        }
    }

    /// 从已分离的小写扩展名字符串推断 MIME 类型(`"jpg"`/`"jpeg"`/`"png"`/`"svg"`)。
    pub fn from_path_ext(ext: &str) -> Option<Self> {
        match ext {
            "jpg" | "jpeg" => Some(CoverMime::Jpeg),
            "png" => Some(CoverMime::Png),
            "svg" => Some(CoverMime::Svg),
            _ => None,
        }
    }
}

/// 嵌入字体的二进制资产。阶段四 4.1 起在 `build()` 中实际写入 zip。
///
/// 字节由桥接层从 Tauri resource 目录读取后传入——核心库不做文件 I/O。
#[derive(Debug, Clone)]
pub struct FontBytes {
    /// 字体展示名称,用于 CSS `font-family` 声明(如 `"LXGWWenKai"`)。
    pub name: String,
    /// Regular 字重字体字节(TTF 或 OTF)。
    pub regular: Vec<u8>,
}

/// EPUB 构建选项。所有字段均可选,缺省值取保守默认。
///
/// ## 子阶段实现状态
/// - **4.0**:`css_override` + `cover` + `cover_mime`。
/// - **4.1**:`font_bytes`(接口已在此定义,build 逻辑待实现)。
pub struct EpubOptions<'a> {
    /// 覆盖内嵌的 `DEFAULT_CSS`。`None` 使用默认样式。
    pub css_override: Option<&'a str>,
    /// 封面图片字节。`None` 表示无封面,epub 不生成封面页。
    pub cover: Option<&'a [u8]>,
    /// 封面 MIME 类型。`cover` 非 `None` 时有效;默认 `CoverMime::Jpeg`。
    pub cover_mime: CoverMime,
    /// 嵌入字体(阶段四 4.1 正式实现)。`None` 不嵌入字体。
    pub font_bytes: Option<&'a FontBytes>,
}

impl Default for EpubOptions<'_> {
    fn default() -> Self {
        EpubOptions {
            css_override: None,
            cover: None,
            cover_mime: CoverMime::Jpeg,
            font_bytes: None,
        }
    }
}

// ==================== 错误类型 ====================

#[derive(Debug, Error)]
pub enum EpubError {
    #[error("无法创建输出文件 {path}: {source}")]
    CreateOutput {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("写入 epub 失败: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("写入 epub 失败: {0}")]
    Io(#[from] io::Error),
}

const DEFAULT_CSS: &str = include_str!("default.css");

// ==================== 主构建入口 ====================

pub fn build(book: &Book, out_path: &Path, opts: &EpubOptions<'_>) -> Result<(), EpubError> {
    let file = File::create(out_path).map_err(|e| EpubError::CreateOutput {
        path: out_path.display().to_string(),
        source: e,
    })?;
    let mut zw = ZipWriter::new(file);

    let stored = FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = FileOptions::default().compression_method(CompressionMethod::Deflated);

    // mimetype 必须第一个、STORED、无 extra field。
    zw.start_file("mimetype", stored)?;
    zw.write_all(b"application/epub+zip")?;

    zw.start_file("META-INF/container.xml", deflated)?;
    zw.write_all(CONTAINER_XML.as_bytes())?;

    // CSS(可覆盖)
    let css = opts.css_override.unwrap_or(DEFAULT_CSS);
    zw.start_file("OEBPS/styles.css", deflated)?;
    zw.write_all(css.as_bytes())?;

    // 封面图片 + 封面 XHTML 页(可选)
    let has_cover = opts.cover.is_some();
    if let Some(cover_bytes) = opts.cover {
        let ext = opts.cover_mime.extension();
        zw.start_file(format!("OEBPS/cover.{ext}"), deflated)?;
        zw.write_all(cover_bytes)?;

        zw.start_file("OEBPS/cover.xhtml", deflated)?;
        zw.write_all(cover_xhtml(opts.cover_mime).as_bytes())?;
    }

    // TODO(4.1):字体嵌入(font_bytes 非空时写入 OEBPS/fonts/ + 追加 @font-face CSS)

    let sections = collect_sections(&book.entries);
    let uid = generate_uid(book);
    let modified = format_modified();

    for s in &sections {
        zw.start_file(format!("OEBPS/{}", s.href), deflated)?;
        let xhtml = match s.kind {
            SectionKind::Chapter(c) => chapter_xhtml(c),
            SectionKind::Volume(v) => volume_xhtml(v),
        };
        zw.write_all(xhtml.as_bytes())?;
    }

    zw.start_file("OEBPS/nav.xhtml", deflated)?;
    zw.write_all(nav_xhtml(&book.entries, &sections).as_bytes())?;

    zw.start_file("OEBPS/toc.ncx", deflated)?;
    zw.write_all(toc_ncx(book, &book.entries, &sections, &uid).as_bytes())?;

    zw.start_file("OEBPS/content.opf", deflated)?;
    zw.write_all(content_opf(book, &sections, &uid, &modified, opts.cover_mime, has_cover).as_bytes())?;

    zw.finish()?;
    Ok(())
}

// ==================== 内部类型 ====================

/// 写入 EPUB 的一份"小节":要么是卷分隔页,要么是章节正文页。每份小节对应
/// 一个 manifest item + spine itemref + 独立 xhtml 文件。
struct Section<'a> {
    href: String,
    id: String,
    kind: SectionKind<'a>,
}

enum SectionKind<'a> {
    Volume(&'a Volume),
    Chapter(&'a Chapter),
}

/// 把书的卷章树展开成线性的 [`Section`] 列表。卷标题被赋予独立的占位页,以保证:
/// 1. 卷标题不再丢失(阶段零的 flatten_chapters 直接吞了卷标题——本次修复的 bug)。
/// 2. 卷在 TOC 与 spine 中都有可点击/可翻阅的入口。
fn collect_sections(entries: &[BookEntry]) -> Vec<Section<'_>> {
    let mut out = Vec::new();
    let mut vol_idx = 0usize;
    let mut ch_idx = 0usize;
    for e in entries {
        match e {
            BookEntry::Chapter(c) => {
                ch_idx += 1;
                out.push(Section {
                    href: format!("chapter_{:04}.xhtml", ch_idx),
                    id: format!("ch{:04}", ch_idx),
                    kind: SectionKind::Chapter(c),
                });
            }
            BookEntry::Volume(v) => {
                vol_idx += 1;
                out.push(Section {
                    href: format!("volume_{:04}.xhtml", vol_idx),
                    id: format!("vol{:04}", vol_idx),
                    kind: SectionKind::Volume(v),
                });
                for c in &v.chapters {
                    ch_idx += 1;
                    out.push(Section {
                        href: format!("chapter_{:04}.xhtml", ch_idx),
                        id: format!("ch{:04}", ch_idx),
                        kind: SectionKind::Chapter(c),
                    });
                }
            }
        }
    }
    out
}

/// 查找某 chapter / volume 在 sections 列表中的 href(供 TOC 引用)。
/// 走指针相等比较 + 类型分流。
fn href_of_chapter<'a>(sections: &'a [Section<'_>], target: &Chapter) -> &'a str {
    for s in sections {
        if let SectionKind::Chapter(c) = s.kind {
            if std::ptr::eq(c, target) {
                return &s.href;
            }
        }
    }
    "" // 不应当发生:sections 是从同一棵 entries 构造的
}

fn href_of_volume<'a>(sections: &'a [Section<'_>], target: &Volume) -> &'a str {
    for s in sections {
        if let SectionKind::Volume(v) = s.kind {
            if std::ptr::eq(v, target) {
                return &s.href;
            }
        }
    }
    ""
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn generate_uid(book: &Book) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // 不是 RFC 4122 真 UUID,只是对本书唯一的不透明标识。够 EPUB reader 用。
    format!(
        "urn:uuid:endpoint-{:032x}-{:x}",
        nanos,
        simple_hash(&book.metadata.title) ^ simple_hash(&book.metadata.author)
    )
}

fn simple_hash(s: &str) -> u64 {
    // FNV-1a 64-bit。够稳定、零依赖。
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn format_modified() -> String {
    // EPUB 3 要求 ISO 8601 UTC,精度到秒,如 2026-05-23T10:00:00Z。
    // 阶段零不引入 chrono,手工算一个粗略值即可:reader 不会卡这个字段。
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, s)
}

fn epoch_to_ymdhms(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = (secs % 60) as u32;
    secs /= 60;
    let mi = (secs % 60) as u32;
    secs /= 60;
    let h = (secs % 24) as u32;
    secs /= 24;
    // days since 1970-01-01
    let mut days = secs as i64;
    let mut y: i64 = 1970;
    loop {
        let dy = if is_leap(y) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 0usize;
    while mo < 12 && days >= month_days[mo] {
        days -= month_days[mo];
        mo += 1;
    }
    (y as u32, (mo + 1) as u32, (days + 1) as u32, h, mi, s)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ==================== XHTML 生成 ====================

fn cover_xhtml(mime: CoverMime) -> String {
    let src = format!("cover.{}", mime.extension());
    // CSS 大括号在 format! 里必须用 {{ / }} 转义。
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN" lang="zh-CN">
<head>
  <title>封面</title>
  <meta charset="utf-8"/>
  <style type="text/css">
    @page {{ padding: 0; margin: 0; }}
    body {{ margin: 0; padding: 0; text-align: center; }}
    img {{ height: 100%; max-width: 100%; }}
  </style>
</head>
<body>
  <div><img src="{src}" alt="封面"/></div>
</body>
</html>
"#,
        src = src,
    )
}

fn chapter_xhtml(ch: &Chapter) -> String {
    let title = xml_escape(&ch.title);
    let mut paras = String::new();
    for p in &ch.paragraphs {
        paras.push_str("  <p>");
        paras.push_str(&xml_escape(p.as_str()));
        paras.push_str("</p>\n");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN" lang="zh-CN">
<head>
  <title>{title}</title>
  <meta charset="utf-8"/>
  <link rel="stylesheet" type="text/css" href="styles.css"/>
</head>
<body>
  <h1>{title}</h1>
{paras}</body>
</html>
"#,
        title = title,
        paras = paras
    )
}

fn volume_xhtml(v: &Volume) -> String {
    let title = xml_escape(&v.title);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN" lang="zh-CN">
<head>
  <title>{title}</title>
  <meta charset="utf-8"/>
  <link rel="stylesheet" type="text/css" href="styles.css"/>
</head>
<body>
  <h1 class="volume-title">{title}</h1>
</body>
</html>
"#,
        title = title
    )
}

fn nav_xhtml(entries: &[BookEntry], sections: &[Section<'_>]) -> String {
    let mut items = String::new();
    for e in entries {
        match e {
            BookEntry::Chapter(c) => {
                let href = href_of_chapter(sections, c);
                items.push_str(&format!(
                    "      <li><a href=\"{}\">{}</a></li>\n",
                    href,
                    xml_escape(&c.title)
                ));
            }
            BookEntry::Volume(v) => {
                let vhref = href_of_volume(sections, v);
                items.push_str(&format!(
                    "      <li><a href=\"{}\">{}</a>\n",
                    vhref,
                    xml_escape(&v.title)
                ));
                if !v.chapters.is_empty() {
                    items.push_str("        <ol>\n");
                    for c in &v.chapters {
                        let chref = href_of_chapter(sections, c);
                        items.push_str(&format!(
                            "          <li><a href=\"{}\">{}</a></li>\n",
                            chref,
                            xml_escape(&c.title)
                        ));
                    }
                    items.push_str("        </ol>\n");
                }
                items.push_str("      </li>\n");
            }
        }
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="zh-CN" lang="zh-CN">
<head>
  <title>目录</title>
  <meta charset="utf-8"/>
  <link rel="stylesheet" type="text/css" href="styles.css"/>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>目录</h1>
    <ol>
{items}    </ol>
  </nav>
</body>
</html>
"#,
        items = items
    )
}

fn toc_ncx(book: &Book, entries: &[BookEntry], sections: &[Section<'_>], uid: &str) -> String {
    let title = xml_escape(&book.metadata.title);
    let mut nav_points = String::new();
    let mut play_order = 0u32;
    let mut depth = 1u32;
    for e in entries {
        match e {
            BookEntry::Chapter(c) => {
                play_order += 1;
                let href = href_of_chapter(sections, c);
                nav_points.push_str(&format!(
                    r#"    <navPoint id="navPoint-{n}" playOrder="{n}">
      <navLabel><text>{t}</text></navLabel>
      <content src="{href}"/>
    </navPoint>
"#,
                    n = play_order,
                    t = xml_escape(&c.title),
                    href = href
                ));
            }
            BookEntry::Volume(v) => {
                play_order += 1;
                let vhref = href_of_volume(sections, v);
                let vnum = play_order;
                let mut inner = String::new();
                for c in &v.chapters {
                    depth = 2;
                    play_order += 1;
                    let chref = href_of_chapter(sections, c);
                    inner.push_str(&format!(
                        r#"      <navPoint id="navPoint-{n}" playOrder="{n}">
        <navLabel><text>{t}</text></navLabel>
        <content src="{href}"/>
      </navPoint>
"#,
                        n = play_order,
                        t = xml_escape(&c.title),
                        href = chref
                    ));
                }
                nav_points.push_str(&format!(
                    r#"    <navPoint id="navPoint-{n}" playOrder="{n}">
      <navLabel><text>{t}</text></navLabel>
      <content src="{href}"/>
{inner}    </navPoint>
"#,
                    n = vnum,
                    t = xml_escape(&v.title),
                    href = vhref,
                    inner = inner
                ));
            }
        }
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1" xml:lang="zh-CN">
  <head>
    <meta name="dtb:uid" content="{uid}"/>
    <meta name="dtb:depth" content="{depth}"/>
    <meta name="dtb:totalPageCount" content="0"/>
    <meta name="dtb:maxPageNumber" content="0"/>
  </head>
  <docTitle><text>{title}</text></docTitle>
  <navMap>
{nav_points}  </navMap>
</ncx>
"#,
        uid = xml_escape(uid),
        title = title,
        depth = depth,
        nav_points = nav_points,
    )
}

fn content_opf(
    book: &Book,
    sections: &[Section<'_>],
    uid: &str,
    modified: &str,
    cover_mime: CoverMime,
    has_cover: bool,
) -> String {
    let title = xml_escape(&book.metadata.title);
    let author = xml_escape(&book.metadata.author);
    let language = xml_escape(&book.metadata.language);

    let mut manifest = String::new();
    let mut spine = String::new();
    for s in sections {
        manifest.push_str(&format!(
            "    <item id=\"{id}\" href=\"{href}\" media-type=\"application/xhtml+xml\"/>\n",
            id = s.id,
            href = s.href,
        ));
        spine.push_str(&format!("    <itemref idref=\"{}\"/>\n", s.id));
    }

    // 封面:manifest 加 cover-image item + cover XHTML item;spine 首位加封面页
    let cover_manifest = if has_cover {
        format!(
            "    <item id=\"cover-image\" href=\"cover.{ext}\" media-type=\"{mime}\" properties=\"cover-image\"/>\n\
                 <item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            ext = cover_mime.extension(),
            mime = cover_mime.as_str(),
        )
    } else {
        String::new()
    };

    let cover_spine = if has_cover {
        "    <itemref idref=\"cover\" linear=\"yes\"/>\n".to_string()
    } else {
        String::new()
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid" xml:lang="{language}">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">{uid}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:language>{language}</dc:language>
    <meta property="dcterms:modified">{modified}</meta>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="css" href="styles.css" media-type="text/css"/>
{cover_manifest}{manifest}  </manifest>
  <spine toc="ncx">
{cover_spine}{spine}  </spine>
</package>
"#,
        uid = xml_escape(uid),
        title = title,
        author = author,
        language = language,
        modified = modified,
        cover_manifest = cover_manifest,
        manifest = manifest,
        cover_spine = cover_spine,
        spine = spine,
    )
}

const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;

// ==================== 测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{BookEntry, ChapterOrigin, Metadata, Paragraph, Span};
    use std::io::Read;

    fn sample_book() -> Book {
        Book {
            metadata: Metadata::new("测试书 & 标点", "测试作者"),
            entries: vec![
                BookEntry::Chapter(Chapter {
                    title: "第一章 起".into(),
                    paragraphs: vec![Paragraph::new("正文1"), Paragraph::new("正文 < 含特殊字符 >")],
                    heading_span: Span::new(0, 6),
                    body_span: Span::new(7, 20),
                    origin: ChapterOrigin::RegexMatch,
                    matched_rule_id: Some("builtin-chapter-cn-zhang".into()),
                }),
                BookEntry::Chapter(Chapter {
                    title: "第二章 承".into(),
                    paragraphs: vec![Paragraph::new("第二章的内容")],
                    heading_span: Span::new(20, 26),
                    body_span: Span::new(27, 40),
                    origin: ChapterOrigin::RegexMatch,
                    matched_rule_id: Some("builtin-chapter-cn-zhang".into()),
                }),
            ],
        }
    }

    #[test]
    fn writes_a_valid_zip_with_mimetype_first_and_stored() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.epub");
        build(&sample_book(), &out, &EpubOptions::default()).unwrap();

        let f = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();

        // mimetype 必须是第一个 entry,且为 Stored。
        let first = archive.by_index(0).unwrap();
        assert_eq!(first.name(), "mimetype");
        assert_eq!(first.compression(), CompressionMethod::Stored);
        drop(first);

        // 必需的文件全部存在。
        let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
        for needed in [
            "mimetype",
            "META-INF/container.xml",
            "OEBPS/content.opf",
            "OEBPS/toc.ncx",
            "OEBPS/nav.xhtml",
            "OEBPS/styles.css",
            "OEBPS/chapter_0001.xhtml",
            "OEBPS/chapter_0002.xhtml",
        ] {
            assert!(names.iter().any(|n| n == needed), "缺少 {}", needed);
        }
    }

    #[test]
    fn mimetype_content_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.epub");
        build(&sample_book(), &out, &EpubOptions::default()).unwrap();

        let f = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut mt = archive.by_name("mimetype").unwrap();
        let mut s = String::new();
        mt.read_to_string(&mut s).unwrap();
        assert_eq!(s, "application/epub+zip");
    }

    fn book_with_volumes() -> Book {
        use crate::domain::Volume;
        Book {
            metadata: Metadata::new("分卷书", "作者"),
            entries: vec![
                BookEntry::Chapter(Chapter {
                    title: "楔子".into(),
                    paragraphs: vec![Paragraph::new("楔子内容")],
                    heading_span: Span::new(0, 6),
                    body_span: Span::new(7, 20),
                    origin: ChapterOrigin::RegexMatch,
                    matched_rule_id: Some("builtin-chapter-prologue".into()),
                }),
                BookEntry::Volume(Volume {
                    title: "第一部 风起".into(),
                    chapters: vec![
                        Chapter {
                            title: "第一章 起".into(),
                            paragraphs: vec![Paragraph::new("正文1")],
                            heading_span: Span::new(30, 36),
                            body_span: Span::new(37, 50),
                            origin: ChapterOrigin::RegexMatch,
                            matched_rule_id: Some("builtin-chapter-cn-zhang".into()),
                        },
                        Chapter {
                            title: "第二章 承".into(),
                            paragraphs: vec![Paragraph::new("正文2")],
                            heading_span: Span::new(50, 56),
                            body_span: Span::new(57, 70),
                            origin: ChapterOrigin::RegexMatch,
                            matched_rule_id: Some("builtin-chapter-cn-zhang".into()),
                        },
                    ],
                    heading_span: Span::new(20, 30),
                    origin: ChapterOrigin::RegexMatch,
                    matched_rule_id: Some("builtin-volume-cn-bu".into()),
                }),
            ],
        }
    }

    #[test]
    fn volumes_get_their_own_xhtml_and_appear_in_toc() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("with_volumes.epub");
        build(&book_with_volumes(), &out, &EpubOptions::default()).unwrap();

        let f = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();

        // 卷分隔页应当独立生成
        assert!(
            names.iter().any(|n| n == "OEBPS/volume_0001.xhtml"),
            "缺少 volume_0001.xhtml,卷标题会丢失:实际文件清单 {:?}",
            names
        );

        // 卷标题文本应出现在卷分隔页 xhtml 里
        let mut volp = archive.by_name("OEBPS/volume_0001.xhtml").unwrap();
        let mut s = String::new();
        volp.read_to_string(&mut s).unwrap();
        assert!(s.contains("第一部 风起"), "卷分隔页未包含卷标题: {}", s);
        assert!(s.contains("volume-title"), "卷分隔页未使用 .volume-title 样式");
        drop(volp);

        // nav.xhtml 应当包含卷标题与嵌套章节链接
        let mut nav = archive.by_name("OEBPS/nav.xhtml").unwrap();
        let mut nav_s = String::new();
        nav.read_to_string(&mut nav_s).unwrap();
        assert!(nav_s.contains("第一部 风起"), "nav 缺少卷标题");
        assert!(nav_s.contains("volume_0001.xhtml"));
        assert!(nav_s.contains("楔子"));
        drop(nav);

        // toc.ncx 应当用嵌套 navPoint 表达卷-章层级
        let mut ncx_s = String::new();
        {
            let mut ncx = archive.by_name("OEBPS/toc.ncx").unwrap();
            ncx.read_to_string(&mut ncx_s).unwrap();
        }
        assert!(ncx_s.contains("第一部 风起"), "ncx 缺少卷标题");
        // depth 应为 2(嵌套层级)
        assert!(ncx_s.contains(r#"name="dtb:depth" content="2""#));

        // spine 应当含卷分隔页
        let mut opf_s = String::new();
        {
            let mut opf = archive.by_name("OEBPS/content.opf").unwrap();
            opf.read_to_string(&mut opf_s).unwrap();
        }
        assert!(opf_s.contains("volume_0001.xhtml"));
        assert!(opf_s.contains(r#"idref="vol0001""#));
    }

    #[test]
    fn opf_contains_escaped_title_and_all_chapters() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.epub");
        build(&sample_book(), &out, &EpubOptions::default()).unwrap();

        let f = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut opf = archive.by_name("OEBPS/content.opf").unwrap();
        let mut s = String::new();
        opf.read_to_string(&mut s).unwrap();
        assert!(s.contains("测试书 &amp; 标点"), "title 未转义");
        assert!(s.contains("chapter_0001.xhtml"));
        assert!(s.contains("chapter_0002.xhtml"));
    }

    #[test]
    fn cover_image_appears_in_manifest_and_spine_when_provided() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("with_cover.epub");
        // 最小 JPEG 占位字节(不要求图片有效,只验证 epub 结构)
        let fake_jpeg: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let opts = EpubOptions {
            cover: Some(&fake_jpeg),
            cover_mime: CoverMime::Jpeg,
            ..EpubOptions::default()
        };
        build(&sample_book(), &out, &opts).unwrap();

        let f = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();

        // 封面图片 + 封面 XHTML 都应存在
        assert!(names.iter().any(|n| n == "OEBPS/cover.jpg"), "缺少 cover.jpg");
        assert!(names.iter().any(|n| n == "OEBPS/cover.xhtml"), "缺少 cover.xhtml");

        // OPF 应含 cover-image property + cover xhtml item
        let mut opf_s = String::new();
        {
            let mut opf = archive.by_name("OEBPS/content.opf").unwrap();
            opf.read_to_string(&mut opf_s).unwrap();
        }
        assert!(opf_s.contains("properties=\"cover-image\""), "OPF 缺少 cover-image property");
        assert!(opf_s.contains("cover.jpg"), "OPF manifest 缺少 cover.jpg");
        assert!(opf_s.contains("cover.xhtml"), "OPF manifest 缺少 cover.xhtml");

        // spine 首位应是封面页
        assert!(opf_s.contains(r#"idref="cover""#), "spine 缺少封面 itemref");
    }

    #[test]
    fn css_override_replaces_default_stylesheet() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("custom_css.epub");
        let my_css = "body { font-size: 20px; color: #111; }";
        let opts = EpubOptions {
            css_override: Some(my_css),
            ..EpubOptions::default()
        };
        build(&sample_book(), &out, &opts).unwrap();

        let f = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut css_entry = archive.by_name("OEBPS/styles.css").unwrap();
        let mut css_content = String::new();
        css_entry.read_to_string(&mut css_content).unwrap();
        assert_eq!(css_content, my_css, "styles.css 内容应等于传入的 css_override");
    }

    #[test]
    fn no_cover_epub_has_no_cover_files() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("no_cover.epub");
        build(&sample_book(), &out, &EpubOptions::default()).unwrap();

        let f = File::open(&out).unwrap();
        let archive = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
        assert!(
            !names.iter().any(|n| n.contains("cover")),
            "无封面时不应有任何 cover 文件,实际: {:?}",
            names
        );
    }
}
