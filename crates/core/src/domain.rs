//! 领域模型:贯穿全部模块的「通用语言」。
//!
//! 本文件除了少量便利构造器之外只放类型定义,不放业务逻辑。
//!
//! # 富标注输出契约(阶段一冻结)
//!
//! CLAUDE.md 第三节第 6 条 + 第八节反复强调:核心库的输出不能只是"干净的最终文本",
//! 而必须是**带原文坐标的结构化标注**。同一份富标注在阶段二之后会驱动三处 UI 消费:
//! 正文高亮、侧边栏列表、概览标尺。一旦冻结,后续修改需特别谨慎(参见第十二节工作约定第 4 条)。
//!
//! 契约要点如下:
//!
//! ## 1. 偏移单位:**UTF-8 字节**
//!
//! 所有 [`Span`] 的 `start` / `end` 都是 UTF-8 字节偏移,不是字符计数。理由:
//! - Rust `&str` 原生按字节切片,O(1) 切片是扫描两百万字文本时的性能保障。
//! - char 计数每次访问都是 O(n),全程使用会让阶段三的水印检测无法承受。
//! - 字节偏移在不同语言/序列化层中是无歧义的(char count 会因 NFC/NFD 漂移)。
//!
//! 前端(JavaScript 字符串是 UTF-16)在加载源文本时建一次 byte→UTF-16 索引表,
//! 之后所有 highlight 区间 O(1) 查表即可。CJK BMP 字符在 UTF-8 中占 3 字节、UTF-16 中占 2 字节,
//! 这一层映射对内存压力可忽略。
//!
//! ## 2. 端点语义:**半开区间 `[start, end)`**
//!
//! 与 Rust `str::get(range)` 完全一致。不变式:
//! - `start <= end`
//! - `start` 与 `end` 都必须落在 UTF-8 字符边界上(用 [`str::is_char_boundary`] 校验)
//! - `end <= source_text.len()`
//!
//! ## 3. 坐标参照系:**解码后、清洗前的源文本**(下称 *decoded source*)
//!
//! 一切 span 都指向 [`PipelineOutput::source_text`]——即编码探测之后、任何清洗发生之前的字符串。
//! 三个候选方案中只有它能同时承担"UI 主显示"和"哪里被删的锚"两个角色:
//! - 原始字节坐标会与具体编码耦合,前端无法稳定渲染。
//! - 清洗后文本坐标会丢失"被删除区域曾经在这里"的信息——而 UI 必须能把删除区域作为
//!   红色高亮显示在原文上,这是阶段二界面的核心交互。
//! - 解码后文本 = "用户的 txt 文件,但当作 Unicode 文本看"——是唯一兼具稳定性与可视化价值的锚。
//!
//! ## 4. 清洗以**标注列表**存在,不预先 materialize 清洗后文本
//!
//! [`CleaningAnnotation`] 是一条"删除/替换指令",其 `span` 指向 *decoded source* 中应被处理的区间。
//! 真正的"清洗后文本"是按需 derive 的视图——EPUB 构建从某章 `body_span` 区间内取文本时,
//! 应当扣除该范围内命中的清洗标注,得到最终段落内容。这种"标注 + 按需应用"的模式是把
//! "哪里被改了"留在数据流中而非埋进字符串的关键。
//!
//! ## 5. 出处必须标记
//!
//! 每个 [`Chapter`] 与 [`Volume`] 都必须填 [`ChapterOrigin`](从哪条规则、还是结构补偿、还是 LLM、
//! 还是兜底)与 `matched_rule_id`(若由规则命中则记录规则 id)。阶段二的预览界面据此把
//! "高置信度章节"与"程序猜测的章节"区分开。
//!
//! ## 6. 阶段三契约扩展:`watermark` 字段 + auto 镜像
//!
//! 阶段三的水印检测以 [`WatermarkAnnotation`] 列表存放在 [`PipelineOutput::watermark`] 字段。
//! 该列表持**全部**水印评分明细(auto + suspect + scores + signals + verdict),供前端正文高亮、
//! 侧边栏"为什么被标"、概览标尺消费。
//!
//! verdict == [`WatermarkVerdict::Auto`] 的水印**同时**镜像到 `cleaning` 列表中,
//! kind 为 `CleaningKind::Watermark*` 三个变体之一。这样 EPUB / [`crate::chapter`] 物化路径
//! 单一读 `cleaning` 即可同时扣除格式清洗 + 自动水印,不必感知 watermark 字段的存在。
//! verdict == [`WatermarkVerdict::Suspect`] **不**进 cleaning,EPUB 输出中保留,
//! 等待阶段四 approve/dismiss 交互后决策。
//!
//! 镜像不变式与重叠合并规则详见 `docs/stage3-design.md` 第二节。
//!
//! ## 7. 冻结状态
//!
//! 阶段一交付时本契约冻结。阶段二之后修改契约需特别谨慎;新增字段优于改动已有字段。
//! 阶段三按 `docs/stage3-design.md` 第十节决策记录新增了 `watermark` 字段 + 3 个
//! `CleaningKind::Watermark*` 变体,属于增量扩展,不破坏阶段二已冻结的字段名与形状。

