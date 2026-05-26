//! 编码探测与解码。
//!
//! 阶段零只支持 UTF-8;阶段一扩展到中文网文常见的 GBK / GB18030 / UTF-8(含 BOM) /
//! UTF-16(LE/BE)。
//!
//! # 探测流程
//!
//! 1. **BOM sniff(ground truth)**:UTF-8 BOM (`EF BB BF`)、UTF-16 LE BOM (`FF FE`)、
//!    UTF-16 BE BOM (`FE FF`)。命中即直接解码,**不**走启发式探测——BOM 是确定性信号。
//! 2. **手动覆盖**:调用方传入显式的编码标签时跳过自动探测;支持 IANA 标签或常见别名
//!    (如 `"GBK"`、`"GB18030"`、`"UTF-8"`、`"UTF-16LE"`)。
//! 3. **chardetng 自动探测**:Mozilla `chardetng` crate,基于 Web 内容统计、对 CJK 编码
//!    区分较准。GB18030 是 GBK 的严格超集,`encoding_rs` 把二者统一映射到 GB18030 解码器,
//!    因此即使探测器报 GBK 我们也用 GB18030 解码,确保覆盖罕用字。
//!
//! # 选型理由
//!
//! - `encoding_rs`:WHATWG 规范实现,事实标准,GBK 解码经过 web 实战验证。
//! - `chardetng`:Mozilla chardet 继任者,持续维护,专为中文 Web 内容设计。备选方案
//!   `chardet` 已停更、`encoding`(去掉 _rs)依赖老旧。

use std::fs;
use std::io;
use std::path::Path;

use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum EncodingError {
    #[error("无法读取文件 {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("无法识别编码标签 `{0}`")]
    UnknownLabel(String),
    #[error("文件以 `{label}` 解码时遇到非法字节序列;请尝试手动指定其他编码")]
    Malformed { label: String },
}

const BOM_UTF8: &[u8] = &[0xEF, 0xBB, 0xBF];
const BOM_UTF16_LE: &[u8] = &[0xFF, 0xFE];
const BOM_UTF16_BE: &[u8] = &[0xFE, 0xFF];

/// 解码字节并返回(文本, 实际使用的编码标签)。
///
/// - `override_label = Some("GBK")`:跳过自动探测,直接以 GBK 解码。
/// - `override_label = None`:先看 BOM,再用 chardetng 探测。
///
/// 阶段一对 malformed 字节采取"严格失败"策略:`encoding_rs` 报告 had_errors → 返回
/// [`EncodingError::Malformed`]。UI 拿到错误后可以提示用户手动覆盖编码。
pub fn decode(
    bytes: &[u8],
    override_label: Option<&str>,
) -> Result<(String, String), EncodingError> {
    let (encoding, payload) = pick_encoding(bytes, override_label)?;
    let (cow, _, had_errors) = encoding.decode(payload);
    if had_errors {
        warn!(label = encoding.name(), "解码遇到非法字节序列");
        return Err(EncodingError::Malformed {
            label: encoding.name().to_string(),
        });
    }
    Ok((cow.into_owned(), encoding.name().to_string()))
}

/// 从磁盘读取文件,再交给 [`decode`]。
pub fn read_file(
    path: &Path,
    override_label: Option<&str>,
) -> Result<(String, String), EncodingError> {
    let bytes = fs::read(path).map_err(|e| EncodingError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    decode(&bytes, override_label)
}

fn pick_encoding<'a>(
    bytes: &'a [u8],
    override_label: Option<&str>,
) -> Result<(&'static Encoding, &'a [u8]), EncodingError> {
    if let Some(label) = override_label {
        let enc = Encoding::for_label(label.as_bytes())
            .ok_or_else(|| EncodingError::UnknownLabel(label.to_string()))?;
        debug!(label, resolved = enc.name(), "手动覆盖编码");
        return Ok((enc, strip_bom_for(enc, bytes)));
    }

    if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
        debug!(encoding = enc.name(), bom_len, "BOM 探测命中");
        return Ok((enc, &bytes[bom_len..]));
    }

    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let guessed = detector.guess(None, true);

    let enc = if guessed == encoding_rs::WINDOWS_1252 && bytes.iter().all(|b| *b < 0x80) {
        encoding_rs::UTF_8
    } else if guessed == encoding_rs::GBK {
        encoding_rs::GB18030
    } else {
        guessed
    };
    debug!(
        guessed = guessed.name(),
        chosen = enc.name(),
        "chardetng 启发式探测"
    );

    Ok((enc, strip_bom_for(enc, bytes)))
}

