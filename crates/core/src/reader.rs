//! 文本读取。
//!
//! 阶段零只支持 UTF-8。非 UTF-8 字节序列直接报错——编码自动探测留给阶段一。
//! 顺带剥掉 UTF-8 BOM(`EF BB BF`),否则后续正则匹配会被前导不可见字符干扰。

use std::fs;
use std::io;
use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("无法读取文件 {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("文件 {path} 不是合法的 UTF-8 文本;阶段零仅支持 UTF-8(后续阶段会加编码探测)")]
    NotUtf8 { path: String },
}

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

pub fn read_utf8(path: &Path) -> Result<String, ReadError> {
    let bytes = fs::read(path).map_err(|e| ReadError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let bytes = bytes.strip_prefix(BOM).unwrap_or(&bytes);

    String::from_utf8(bytes.to_vec()).map_err(|_| ReadError::NotUtf8 {
        path: path.display().to_string(),
    })
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
    fn reads_plain_utf8() {
        let f = tmp_with("第一章\n正文".as_bytes());
        let s = read_utf8(f.path()).unwrap();
        assert_eq!(s, "第一章\n正文");
    }

    #[test]
    fn strips_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("第一章".as_bytes());
        let f = tmp_with(&bytes);
        let s = read_utf8(f.path()).unwrap();
        assert_eq!(s, "第一章");
    }

    #[test]
    fn rejects_non_utf8() {
        // GBK 编码的「你好」
        let f = tmp_with(&[0xC4, 0xE3, 0xBA, 0xC3]);
        let err = read_utf8(f.path()).unwrap_err();
        assert!(matches!(err, ReadError::NotUtf8 { .. }));
    }
}
