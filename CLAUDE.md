# CLAUDE.md

给 Claude Code 看的维护指引。所有功能已开发完毕（阶段零至阶段四 4.8），本文件聚焦于**维护和迭代**，而非规划。

---

## 产品定位

将中文网络小说 `.txt` 转换为 EPUB / Kobo kepub 的 Windows 桌面工具。面向个人使用，定位「智能默认 + 高级选项」。核心管线：编码探测 → 文本清洗 → 章节识别 → 水印检测 → EPUB 构建 → kepubify 优化。

---

## 技术栈

- **框架**：Tauri 2（Rust + Svelte 5 + Vite 6）
- **核心库**：`crates/core`（纯 Rust，零 Tauri 依赖）
- **桥接层**：`src-tauri/src/`（Tauri 命令，薄胶水）
- **前端**：`ui/src/`（Svelte 5 runes API）
- **LLM**：OpenAI 兼容接口（reqwest blocking），通过 trait 可替换，完全可选

---

## 三层架构原则（不得违背）

1. **核心库不依赖 Tauri**：`crates/core` 中不得 import Tauri 或前端相关依赖。
2. **桥接层极薄**：每个 Tauri 命令只做「解析参数 → `spawn_blocking` 调核心库 → 返回结果」，不含业务逻辑。
3. **LLM 不得变为强依赖**：所有 LLM 功能必须可降级。未配 API key 时使用 `NoopLlmClient`，功能静默跳过，不报错。

---

## 文件地图

```
crates/core/src/
  domain.rs        领域模型（冻结契约，见下文）
  encoding.rs      编码探测与解码（BOM/chardetng/encoding_rs）
  cleaning.rs      格式清洗（产出 CleaningAnnotation，不修改原文）
  chapter.rs       章节解析 + 超长章节检测 + materialize_paragraphs
  watermark.rs     三特征水印检测 + 双阈值分流 + 用户决策叠加
  rules.rs         规则库（JSON I/O + 16 条内置规则）
  epub.rs          EPUB 3 构建（EpubOptions：css/cover/font）
  kepubify.rs      外部 kepubify 进程调用
  cover_gen.rs     文字封面 PNG 生成（image + ab_glyph）
  llm.rs           LlmClient trait + NoopLlmClient
  lib.rs           run_pipeline / build_epub_from / ConvertOptions / ProgressSink

src-tauri/src/
  commands.rs      全部 Tauri 命令
  state.rs         AppState（pipeline 缓存 + cancel flags + llm_client）
  llm_config.rs    config.toml 读写 + user_rules_path()
  openai_client.rs OpenAI 兼容 HTTP 客户端（实现 LlmClient trait）
  main.rs          命令注册

ui/src/
  App.svelte             三栏骨架（标题栏 + ActivityBar + Sidebar + TextView）
  ipc.js                 Tauri IPC 薄封装（所有 invoke 调用在此）
  stores/                响应式状态（pipeline / stage / progress / annotations / decisions / llm）
  text/                  ByteIndex.js / VirtualText.svelte / OverviewRuler.svelte
  layout/                ActivityBar.svelte / Sidebar.svelte
  stages/                Stage1Input / Stage2Cleaning / Stage3Chapter / Stage4Export
```

---

## 冻结契约（IPC 锁定，改动需同步测试和前端）

核心类型（`crates/core/src/domain.rs`）：

```rust
Span { start: usize, end: usize }  // UTF-8 字节半开区间，参照 decoded source

PipelineOutput {
    source_text: String,
    source_encoding: String,
    cleaning: Vec<CleaningAnnotation>,
    watermark: Vec<WatermarkAnnotation>,
    book: Book,
}

CleaningKind（8 项，snake_case IPC 锁定）：
  blank_line_compression | leading_fullwidth_space | inline_fullwidth_space
  control_char | trailing_whitespace
  watermark_keyword | watermark_repetition | watermark_non_cjk

WatermarkVerdict: Auto | Suspect
WatermarkSignalKind: Repetition | NonCjkRatio | KeywordRegex | LlmAdjudication
```

**IPC 锁定测试**（不得修改期望值来「让测试过」）：
- `pipeline_output_json_shape_is_frozen`：PipelineOutput 字段名
- `cleaning_kind_serializes_all_eight_variants_snake_case`：CleaningKind 8 个变体

**改动规则**：新增字段安全；修改已有字段名必须同步更新前端所有引用和锁定测试。

---

## 默认哲学

- **「智能默认 = 不动用户文本」**：`CleaningConfig::default()` 全关；`WatermarkConfig::suspect_threshold = 0.42`（需两个独立特征才触发灰区）。新功能应继承「用户主动开启」的默认哲学。
- **用户决策仅本次会话**：`UserDecision` 不持久化，`reload` 即丢。持久化走规则回流（`rules.json`）。

---

## 配置与数据文件

| 路径 | 用途 |
|------|------|
| `%APPDATA%\Endpoint\config.toml` | LLM 配置（base_url / model / api_key） |
| `%APPDATA%\Endpoint\rules.json` | 用户/LLM 归纳的水印规则（跨会话持久化） |
| `src-tauri/resources/fonts/*.ttf` | 嵌入字体（不进 git，用 `scripts/fetch-fonts.ps1` 下载） |
| `src-tauri/resources/themes/*.css` | CSS 主题预设（standard / classic / highcontrast） |
| `%TEMP%\endpoint_text_cover.png` | 文字封面临时文件 |

---

## 禁区（不得改动）

- `domain.rs` 中的冻结类型和 `Span` 语义（UTF-8 字节 + 半开区间 + decoded source 参照）
- `VirtualText.svelte` / `OverviewRuler.svelte` / `ActivityBar.svelte` / 三栏骨架核心交互
- IPC 锁定测试的期望值

---

## 已知技术债

| 项目 | 说明 |
|------|------|
| VirtualText 行高估算 | 非精确，CJK 短行场景未发现明显问题，可先不动 |
| Cancel 接口 | `TODO(cancel)` 散落核心库，长循环未检查信号 |
| 字体子集化 | 嵌入完整字体约 16MB，合法但较大 |
| OpenAI 以外的 provider | 目前只支持 OpenAI 兼容接口 |

---

## 开发命令

```powershell
# 下载内置字体
.\scripts\fetch-fonts.ps1

# 开发模式（热重载）
cargo tauri dev

# 运行核心库测试
cargo test --workspace

# 构建发行版
cargo tauri build
```

详细架构见 `docs/architecture.md`，历史决策见 `docs/changelogs.md`。
