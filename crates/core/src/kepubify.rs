//! 外部 kepubify 进程封装。
//!
//! 阶段零按 CLAUDE.md 二节决定:**不**做原生集成,直接调外部 CLI(pgaskin/kepubify 的
//! Windows 预编译二进制)。
//!
//! 用法:`kepubify -o <output_dir> <input.epub>`。当 `-o` 为目录、且目录下已经存在与
//! 输入同名的 epub 时,kepubify 会把输出命名为 `<stem>_converted.kepub.epub` 避免冲突。
//! 阶段零接受这个默认行为(无需指定精确文件名),直接按 `_converted` 后缀去找产物即可。

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum KepubifyError {
    #[error("无法启动 kepubify ({path}): {source}")]
    Spawn {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("kepubify 进程返回非零退出码 ({code:?}). stderr: {stderr}")]
    NonZero { code: Option<i32>, stderr: String },
    #[error("kepubify 执行后未在 {dir} 找到预期的输出 {expected}")]
    OutputMissing { dir: String, expected: String },
    #[error("输入路径 {0} 没有合法文件名")]
    InvalidInput(String),
}

/// 调用 kepubify。返回产出的 `<stem>_converted.kepub.epub` 路径。
///
/// 阶段零做最朴素的调用:同步 + 阻塞,无进度。
pub fn run(
    kepubify_exe: &Path,
    input_epub: &Path,
    output_dir: &Path,
) -> Result<PathBuf, KepubifyError> {
    let output = Command::new(kepubify_exe)
        .arg("-o")
        .arg(output_dir)
        .arg(input_epub)
        .output()
        .map_err(|e| KepubifyError::Spawn {
            path: kepubify_exe.display().to_string(),
            source: e,
        })?;

    if !output.status.success() {
        return Err(KepubifyError::NonZero {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let stem = input_epub
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| KepubifyError::InvalidInput(input_epub.display().to_string()))?;
    let expected = output_dir.join(format!("{}_converted.kepub.epub", stem));

    if !expected.exists() {
        return Err(KepubifyError::OutputMissing {
            dir: output_dir.display().to_string(),
            expected: expected.display().to_string(),
        });
    }
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_spawn_error_when_executable_missing() {
        let nope = Path::new("Z:/definitely/not/a/real/kepubify.exe");
        let input = Path::new("input.epub");
        let dir = Path::new(".");
        let err = run(nope, input, dir).unwrap_err();
        assert!(matches!(err, KepubifyError::Spawn { .. }));
    }
}