use serde::{Deserialize, Serialize};

/// 解码后源文本中的半开字节区间 `[start, end)`。详见模块文档「富标注输出契约」。
///
/// # 不变式
/// - `start <= end`
/// - `start`、`end` 都落在 UTF-8 字符边界上
/// - `end <= source.len()`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// 构造一个 span。**不**校验字符边界——校验留给消费者或 debug_assert。
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// 从 decoded source 中按本 span 切出字符串切片。调用方须保证 `source` 与本 span
    /// 出自同一份 decoded source,且 span 端点在 UTF-8 字符边界上。
    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}

/// 清洗操作的标注。**这不是已清洗的文本**,而是一条"在 decoded source 的某区间上执行
/// 删除或替换"的指令。详见模块文档第 4 条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleaningAnnotation {
    /// 指向 decoded source 中将被清洗的区间。
    pub span: Span,
    pub kind: CleaningKind,
    /// `None` 表示纯删除;`Some(s)` 表示用 `s` 替换。多余空行压缩后保留的那一个 `\n`
    /// 也用 `Some("\n")` 表达,以保持"标注完全描述行为"的一致性。
    pub replacement: Option<String>,
}

/// 清洗类型。**阶段三 v2 起共 8 个变体**:
/// - 前 5 个是阶段一的"确定性、低风险格式整理";其中 `LeadingFullwidthSpace` 与
///   `InlineFullwidthSpace` 是 v2 拆分 `FullwidthSpace` 而来(详见
///   `docs/stage3-v2-design.md` 第三节 3.1)。
/// - 后 3 个 `Watermark*` 变体是阶段三新增的"自动判定水印的镜像入口"——详见模块文档
///   第 6 节与 `docs/stage3-design.md` 第二节。
///
/// `#[serde(rename_all = "snake_case")]` 让前端拿到 `"leading_fullwidth_space"` /
/// `"watermark_keyword"` 等。
///
/// **v2 契约破坏**:阶段三 v1 时存在的 `fullwidth_space` snake_case 值在 v2.0 起消失,
/// 拆为 `leading_fullwidth_space`(默认 disabled,保留段首缩进)+ `inline_fullwidth_space`
/// (默认 enabled,清理行内连续多余全角)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleaningKind {
    /// 多余空行压缩(连续 2 个及以上空行 → 保留 1 个)
    BlankLineCompression,
    /// **v2 新增**:行首连续全角空格 `U+3000`(典型为中文段首缩进 2 全角)
    /// 默认 `CleaningConfig::leading_fullwidth_space = false`——尊重中文排版习惯,**不**删除。
    LeadingFullwidthSpace,
    /// **v2 新增**:行内连续 ≥2 个全角空格(典型为排版错误,不应出现在段首之外)
    /// 默认 `CleaningConfig::inline_fullwidth_space = true` 删除。
    InlineFullwidthSpace,
    /// `U+0000`-`U+001F` 中的非可视控制字符(\t / \n 除外)被剥离
    ControlChar,
    /// 行尾尾随空白。v2 起字符集扩展到 `\r` / NBSP(`U+00A0`)/
    /// 零宽(`U+200B`-`U+200D`、`U+FEFF`)。
    TrailingWhitespace,
    /// 阶段三新增:auto 水印镜像——关键词正则命中
    WatermarkKeyword,
    /// 阶段三新增:auto 水印镜像——行频统计触发
    WatermarkRepetition,
    /// 阶段三新增:auto 水印镜像——非中文占比触发
    WatermarkNonCjk,
}

