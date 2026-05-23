//! EPUB 3 构建。
//!
//! 阶段零最小可用版本:输入 `Book`,输出符合规范的 .epub 文件。包含 OPF、NCX、EPUB 3 nav、
//! 章节 XHTML、写死的默认 CSS。**不嵌入字体**——字体嵌入归到后续阶段。
//!
//! 卷的支持是占位的:阶段零的章节切分不会产出卷,但本模块对 `BookEntry::Volume` 做了展平,
//! 后续阶段无需重写此处即可让 spine 工作(目录展示卷层级则留待后续)。

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;
use zip::write::{FileOptions, ZipWriter};
use zip::CompressionMethod;

use crate::domain::{Book, BookEntry, Chapter, Volume};

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

pub fn build(book: &Book, out_path: &Path) -> Result<(), EpubError> {
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

    zw.start_file("OEBPS/styles.css", deflated)?;
    zw.write_all(DEFAULT_CSS.as_bytes())?;

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
    zw.write_all(content_opf(book, &sections, &uid, &modified).as_bytes())?;

    zw.finish()?;
    Ok(())
}

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

fn content_opf(book: &Book, sections: &[Section<'_>], uid: &str, modified: &str) -> String {
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
{manifest}  </manifest>
  <spine toc="ncx">
{spine}  </spine>
</package>
"#,
        uid = xml_escape(uid),
        title = title,
        author = author,
        language = language,
        modified = modified,
        manifest = manifest,
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
        build(&sample_book(), &out).unwrap();

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
        build(&sample_book(), &out).unwrap();

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
        build(&book_with_volumes(), &out).unwrap();

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
        build(&sample_book(), &out).unwrap();

        let f = File::open(&out).unwrap();
        let mut archive = zip::ZipArchive::new(f).unwrap();
        let mut opf = archive.by_name("OEBPS/content.opf").unwrap();
        let mut s = String::new();
        opf.read_to_string(&mut s).unwrap();
        assert!(s.contains("测试书 &amp; 标点"), "title 未转义");
        assert!(s.contains("chapter_0001.xhtml"));
        assert!(s.contains("chapter_0002.xhtml"));
    }
}