fn strip_bom_for<'a>(enc: &'static Encoding, bytes: &'a [u8]) -> &'a [u8] {
    if enc == encoding_rs::UTF_8 && bytes.starts_with(BOM_UTF8) {
        &bytes[BOM_UTF8.len()..]
    } else if enc == encoding_rs::UTF_16LE && bytes.starts_with(BOM_UTF16_LE) {
        &bytes[BOM_UTF16_LE.len()..]
    } else if enc == encoding_rs::UTF_16BE && bytes.starts_with(BOM_UTF16_BE) {
        &bytes[BOM_UTF16_BE.len()..]
    } else {
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_with(content: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn decodes_plain_utf8() {
        let (text, enc) = decode("第一章\n正文".as_bytes(), None).unwrap();
        assert_eq!(text, "第一章\n正文");
        assert_eq!(enc, "UTF-8");
    }

    #[test]
    fn strips_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("第一章".as_bytes());
        let (text, enc) = decode(&bytes, None).unwrap();
        assert_eq!(text, "第一章");
        assert_eq!(enc, "UTF-8");
    }

    #[test]
    fn decodes_gbk_bytes_via_detection() {
        // GBK 编码的样本文本(扩长以增大 chardetng 置信度)
        let (bytes, _, _) = encoding_rs::GBK
            .encode("你好,世界。这是一段用 GBK 编码的中文网络小说样本,用来给探测器足够的统计依据。");
        let (decoded, label) = decode(bytes.as_ref(), None).unwrap();
        assert_eq!(
            decoded,
            "你好,世界。这是一段用 GBK 编码的中文网络小说样本,用来给探测器足够的统计依据。"
        );
        // chardetng 可能报 GBK,但我们统一升到 GB18030;encoding_rs 的 name() 是小写。
        let lower = label.to_ascii_lowercase();
        assert!(lower == "gb18030" || lower == "gbk", "got {}", label);
    }

    #[test]
    fn manual_override_gbk_works() {
        let (bytes, _, _) = encoding_rs::GBK.encode("强制 GBK 解码");
        let (decoded, label) = decode(bytes.as_ref(), Some("GBK")).unwrap();
        assert_eq!(decoded, "强制 GBK 解码");
        assert_eq!(label, "GBK");
    }

    #[test]
    fn manual_override_gb18030_works() {
        let (bytes, _, _) = encoding_rs::GB18030.encode("GB18030 显式覆盖");
        let (decoded, label) = decode(bytes.as_ref(), Some("GB18030")).unwrap();
        assert_eq!(decoded, "GB18030 显式覆盖");
        assert_eq!(label, "gb18030");
    }

    fn encode_utf16(s: &str, little_endian: bool) -> Vec<u8> {
        // encoding_rs 的 UTF_16LE/UTF_16BE 是 decode-only(`encode()` 会输出 UTF-8),
        // 因此 UTF-16 测试样本需要手工构造。
        let mut out = Vec::with_capacity(s.len() * 2);
        for u in s.encode_utf16() {
            if little_endian {
                out.push(u as u8);
                out.push((u >> 8) as u8);
            } else {
                out.push((u >> 8) as u8);
                out.push(u as u8);
            }
        }
        out
    }

    #[test]
    fn decodes_utf16_le_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&encode_utf16("UTF-16 LE 测试", true));
        let (text, label) = decode(&bytes, None).unwrap();
        assert_eq!(text, "UTF-16 LE 测试");
        assert!(label.eq_ignore_ascii_case("UTF-16LE"), "got {}", label);
    }

    #[test]
    fn decodes_utf16_be_with_bom() {
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend_from_slice(&encode_utf16("UTF-16 BE 测试", false));
        let (text, label) = decode(&bytes, None).unwrap();
        assert_eq!(text, "UTF-16 BE 测试");
        assert!(label.eq_ignore_ascii_case("UTF-16BE"), "got {}", label);
    }

    #[test]
    fn unknown_override_label_errors() {
        let err = decode(b"hello", Some("not-a-real-encoding")).unwrap_err();
        assert!(matches!(err, EncodingError::UnknownLabel(_)));
    }

    #[test]
    fn manual_override_strips_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("abc".as_bytes());
        let (text, _) = decode(&bytes, Some("UTF-8")).unwrap();
        assert_eq!(text, "abc");
    }

    #[test]
    fn read_file_round_trip() {
        let (bytes, _, _) = encoding_rs::GBK.encode("从文件读取的 GBK 文本");
        let f = tmp_with(bytes.as_ref());
        let (text, _) = read_file(f.path(), Some("GBK")).unwrap();
        assert_eq!(text, "从文件读取的 GBK 文本");
    }

    #[test]
    fn malformed_utf8_with_explicit_override_reports_error() {
        // 非法的 UTF-8 续字节
        let bytes: &[u8] = &[0xC0, 0xC0, 0xC0];
        let err = decode(bytes, Some("UTF-8")).unwrap_err();
        assert!(matches!(err, EncodingError::Malformed { .. }));
    }
}