impl CleaningKind {
    /// 是否是阶段三 watermark 镜像变体(`WatermarkKeyword` / `WatermarkRepetition` /
    /// `WatermarkNonCjk`)。前端按 kind 拆"清洗"与"水印"两层时使用。
    pub fn is_watermark(self) -> bool {
        matches!(
            self,
            CleaningKind::WatermarkKeyword
                | CleaningKind::WatermarkRepetition
                | CleaningKind::WatermarkNonCjk
        )
    }
}

/// 一本完整的书。所有内嵌 chapter/volume 的 span 都指向 [`PipelineOutput::source_text`]。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub metadata: Metadata,
    /// 顶层条目有序列表:卷或章。无卷小说就是一串 `Chapter`,有卷小说混入 `Volume`。
    pub entries: Vec<BookEntry>,
}

/// `#[serde(tag = "type", rename_all = "snake_case")]` 让前端拿到的 JSON 形如
/// `{ "type": "volume", ... }` / `{ "type": "chapter", ... }`,详见
/// `docs/stage2-design.md` 第三节 JSON shape 冻结。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BookEntry {
    Volume(Volume),
    Chapter(Chapter),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub title: String,
    pub chapters: Vec<Chapter>,
    /// 卷标题行在 decoded source 中的位置。
    pub heading_span: Span,
    pub origin: ChapterOrigin,
    /// 命中此卷的规则 id(仅 `origin == RegexMatch` 时有意义)。
    pub matched_rule_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    /// **不进 IPC**:与 `body_span` + `cleaning` 完全冗余,前端可按需 derive。
    /// 详见 `docs/stage2-design.md` 第三节"两个 skip_serializing 字段"。
    /// Rust 端仍保留以供 EPUB 构建模块使用。
    #[serde(skip)]
    pub paragraphs: Vec<Paragraph>,
    /// 章标题行(不含正文)在 decoded source 中的位置。
    /// 阶段三水印检测、阶段二 UI 章节高亮均以此为锚点。
    pub heading_span: Span,
    /// 正文区域:标题行末尾 → 下一章/卷标题起始(或文末)。
    /// `body_span` 一定与 `heading_span` 不重叠,且紧接其后。
    pub body_span: Span,
    pub origin: ChapterOrigin,
    /// 命中此章的规则 id(仅 `origin == RegexMatch` 时有意义)。
    pub matched_rule_id: Option<String>,
}

/// 段落:纯文本,**不包含任何 XHTML**。段落 → XHTML 的转换在 EPUB 构建阶段进行。
///
/// `#[serde(transparent)]` 让 `Paragraph("hi")` 直接序列化为字符串 `"hi"`,而非
/// `["hi"]`——尽管目前 `Chapter.paragraphs` 已被 `#[serde(skip)]`,这里仍保留
/// 透明序列化以保安全。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Paragraph(pub String);

