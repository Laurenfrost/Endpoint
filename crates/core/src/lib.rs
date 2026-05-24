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
//! # 阶段三新增能力(进行中)
//! - **本地水印检测**:[`watermark`] 模块,本地廉价、可解释、零 LLM 依赖。
//!   3.0 子阶段只有骨架(`analyze` 返空);3.1/3.2 填三特征,3.3 接入 auto 镜像。
//!
//! # 三层管线 API
//!
//! 1. [`run_pipeline`]:从字节出发跑完编码 + 清洗 + 章节边界 + 水印 + 段落物化,
//!    产出 [`PipelineOutput`](含富标注,供阶段二的界面消费)。**不**写 EPUB。
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
pub mod watermark;

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
    /// 阶段三新增:水印检测配置。`None` = 使用 [`watermark::WatermarkConfig::default()`]
    /// (智能默认:`auto=0.70` / `suspect=0.35`,权重 `0.40/0.20/0.40`)。
    /// 阶段三 3.5(推迟到阶段四开头)前,前端**不**会传此字段;桥接层用 `default()`。
    pub watermark: Option<watermark::WatermarkConfig>,
}

/// 阶段二界面会消费的入口:从字节运行完整文本管线,产出富标注。
///
/// 不读文件、不写文件——内存语义。便于核心库脱离 IO 单元测试。
///
/// `progress` 用于向上层回报阶段进度;不需要进度时传 `&NoopSink`。
///
/// # 阶段三流水线顺序
///
/// 详见 `docs/stage3-design.md` 第五节 5.1:
/// `decoding → cleaning → chapter::parse(只识别边界) → watermark → 合并 auto 镜像 → materialize_paragraphs`。
///
/// 3.0 子阶段:[`watermark::analyze`] 返回空列表,合并步骤为恒等,与阶段二行为等价。
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
    let cleaning_anns_base = cleaning::analyze(&source_text);
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

    let mut book = chapter::parse(&source_text, &rules, metadata)?;
    progress.report("chapter", 100, None);

    // 阶段三:水印检测。
    progress.report("watermark", 0, None);
    let wm_config = options.watermark.clone().unwrap_or_default();
    let watermarks = watermark::analyze(
        &source_text,
        &book,
        &rules,
        &cleaning_anns_base,
        &wm_config,
    );
    progress.report("watermark", 100, None);

    // auto 水印镜像 → cleaning:把 verdict==auto 的 watermark 同 span 写入 cleaning
    // (用 WatermarkKeyword/Repetition/NonCjk 三个 CleaningKind 变体),
    // suspect 不进 cleaning。详见 `docs/stage3-design.md` 第二节"镜像不变式"。
    let cleaning_final = watermark::merge_auto_into_cleaning(cleaning_anns_base, &watermarks);

    // 物化段落:此时 cleaning_final 已含格式清洗 + auto 水印镜像,
    // 一次物化即可同时扣除两类删除,EPUB 路径单一不分支。
    chapter::materialize_paragraphs(&mut book, &source_text, &cleaning_final);

    Ok(PipelineOutput {
        source_text,
        source_encoding,
        cleaning: cleaning_final,
        watermark: watermarks,
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

#[cfg(test)]
mod tests {
    //! 阶段三 3.3 端到端测试:验证 watermark auto 镜像在 `run_pipeline` 全链路上的效果。
    //!
    //! 这些测试不写文件、不调 EPUB 构建——只比较 `enabled = true/false` 两份 [`PipelineOutput`]
    //! 的 cleaning / book.paragraphs 差异,确认 auto 水印被从段落正文里自然扣除。

    use super::*;
    use crate::domain::{BookEntry, CleaningKind, WatermarkVerdict};

    /// 端到端:含明显水印的样本,enabled=true → 水印从 paragraphs 消失;
    /// enabled=false → 水印保留在 paragraphs 中。
    #[test]
    fn auto_watermark_disappears_from_paragraphs_when_enabled() {
        // 构造一个章节,正文里夹了 50 行典型水印行(keyword + repetition → auto)
        let watermark_line = "本文首发于纵横中文网,谢谢支持";
        let mut text = String::from("第一章 起\n正文开头一段。\n");
        for i in 0..50 {
            text.push_str(watermark_line);
            text.push('\n');
            text.push_str(&format!("普通正文第 {} 段。\n", i));
        }
        text.push_str("章节末尾。\n");
        let bytes = text.as_bytes();

        let enabled_opts = ConvertOptions::default();
        let mut disabled_opts = ConvertOptions::default();
        disabled_opts.watermark = Some({
            let mut c = watermark::WatermarkConfig::default();
            c.enabled = false;
            c
        });

        let with_wm = run_pipeline(
            bytes,
            Metadata::new("测试书", "测试作者"),
            &enabled_opts,
            &NoopSink,
        )
        .expect("管线应成功");
        let without_wm = run_pipeline(
            bytes,
            Metadata::new("测试书", "测试作者"),
            &disabled_opts,
            &NoopSink,
        )
        .expect("管线应成功");

        // —— enabled:watermark 列表应当含 auto;cleaning 应当含 watermark_* kind ——
        let auto_count = with_wm
            .watermark
            .iter()
            .filter(|w| w.verdict == WatermarkVerdict::Auto)
            .count();
        assert!(auto_count >= 1, "应当至少识别出 1 个 auto 水印");
        let cleaning_wm_count = with_wm
            .cleaning
            .iter()
            .filter(|c| c.kind.is_watermark())
            .count();
        assert_eq!(
            cleaning_wm_count, auto_count,
            "镜像不变式:cleaning 中 watermark_* 数应当等于 auto 数"
        );

        // —— 章节 paragraphs:enabled 不应含水印行,disabled 应含 ——
        fn collect_paragraphs(p: &PipelineOutput) -> Vec<String> {
            let mut all = Vec::new();
            for e in &p.book.entries {
                match e {
                    BookEntry::Chapter(c) => {
                        for para in &c.paragraphs {
                            all.push(para.as_str().to_string());
                        }
                    }
                    BookEntry::Volume(v) => {
                        for ch in &v.chapters {
                            for para in &ch.paragraphs {
                                all.push(para.as_str().to_string());
                            }
                        }
                    }
                }
            }
            all
        }

        let with_paragraphs = collect_paragraphs(&with_wm);
        let without_paragraphs = collect_paragraphs(&without_wm);

        let with_count = with_paragraphs
            .iter()
            .filter(|p| p.contains(watermark_line))
            .count();
        let without_count = without_paragraphs
            .iter()
            .filter(|p| p.contains(watermark_line))
            .count();
        assert_eq!(
            with_count, 0,
            "enabled=true:水印行不应出现在 paragraphs,实际 {} 次",
            with_count
        );
        assert!(
            without_count >= 1,
            "enabled=false:水印行应保留在 paragraphs,实际 {} 次",
            without_count
        );

        // —— 普通正文段落两份应当一致(水印移除不应影响其它内容) ——
        let with_normal: Vec<&str> = with_paragraphs
            .iter()
            .filter(|p| !p.contains(watermark_line))
            .map(String::as_str)
            .collect();
        let without_normal: Vec<&str> = without_paragraphs
            .iter()
            .filter(|p| !p.contains(watermark_line))
            .map(String::as_str)
            .collect();
        assert_eq!(
            with_normal, without_normal,
            "去除水印后,正常段落两份应当一致"
        );
    }

    /// suspect 不进 cleaning;EPUB 输出不会自动扣除 suspect 行。
    #[test]
    fn suspect_only_does_not_alter_cleaning_or_paragraphs() {
        // 一行 keyword 命中 → 单特征 fused 0.40 → suspect(不重复,所以不会升 auto)
        let text = "第一章 起\n正文一。\n本文首发于纵横中文网。\n正文二。\n";
        let pipeline = run_pipeline(
            text.as_bytes(),
            Metadata::new("测试", "作者"),
            &ConvertOptions::default(),
            &NoopSink,
        )
        .unwrap();

        // 应当至少有一个 suspect
        assert!(
            pipeline
                .watermark
                .iter()
                .any(|w| w.verdict == WatermarkVerdict::Suspect),
            "应当识别出 suspect 水印"
        );
        // cleaning 中应当**没有** watermark_* kind(suspect 不镜像)
        assert!(
            pipeline.cleaning.iter().all(|c| !c.kind.is_watermark()),
            "suspect 不应该镜像到 cleaning:{:?}",
            pipeline.cleaning
        );
        // paragraphs 应当保留水印行
        let has_suspect_line = match &pipeline.book.entries[0] {
            BookEntry::Chapter(c) => c.paragraphs.iter().any(|p| p.as_str().contains("纵横中文网")),
            _ => false,
        };
        assert!(has_suspect_line, "suspect 行应保留在 paragraphs");
    }

    /// run_pipeline 输出的 cleaning 始终按 span.start 升序、互不重叠
    /// (即使加入了 auto 水印镜像后)。
    #[test]
    fn pipeline_cleaning_stays_sorted_and_non_overlapping_after_mirror() {
        let watermark_line = "首发于纵横中文网,更新最快";
        let mut text = String::from("第一章\n");
        for i in 0..30 {
            text.push_str(watermark_line);
            text.push('\n');
            text.push_str(&format!("正文 {}\n", i));
        }
        // 加些会触发 cleaning 的全角空格
        text.push_str("\u{3000}有缩进的一行。\n");

        let pipeline = run_pipeline(
            text.as_bytes(),
            Metadata::new("测试", "作者"),
            &ConvertOptions::default(),
            &NoopSink,
        )
        .unwrap();

        for w in pipeline.cleaning.windows(2) {
            assert!(
                w[0].span.start <= w[1].span.start,
                "cleaning 未升序:{:?} 后跟 {:?}",
                w[0],
                w[1]
            );
            assert!(
                w[0].span.end <= w[1].span.start,
                "cleaning 重叠:{:?} vs {:?}",
                w[0],
                w[1]
            );
        }

        // 校验既有 cleaning 类型(FullwidthSpace)与镜像(WatermarkKeyword) 都出现
        let kinds: std::collections::HashSet<CleaningKind> =
            pipeline.cleaning.iter().map(|c| c.kind).collect();
        assert!(
            kinds.iter().any(|k| matches!(k, CleaningKind::FullwidthSpace)),
            "应当出现原 cleaning FullwidthSpace"
        );
        assert!(
            kinds.iter().any(|k| k.is_watermark()),
            "应当出现镜像后的 watermark_* kind"
        );
    }
}
