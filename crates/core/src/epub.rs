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

use crate::domain::{Book, BookEntry, Chapter};

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

    let chapters = flatten_chapters(&book.entries);
    let uid = generate_uid(book);
    let modified = format_modified();

    for (idx, ch) in chapters.iter().enumerate() {
        zw.start_file(format!("OEBPS/{}", chapter_href(idx)), deflated)?;
        zw.write_all(chapter_xhtml(ch).as_bytes())?;
    }

    zw.start_file("OEBPS/nav.xhtml", deflated)?;
    zw.write_all(nav_xhtml(&chapters).as_bytes())?;

    zw.start_file("OEBPS/toc.ncx", deflated)?;
    zw.write_all(toc_ncx(book, &chapters, &uid).as_bytes())?;

    zw.start_file("OEBPS/content.opf", deflated)?;
    zw.write_all(content_opf(book, &chapters, &uid, &modified).as_bytes())?;

    zw.finish()?;
    Ok(())
}

fn flatten_chapters(entries: &[BookEntry]) -> Vec<&Chapter> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            BookEntry::Chapter(c) => out.push(c),
            BookEntry::Volume(v) => out.extend(v.chapters.iter()),
        }
    }
    out
}

fn chapter_href(idx: usize) -> String {
    format!("chapter_{:04}.xhtml", idx + 1)
}

fn chapter_id(idx: usize) -> String {
    format!("ch{:04}", idx + 1)
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

fn nav_xhtml(chapters: &[&Chapter]) -> String {
    let mut items = String::new();
    for (i, ch) in chapters.iter().enumerate() {
        items.push_str(&format!(
            "      <li><a href=\"{}\">{}</a></li>\n",
            chapter_href(i),
            xml_escape(&ch.title)
        ));
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

fn toc_ncx(book: &Book, chapters: &[&Chapter], uid: &str) -> String {
    let title = xml_escape(&book.metadata.title);
    let mut nav_points = String::new();
    for (i, ch) in chapters.iter().enumerate() {
        nav_points.push_str(&format!(
            r#"    <navPoint id="navPoint-{n}" playOrder="{n}">
      <navLabel><text>{t}</text></navLabel>
      <content src="{href}"/>
    </navPoint>
"#,
            n = i + 1,
            t = xml_escape(&ch.title),
            href = chapter_href(i)
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1" xml:lang="zh-CN">
  <head>
    <meta name="dtb:uid" content="{uid}"/>
    <meta name="dtb:depth" content="1"/>
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
        nav_points = nav_points,
    )
}

fn content_opf(book: &Book, chapters: &[&Chapter], uid: &str, modified: &str) -> String {
    let title = xml_escape(&book.metadata.title);
    let author = xml_escape(&book.metadata.author);
    let language = xml_escape(&book.metadata.language);

    let mut manifest = String::new();
    let mut spine = String::new();
    for i in 0..chapters.len() {
        manifest.push_str(&format!(
            "    <item id=\"{id}\" href=\"{href}\" media-type=\"application/xhtml+xml\"/>\n",
            id = chapter_id(i),
            href = chapter_href(i),
        ));
        spine.push_str(&format!("    <itemref idref=\"{}\"/>\n", chapter_id(i)));
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
    use crate::domain::{BookEntry, ChapterOrigin, Metadata, Paragraph};
    use std::io::Read;

    fn sample_book() -> Book {
        Book {
            metadata: Metadata::new("测试书 & 标点", "测试作者"),
            entries: vec![
                BookEntry::Chapter(Chapter {
                    title: "第一章 起".into(),
                    paragraphs: vec![Paragraph::new("正文1"), Paragraph::new("正文 < 含特殊字符 >")],
                    source_start: 0,
                    source_end: 10,
                    origin: ChapterOrigin::RegexMatch,
                }),
                BookEntry::Chapter(Chapter {
                    title: "第二章 承".into(),
                    paragraphs: vec![Paragraph::new("第二章的内容")],
                    source_start: 10,
                    source_end: 20,
                    origin: ChapterOrigin::RegexMatch,
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