impl Paragraph {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 「这一章/卷是怎么来的」的出处标记。
///
/// `#[serde(rename_all = "snake_case")]` 让前端拿到 `"regex_match"` / `"fallback"` 等。
/// 详见 `docs/stage2-design.md` 第三节枚举锁定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterOrigin {
    /// 由规则库的某条规则匹配得到(`matched_rule_id` 应同时填写)。
    RegexMatch,
    /// 结构分析(超长章二次切分)补出。阶段三才会出现。
    Structural,
    /// LLM 灰区仲裁产出。阶段四才会出现。
    LlmAdjudicated,
    /// 整本未识别出任何章节标题,按空行/字数兜底切分;或被识别为楔子/序章前文。
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub language: String,
    /// **不进 IPC**:二进制图片不走 JSON。阶段四再做封面 UI。Rust 端 EPUB 构建仍会用。
    #[serde(skip)]
    pub cover: Option<Vec<u8>>,
    pub description: Option<String>,
    /// 分类/标签,EPUB 的 `dc:subject` × N。供 Kobo/Calibre 按分类分组。
    #[serde(default)]
    pub subjects: Vec<String>,
    /// 系列名(EPUB `belongs-to-collection` + `calibre:series`)。
    #[serde(default)]
    pub series: Option<String>,
    /// 系列内序号(`group-position` + `calibre:series_index`)。
    #[serde(default)]
    pub series_index: Option<u32>,
    /// 版权声明(`dc:rights`)。`None` 时 EPUB 构建会填入默认模板。
    #[serde(default)]
    pub rights: Option<String>,
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
            subjects: Vec::new(),
            series: None,
            series_index: None,
            rights: None,
        }
    }
}

/// 一条水印检测的输出。`span` 指向 [`PipelineOutput::source_text`] 中被命中的行
/// (UTF-8 字节偏移,半开区间)。
///
/// 阶段三新增,详见模块文档第 6 节与 `docs/stage3-design.md` 第二节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatermarkAnnotation {
    /// 被命中的行在 decoded source 中的区间。
    pub span: Span,
    /// 自动 / 灰区。
    pub verdict: WatermarkVerdict,
    /// 加权融合后的总分 `[0.0, 1.0]`。
    pub score: f32,
    /// 每个触发特征的明细。至少有一项。
    pub signals: Vec<WatermarkSignal>,
}

/// 水印判定结果。`#[serde(rename_all = "snake_case")]` 让前端拿到 `"auto"` / `"suspect"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkVerdict {
    /// 分数 ≥ `auto_threshold`,自动判定为水印——同时镜像到 `cleaning` 列表让 EPUB 扣除。
    Auto,
    /// `suspect_threshold` ≤ 分数 < `auto_threshold`,灰区,前端黄色高亮 + 侧边栏列表,
    /// **不**进 cleaning,EPUB 输出中保留。
    Suspect,
}

/// 单个触发特征的评分明细。前端把多条 signal 展开成可解释列表
/// (`"出现 87 次"` / `"60% 非中文字符"` / `"命中规则 builtin-watermark-url"`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatermarkSignal {
    pub kind: WatermarkSignalKind,
    /// 本特征自身的可疑度分数 `[0.0, 1.0]`。
    pub score: f32,
    /// 给前端展示的可解释文本。可空(纯调试场景)。
    pub detail: Option<String>,
}

/// 水印特征类型。`#[serde(rename_all = "snake_case")]` 让前端拿到
/// `"repetition"` / `"non_cjk_ratio"` / `"keyword_regex"` / `"llm_adjudication"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkSignalKind {
    /// 行频统计:本行内容在全文出现次数过高。
    Repetition,
    /// 非中文字符占比过高(URL / TG 链接 / 数字 ID 串等)。
    NonCjkRatio,
    /// 命中 [`crate::rules::RuleKind::Watermark`] 类规则。
    KeywordRegex,
    /// 阶段四 4.7 新增:LLM 判定(语义层仲裁结果)。
    LlmAdjudication,
}

