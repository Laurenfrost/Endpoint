//! Endpoint 核心库。
//!
//! 纯 Rust 实现的「txt → epub」转换管线。任何依赖 Tauri 或前端的东西都不应进入本 crate。
//! 阶段零(Walking Skeleton)只做最朴素的端到端链路:UTF-8 读取 → 「第X章」正则切章
//! → 生成最小可用 epub → 可选 kepubify。

pub mod chapter;
pub mod domain;
pub mod epub;
pub mod kepubify;
pub mod reader;

use std::path::{Path, PathBuf};

use thiserror::Error;

pub use crate::domain::Metadata;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Read(#[from] reader::ReadError),
    #[error(transparent)]
    Chapter(#[from] chapter::ChapterError),
    #[error(transparent)]
    Epub(#[from] epub::EpubError),
    #[error(transparent)]
    Kepubify(#[from] kepubify::KepubifyError),
}

/// 顶层入口:把一个 UTF-8 txt 文件转成 epub,可选再调用 kepubify 生成 .kepub.epub。
///
/// 返回最终产物路径(若启用了 kepubify,返回 kepub 的路径;否则返回 epub 路径)。
pub fn convert(
    input_txt: &Path,
    output_epub: &Path,
    metadata: Metadata,
    kepubify_path: Option<&Path>,
) -> Result<PathBuf, CoreError> {
    let text = reader::read_utf8(input_txt)?;
    let book = chapter::split(&text, metadata)?;
    epub::build(&book, output_epub)?;

    if let Some(kepubify) = kepubify_path {
        let out_dir = output_epub
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let kepub = kepubify::run(kepubify, output_epub, &out_dir)?;
        return Ok(kepub);
    }

    Ok(output_epub.to_path_buf())
}
