//! Endpoint 核心库。
//!
//! 纯 Rust 实现的「txt → epub」转换管线。任何依赖 Tauri 或前端的东西都不应进入本 crate。
//!
//! # 阶段一新增能力
//! - **编码探测**:[`encoding`] 模块覆盖 UTF-8 / UTF-8 BOM / GBK / GB18030 / UTF-16(LE/BE),
//!   支持手动覆盖。
//! - **文本清洗**:[`cleaning`] 模块产出**坐标标注**而非已清洗的字符串。
//! - **规则化章节解析**:[`chapter`] 模块消费 [`rules::RuleSet`],支持卷章两级层级。
//! - **富标注契约冻结**:见 [`domain`] 模块顶部 doc comment。
//!
//! # 三层管线 API
//!
//! 1. [`run_pipeline`]:从字节出发跑完编码 + 清洗 + 章节解析,产出 [`PipelineOutput`]
//!    (含富标注,供阶段二的界面消费)。**不**写 EPUB。
//! 2. [`build_epub_from`]:把 [`PipelineOutput`] 写为 EPUB,可选 kepubify。
//! 3. [`convert`]:阶段零兼容入口——读文件 → 跑管线 → 写 EPUB → 可选 kepubify,
//!    一站式调用。桥接层仍使用此入口,无需感知 [`PipelineOutput`]。

pub mod chapter;
pub mod cleaning;
pub mod domain;
pub mod encoding;
pub mod epub;
pub mod kepubify;
pub mod rules;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use thiserror::Error;

pub use crate::domain::{Metadata, PipelineOutput};
pub use crate::rules::RuleSet;

/// 进度回传通道。核心库依赖此 trait,桥接层(Tauri)实现把回调转 `app.emit`。
///
/// 设计成 trait 而非具体类型是为了:
/// 1. 让核心库脱离 Tauri 单独编译/测试([`NoopSink`] 用于无进度场景与单测)。
/// 2. 将来若改用其他 UI(CLI、WebSocket)只换实现即可。
///
/// `stage` 枚举锁定为:`"decoding"` / `"cleaning"` / `"chapter"` / `"epub"` / `"kepubify"`。
/// 详见 `docs/stage2-design.md` 第三节进度事件冻结。
pub trait ProgressSink: Send + Sync {
    fn report(&self, stage: &str, percent: u8, detail: Option<&str>);
}

/// 不发任何进度的实现。用于阶段零的一站式 [`convert`] 与单元测试。
pub struct NoopSink;

impl ProgressSink for NoopSink {
    fn report(&self, _: &str, _: u8, _: Option<&str>) {}
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Encoding(#[from] encoding::EncodingError),
    #[error(transparent)]
    Chapter(#[from] chapter::ChapterError),
    #[error(transparent)]
    Rules(#[from] rules::RulesError),
    #[error(transparent)]
    Epub(#[from] epub::EpubError),
    #[error(transparent)]
    Kepubify(#[from] kepubify::KepubifyError),
}

/// 转换选项。所有字段都可选,缺省值取「智能默认」。
#[derive(Debug, Default, Clone)]
pub struct ConvertOptions {
    /// 显式指定输入文件编码(如 `"GBK"`、`"UTF-8"`)。`None` 表示自动探测。
    pub encoding_override: Option<String>,
    /// 自定义规则文件路径。`None` 表示仅使用内置默认规则。
    pub rules_path: Option<PathBuf>,
    /// kepubify 可执行路径。`None` 表示只输出 .epub,不做 kepub 优化。
    pub kepubify_path: Option<PathBuf>,
    /// 取消标志。v1 仅接口预留——核心库长循环里只标注 `TODO(cancel)`,**不**实际检查。
    /// 阶段二之后再实装:在 `cleaning::analyze` / `chapter::parse` 的主扫描循环里
    /// 每 N 行检查一次,若被置位则提前返回特定错误。
    pub cancel_token: Option<Arc<AtomicBool>>,
}

/// 阶段二界面会消费的入口:从字节运行完整文本管线,产出富标注。
///
/// 不读文件、不写文件——内存语义。便于核心库脱离 IO 单元测试。
///
/// `progress` 用于向上层回报阶段进度;不需要进度时传 `&NoopSink`。
pub fn run_pipeline(
    bytes: &[u8],
    metadata: Metadata,
    options: &ConvertOptions,
    progress: &dyn ProgressSink,
) -> Result<PipelineOutput, CoreError> {
    progress.report("decoding", 0, None);
    let (source_text, source_encoding) =
        encoding::decode(bytes, options.encoding_override.as_deref())?;
    progress.report("decoding", 100, Some(&source_encoding));

    progress.report("cleaning", 0, None);
    let cleaning_anns = cleaning::analyze(&source_text);
    progress.report("cleaning", 100, None);

    progress.report("chapter", 0, None);
    let rules = match &options.rules_path {
        Some(p) => {
            let mut set = RuleSet::builtin();
            set.merge(RuleSet::load_from_json(p)?);
            set
        }
        None => RuleSet::builtin(),
    };

    let book = chapter::parse(&source_text, &cleaning_anns, &rules, metadata)?;
    progress.report("chapter", 100, None);

    Ok(PipelineOutput {
        source_text,
        source_encoding,
        cleaning: cleaning_anns,
        book,
    })
}

/// 把已经跑过管线的 [`PipelineOutput`] 写成 EPUB,可选 kepubify。
///
/// 返回最终产物路径(若启用 kepubify,返回 .kepub.epub;否则返回 .epub)。
/// `progress` 不需要时传 `&NoopSink`。
pub fn build_epub_from(
    pipeline: &PipelineOutput,
    output_epub: &Path,
    kepubify_path: Option<&Path>,
    progress: &dyn ProgressSink,
) -> Result<PathBuf, CoreError> {
    progress.report("epub", 0, None);
    epub::build(&pipeline.book, output_epub)?;
    progress.report("epub", 100, None);

    if let Some(kepubify) = kepubify_path {
        progress.report("kepubify", 0, None);
        let out_dir = output_epub
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let kepub = kepubify::run(kepubify, output_epub, &out_dir)?;
        progress.report("kepubify", 100, None);
        return Ok(kepub);
    }
    Ok(output_epub.to_path_buf())
}

/// 顶层一站式入口:读文件 → 跑管线 → 写 EPUB → 可选 kepubify。
///
/// 桥接层的旧 `convert` 命令使用此入口(回归保险)。阶段二界面改用
/// [`run_pipeline`] + [`build_epub_from`] 拆两步走以取得富标注。
pub fn convert(
    input_txt: &Path,
    output_epub: &Path,
    metadata: Metadata,
    options: &ConvertOptions,
) -> Result<PathBuf, CoreError> {
    let bytes = std::fs::read(input_txt).map_err(|e| {
        CoreError::Encoding(encoding::EncodingError::Io {
            path: input_txt.display().to_string(),
            source: e,
        })
    })?;
    let pipeline = run_pipeline(&bytes, metadata, options, &NoopSink)?;
    build_epub_from(
        &pipeline,
        output_epub,
        options.kepubify_path.as_deref(),
        &NoopSink,
    )
}