/// **阶段三 v2.2 新增**:用户对自动检测结果的覆盖决策。
///
/// 决策**仅本次转换会话**有效(reload 即失效),不持久化到文件。
/// 决策语义详见 `docs/stage3-v2-design.md` 第三节 3.3:
///
/// | scope / 默认 verdict | approved | rejected |
/// |---------------------|----------|----------|
/// | cleaning(默认删) | 同默认(显式锁定) | **不删** |
/// | watermark auto(默认删) | 同默认 | **不删**(从 cleaning 镜像移除) |
/// | watermark suspect(默认保留) | **加入 cleaning**(等效升 auto) | 同默认 |
///
/// 即:三种"逆向改默认"的决策真正改变 EPUB 输出:
///  - cleaning rejected:span 不删
///  - watermark auto rejected:span 不删
///  - watermark suspect approved:span 删
///
/// 其余三态决策(cleaning approved / auto approved / suspect rejected)只是
/// "显式锁定默认行为",前端可视化用但不改 EPUB 内容。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserDecision {
    pub span: Span,
    pub scope: DecisionScope,
    pub verdict: DecisionVerdict,
}

/// 决策作用域:cleaning 列表 / watermark 列表。
///
/// 注意:`Watermark` 不区分 auto / suspect——前端按 span 是否在
/// `pipeline.cleaning` 中(且 kind 是 `Watermark*`)推断当前 verdict,后端按 span
/// 在 `pipeline.watermark` 中找到对应 `WatermarkAnnotation` 拿 verdict。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionScope {
    /// 来自 `PipelineOutput.cleaning` 列表(kind 是 5 种格式整理变体之一)。
    Cleaning,
    /// 来自 `PipelineOutput.watermark` 列表(auto + suspect)。
    Watermark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionVerdict {
    /// 显式接受 = "确定要删除这个 span"。
    Approved,
    /// 显式拒绝 = "确定要保留这个 span"。
    Rejected,
}

/// 核心库管线的完整输出。三处 UI 消费(正文高亮 / 侧边栏 / 概览标尺)与 EPUB 构建
/// 全部从这里取数据。
///
/// 序列化产物即阶段二的 `PipelineDto` + 阶段三 `watermark` 字段;字段顺序与名称参见
/// `docs/stage2-design.md` 第三节与 `docs/stage3-design.md` 第二节。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineOutput {
    /// 解码后的源文本——所有 span 的坐标参照系。
    pub source_text: String,
    /// 实际生效的编码标签(自动探测出的、或调用方手动覆盖的)。
    pub source_encoding: String,
    /// 清洗标注,按 `span.start` 升序排列且互不重叠。
    /// 阶段三起:其中 kind 属于 `Watermark*` 变体的条目是 verdict==auto 的水印镜像。
    pub cleaning: Vec<CleaningAnnotation>,
    /// 阶段三新增:水印检测的完整输出(auto + suspect + scores + signals)。
    /// auto 类同时在 [`Self::cleaning`] 中镜像存在;suspect 仅在此。
    /// 详见模块文档第 6 节与 `docs/stage3-design.md` 第二节"镜像不变式"。
    #[serde(default)]
    pub watermark: Vec<WatermarkAnnotation>,
    /// 章节/卷结构。所有内嵌 span 都指向 `source_text`。
    pub book: Book,
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
    fn span_slice_returns_correct_substring() {
        let s = "第一章 起\n正文";
        let span = Span::new(0, "第一章 起".len());
        assert_eq!(span.slice(s), "第一章 起");
    }

    #[test]
    fn span_len_and_is_empty() {
        assert_eq!(Span::new(3, 7).len(), 4);
        assert!(Span::new(3, 3).is_empty());
        assert!(Span::new(5, 3).is_empty(), "saturating_sub 不应 panic");
    }

    #[test]
    fn book_entry_enum_round_trips() {
        let ch = Chapter {
            title: "第一章 风云起".into(),
            paragraphs: vec![Paragraph::new("test")],
            heading_span: Span::new(0, 10),
            body_span: Span::new(10, 20),
            origin: ChapterOrigin::RegexMatch,
            matched_rule_id: Some("builtin:chapter:cn-digit".into()),
        };
        let entry = BookEntry::Chapter(ch);
        match entry {
            BookEntry::Chapter(c) => {
                assert_eq!(c.title, "第一章 风云起");
                assert_eq!(c.matched_rule_id.as_deref(), Some("builtin:chapter:cn-digit"));
            }
            BookEntry::Volume(_) => panic!("expected Chapter"),
        }
    }

    #[test]
    fn cleaning_annotation_round_trips_through_json() {
        let a = CleaningAnnotation {
            span: Span::new(10, 12),
            kind: CleaningKind::LeadingFullwidthSpace,
            replacement: Some(" ".into()),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: CleaningAnnotation = serde_json::from_str(&j).unwrap();
        assert_eq!(a, back);
    }

    /// **契约锁定测试**:验证 `PipelineOutput` 序列化后的 JSON 字段名与
    /// `docs/stage2-design.md` 第三节 + `docs/stage3-design.md` 第二节冻结的 shape 完全一致。
    ///
    /// 一旦该测试失败,意味着 IPC 契约被改动——必须同步更新设计文档与前端代码,
    /// **不要**简单调整测试期望值"让它过"。
    #[test]
    fn pipeline_output_json_shape_is_frozen() {
        let auto_wm_span = Span::new(70, 80);
        let suspect_wm_span = Span::new(85, 92);
        let pipeline = PipelineOutput {
            source_text: "第一卷 风云起\n第一章 起\n正文内容".into(),
            source_encoding: "UTF-8".into(),
            cleaning: vec![
                CleaningAnnotation {
                    span: Span::new(0, 3),
                    kind: CleaningKind::LeadingFullwidthSpace,
                    replacement: None,
                },
                // auto 水印镜像 —— 与下面 watermark[0] span 严格一致
                CleaningAnnotation {
                    span: auto_wm_span,
                    kind: CleaningKind::WatermarkKeyword,
                    replacement: None,
                },
            ],
            watermark: vec![
                WatermarkAnnotation {
                    span: auto_wm_span,
                    verdict: WatermarkVerdict::Auto,
                    score: 0.91,
                    signals: vec![WatermarkSignal {
                        kind: WatermarkSignalKind::KeywordRegex,
                        score: 1.0,
                        detail: Some("命中规则 builtin-watermark-url".into()),
                    }],
                },
                WatermarkAnnotation {
                    span: suspect_wm_span,
                    verdict: WatermarkVerdict::Suspect,
                    score: 0.42,
                    signals: vec![WatermarkSignal {
                        kind: WatermarkSignalKind::Repetition,
                        score: 0.78,
                        detail: Some("出现 9 次".into()),
                    }],
                },
            ],
            book: Book {
                metadata: Metadata {
                    title: "测试书".into(),
                    author: "测试作者".into(),
                    language: "zh-CN".into(),
                    cover: Some(vec![0xFFu8, 0xD8, 0xFF]),
                    description: None,
                    subjects: Vec::new(),
                    series: None,
                    series_index: None,
                    rights: None,
                },
                entries: vec![
                    BookEntry::Volume(Volume {
                        title: "第一卷 风云起".into(),
                        chapters: vec![Chapter {
                            title: "第一章 起".into(),
                            paragraphs: vec![Paragraph::new("正文")],
                            heading_span: Span::new(20, 30),
                            body_span: Span::new(30, 50),
                            origin: ChapterOrigin::RegexMatch,
                            matched_rule_id: Some("builtin:chapter:cn-digit".into()),
                        }],
                        heading_span: Span::new(0, 18),
                        origin: ChapterOrigin::RegexMatch,
                        matched_rule_id: Some("builtin:volume:cn-digit".into()),
                    }),
                    BookEntry::Chapter(Chapter {
                        title: "番外".into(),
                        paragraphs: vec![],
                        heading_span: Span::new(60, 66),
                        body_span: Span::new(66, 100),
                        origin: ChapterOrigin::Fallback,
                        matched_rule_id: None,
                    }),
                ],
            },
        };

        let v: serde_json::Value = serde_json::to_value(&pipeline).unwrap();

        // 顶层字段(阶段二 4 项 + 阶段三 watermark)
        assert!(v.get("source_text").is_some(), "缺 source_text");
        assert!(v.get("source_encoding").is_some(), "缺 source_encoding");
        assert!(v.get("cleaning").is_some(), "缺 cleaning");
        assert!(v.get("watermark").is_some(), "阶段三:缺 watermark");
        assert!(v.get("book").is_some(), "缺 book");

        // cleaning[0] 是 v2 拆细后的 LeadingFullwidthSpace
        let c0 = &v["cleaning"][0];
        assert_eq!(c0["span"]["start"], 0);
        assert_eq!(c0["span"]["end"], 3);
        assert_eq!(c0["kind"], "leading_fullwidth_space", "v2 起拆细的 snake_case");
        assert_eq!(c0["replacement"], serde_json::Value::Null);

        // cleaning[1] 是阶段三 auto 水印镜像
        let c1 = &v["cleaning"][1];
        assert_eq!(c1["span"]["start"], auto_wm_span.start);
        assert_eq!(c1["span"]["end"], auto_wm_span.end);
        assert_eq!(c1["kind"], "watermark_keyword");
        assert_eq!(c1["replacement"], serde_json::Value::Null);

        // watermark[0] = auto,字段名锁定为 span / verdict / score / signals
        let w0 = &v["watermark"][0];
        assert_eq!(w0["span"]["start"], auto_wm_span.start);
        assert_eq!(w0["span"]["end"], auto_wm_span.end);
        assert_eq!(w0["verdict"], "auto");
        assert!(w0["score"].as_f64().is_some(), "score 必须是数值");
        assert!(w0["signals"].is_array());
        // signal 字段名锁定为 kind / score / detail
        let s0 = &w0["signals"][0];
        assert_eq!(s0["kind"], "keyword_regex");
        assert!(s0["score"].as_f64().is_some());
        assert_eq!(s0["detail"], "命中规则 builtin-watermark-url");

        // watermark[1] = suspect
        assert_eq!(v["watermark"][1]["verdict"], "suspect");
        assert_eq!(v["watermark"][1]["signals"][0]["kind"], "repetition");

        // 镜像不变式:auto 水印的 span 也必须出现在 cleaning 列表里
        let auto_span_in_cleaning = v["cleaning"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| {
                c["span"]["start"] == auto_wm_span.start
                    && c["span"]["end"] == auto_wm_span.end
                    && c["kind"]
                        .as_str()
                        .map(|s| s.starts_with("watermark_"))
                        .unwrap_or(false)
            });
        assert!(auto_span_in_cleaning, "auto 水印应当镜像到 cleaning");

        // 镜像不变式:suspect 水印的 span **不应**出现在 cleaning 列表里
        let suspect_span_in_cleaning = v["cleaning"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| {
                c["span"]["start"] == suspect_wm_span.start
                    && c["span"]["end"] == suspect_wm_span.end
            });
        assert!(
            !suspect_span_in_cleaning,
            "suspect 水印不应镜像到 cleaning"
        );

        // metadata.cover 必须 **不在** 输出里(skip_serializing)
        assert!(
            v["book"]["metadata"].get("cover").is_none(),
            "Metadata.cover 应当 #[serde(skip)],不进 IPC"
        );
        assert_eq!(v["book"]["metadata"]["title"], "测试书");
        assert_eq!(v["book"]["metadata"]["language"], "zh-CN");

        // entries[0] = volume
        let vol = &v["book"]["entries"][0];
        assert_eq!(vol["type"], "volume", "BookEntry tag 必须是 type");
        assert!(vol.get("title").is_some());
        assert!(vol.get("heading_span").is_some());
        assert_eq!(vol["origin"], "regex_match");
        assert_eq!(vol["matched_rule_id"], "builtin:volume:cn-digit");
        let ch = &vol["chapters"][0];
        // chapter 内部:paragraphs 必须 **不在** 输出里
        assert!(
            ch.get("paragraphs").is_none(),
            "Chapter.paragraphs 应当 #[serde(skip)],不进 IPC"
        );
        assert!(ch.get("heading_span").is_some());
        assert!(ch.get("body_span").is_some());
        assert_eq!(ch["origin"], "regex_match");

        // entries[1] = chapter,顶层直挂(无卷)
        let ext = &v["book"]["entries"][1];
        assert_eq!(ext["type"], "chapter");
        assert_eq!(ext["origin"], "fallback");
        assert_eq!(ext["matched_rule_id"], serde_json::Value::Null);
    }

    /// 锁定阶段三 v2 `CleaningKind` 完整 **8 项** 变体名(snake_case)。
    /// 若有人新增/改名变体而忘记更新设计文档,本测试会拦下。
    /// v2 破坏:`fullwidth_space` 已消失,拆为 `leading_fullwidth_space` + `inline_fullwidth_space`。
    #[test]
    fn cleaning_kind_serializes_all_eight_variants_snake_case() {
        use serde_json::to_string;
        assert_eq!(to_string(&CleaningKind::BlankLineCompression).unwrap(),  "\"blank_line_compression\"");
        assert_eq!(to_string(&CleaningKind::LeadingFullwidthSpace).unwrap(), "\"leading_fullwidth_space\"");
        assert_eq!(to_string(&CleaningKind::InlineFullwidthSpace).unwrap(),  "\"inline_fullwidth_space\"");
        assert_eq!(to_string(&CleaningKind::ControlChar).unwrap(),           "\"control_char\"");
        assert_eq!(to_string(&CleaningKind::TrailingWhitespace).unwrap(),    "\"trailing_whitespace\"");
        assert_eq!(to_string(&CleaningKind::WatermarkKeyword).unwrap(),      "\"watermark_keyword\"");
        assert_eq!(to_string(&CleaningKind::WatermarkRepetition).unwrap(),   "\"watermark_repetition\"");
        assert_eq!(to_string(&CleaningKind::WatermarkNonCjk).unwrap(),       "\"watermark_non_cjk\"");
    }

    /// 锁定 `WatermarkVerdict` 与 `WatermarkSignalKind` 的 snake_case 变体名。
    #[test]
    fn watermark_enums_serialize_snake_case() {
        use serde_json::to_string;
        assert_eq!(to_string(&WatermarkVerdict::Auto).unwrap(),    "\"auto\"");
        assert_eq!(to_string(&WatermarkVerdict::Suspect).unwrap(), "\"suspect\"");
        assert_eq!(to_string(&WatermarkSignalKind::Repetition).unwrap(),   "\"repetition\"");
        assert_eq!(to_string(&WatermarkSignalKind::NonCjkRatio).unwrap(),  "\"non_cjk_ratio\"");
        assert_eq!(to_string(&WatermarkSignalKind::KeywordRegex).unwrap(), "\"keyword_regex\"");
    }

    #[test]
    fn cleaning_kind_is_watermark_helper() {
        assert!(!CleaningKind::LeadingFullwidthSpace.is_watermark());
        assert!(!CleaningKind::InlineFullwidthSpace.is_watermark());
        assert!(!CleaningKind::BlankLineCompression.is_watermark());
        assert!(!CleaningKind::TrailingWhitespace.is_watermark());
        assert!(!CleaningKind::ControlChar.is_watermark());
        assert!(CleaningKind::WatermarkKeyword.is_watermark());
        assert!(CleaningKind::WatermarkRepetition.is_watermark());
        assert!(CleaningKind::WatermarkNonCjk.is_watermark());
    }
}
