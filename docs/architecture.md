# 软件架构

## 整体结构

三层架构：**核心库**（纯 Rust）+ **桥接层**（Tauri 命令）+ **前端**（Svelte 5）。

```
ui/                                     前端（Svelte 5 + Vite + Tailwind v4 + shadcn-svelte 风格）
  vite.config.js                        @tailwindcss/vite 插件 + $lib 别名
  jsconfig.json                         路径别名（$lib → src/lib）
  src/
    App.svelte                          三栏骨架 + <ModeWatcher /> 挂载
    app.css                             Tailwind v4 入口 + 主题 CSS 变量（light/dark）
    main.js                             Vite 入口（import app.css 后 mount App）
    ipc.js                              Tauri IPC 薄封装
    stores/                             响应式状态（pipeline / stage / progress / annotations / decisions / llm）
    text/                               ByteIndex.js / VirtualText / OverviewRuler / TextView
    layout/                             ActivityBar / Sidebar / StatusBar
    stages/                             Stage1Input / Stage2Cleaning / Stage3Chapter / Stage4Export / Settings
    lib/
      utils.js                          cn() = clsx + tailwind-merge
      components/mode-toggle.svelte     三态明暗切换按钮
      components/ui/                    shadcn-svelte 风格组件（手写，bits-ui 驱动）

src-tauri/                              桥接层（Tauri 应用壳）
  src/
    commands.rs                         全部 Tauri 命令（薄胶水，无业务逻辑）
    state.rs                            AppState（pipeline 缓存 + task 计数 + LLM 客户端）
    llm_config.rs                       config.toml 读写 + user_rules_path
    openai_client.rs                    OpenAI 兼容 HTTP 客户端（实现 LlmClient trait）
    main.rs                             命令注册

crates/core/                            核心库（不依赖 Tauri）
  src/
    domain.rs                           领域模型（冻结契约）
    encoding.rs                         编码探测与解码
    cleaning.rs                         格式清洗（产出标注，不修改原文）
    chapter.rs                          章节解析 + 超长章节检测
    watermark.rs                        水印检测（三特征 + 双阈值）
    rules.rs                            规则库（JSON I/O + builtin 规则）
    epub.rs                             EPUB 构建
    kepubify.rs                         kepubify 进程调用
    cover_gen.rs                        文字封面 PNG 生成
    llm.rs                              LlmClient trait + NoopLlmClient
    lib.rs                              run_pipeline / build_epub_from / ConvertOptions / ProgressSink
```

---

## 核心库模块详解

### 领域模型（domain.rs）—— 冻结

所有模块通过共享领域模型通信，不互相直接调用。

**核心类型**：

```rust
// 坐标：UTF-8 字节偏移，半开区间 [start, end)，参照 decoded source（清洗前）
pub struct Span { pub start: usize, pub end: usize }

// 书结构
pub struct Book { pub metadata: Metadata, pub entries: Vec<BookEntry> }
pub enum BookEntry { Volume(Volume), Chapter(Chapter) }
pub struct Volume { pub title: String, pub heading_span: Span, pub chapters: Vec<Chapter>, ... }
pub struct Chapter { pub title: String, pub heading_span: Span, pub body_span: Span,
                     pub origin: ChapterOrigin, pub matched_rule_id: Option<String>, ... }

// 管线输出（IPC 时直接 serde_json::to_value）
pub struct PipelineOutput {
    pub source_text: String,
    pub source_encoding: String,
    pub cleaning: Vec<CleaningAnnotation>,   // 格式清洗 + auto 水印镜像
    pub watermark: Vec<WatermarkAnnotation>, // 全部水印候选（auto + suspect）
    pub book: Book,
}

// 清洗标注（8 种 kind，snake_case IPC 锁定）
pub struct CleaningAnnotation { pub span: Span, pub kind: CleaningKind, pub replacement: Option<String> }
pub enum CleaningKind {
    BlankLineCompression, LeadingFullwidthSpace, InlineFullwidthSpace,
    ControlChar, TrailingWhitespace,
    WatermarkKeyword, WatermarkRepetition, WatermarkNonCjk,  // auto 水印镜像
}

// 水印标注
pub struct WatermarkAnnotation { pub span: Span, pub score: f32,
    pub verdict: WatermarkVerdict, pub signals: Vec<WatermarkSignal> }
pub enum WatermarkVerdict { Auto, Suspect }
pub struct WatermarkSignal { pub kind: WatermarkSignalKind, pub score: f32, pub detail: Option<String> }
pub enum WatermarkSignalKind { Repetition, NonCjkRatio, KeywordRegex, LlmAdjudication }

// 用户决策（仅本次会话，不持久化）
pub struct UserDecision { pub span: Span, pub scope: DecisionScope, pub verdict: DecisionVerdict }
```

**IPC 契约锁定规则**：
- `PipelineOutput` 字段名由测试 `pipeline_output_json_shape_is_frozen` 锁定
- `CleaningKind` 8 个变体由测试 `cleaning_kind_serializes_all_eight_variants_snake_case` 锁定
- `Chapter.paragraphs` 和 `Metadata.cover` 加 `#[serde(skip)]`，不进 IPC
- 新增字段安全；修改已有字段名必须同步更新前端和锁定测试

### 管线流程（lib.rs run_pipeline）

```
字节输入
  → encoding::decode（BOM sniff + chardetng + encoding_rs，支持 GBK/GB18030/UTF-8/UTF-16）
  → cleaning::analyze（产出 CleaningAnnotation 列表，不修改 source_text）
  → chapter::parse（多规则候选扫描 + 卷章层级归属）
  → chapter::detect_oversized_chapters（中位数 × 2.5，拆分超长章）
  → watermark::analyze（三特征 + 加权融合 + 双阈值 auto/suspect 分流）
  → watermark::merge_auto_into_cleaning（auto 水印镜像入 cleaning 列表）
  → chapter::materialize_paragraphs（physical 段落物化，cleaning 已含镜像）
  → PipelineOutput
```

### 水印检测（watermark.rs）

三特征融合，各特征独立产出 `WatermarkSignal`：

| 特征 | 方法 | 默认权重 |
|------|------|---------|
| 行频重复 | 全文行频统计，≥5 次触发 | 0.40 |
| 非中文占比 | Unicode 区块分析，CJK+标点豁免 | 0.20 |
| 关键词正则 | rules.json 中 `RuleKind::Watermark` 规则 | 0.40 |

双阈值：`score ≥ 0.70` → Auto（自动删），`0.42 ≤ score < 0.70` → Suspect（灰区）。
短行（< 10 字符）和对白行（`「」` 包围）豁免重复特征。

### 规则库（rules.rs）

- `Rule { id, pattern, kind, enabled, priority, source, description }`
- `RuleKind`：Chapter / Volume / Watermark
- `RuleSource`：Builtin / User / LlmGenerated
- JSON 文件：`%APPDATA%\Endpoint\rules.json`（用户规则，自动 merge 进内置集）
- `load_and_analyze` 启动时自动合并用户规则文件（若存在）

---

## 桥接层

**设计原则**：极薄。每个命令只做「解析参数 → `spawn_blocking` 调核心库 → 返回结果」。

**关键命令**：

| 命令 | 作用 |
|------|------|
| `load_and_analyze` | 读文件 → `run_pipeline` → 缓存 PipelineOutput → 返回 JSON DTO |
| `build_epub` | 从缓存取 pipeline → 叠加用户决策 → `build_epub_from` → 可选 kepubify |
| `adjudicate_watermarks` | 取 suspect 候选 → LLM → 升级 verdict → patch 缓存 → 返回 diff |
| `induce_watermark_rule` | 取拒绝行文本 → LLM 归纳正则 → 返回 Rule JSON |
| `save_induced_rule` | upsert 到 `rules.json` |
| `suggest_metadata` | 正文前 1 万字 → LLM → 返回书名/作者/简介建议 |
| `generate_text_cover` | 字体字节 + 书名/作者 → cover_gen → PNG 写临时文件 → `{ path, dataUrl }` |
| `list_themes` / `load_theme` | 读 resource `themes/*.css` |
| `get_llm_config` / `set_llm_config` | 读写 `config.toml` + 热替换 AppState.llm_client |

**AppState**（`src-tauri/src/state.rs`）：
- `pipeline: Mutex<Option<CachedPipeline>>`：最近一次分析结果缓存
- `cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>`：任务取消标志（接口预留，v1 未实装）
- `llm_client: Mutex<Box<dyn LlmClient>>`：启动时从 config.toml 初始化；`set_llm_config` 热替换

**进度事件**：
```jsonc
// 事件名：endpoint://progress
{ "task_id": "load-1", "stage": "decoding", "percent": 100, "detail": "GBK" }
// stage ∈ "decoding" | "cleaning" | "chapter" | "watermark" | "epub" | "kepubify"
```

**LLM 客户端**（`openai_client.rs`）：
- 实现 `LlmClient` trait（`arbitrate_watermark` / `induce_rule` / `suggest_metadata`）
- `reqwest::blocking::Client`，POST `/v1/chat/completions`，30s timeout
- 持锁调用（个人工具场景可接受，阻塞单个 tokio 线程 ≤30s）
- 未配置 API key 时 AppState 持有 `NoopLlmClient`，所有 LLM 调用静默跳过

---

## 前端

**框架**：Svelte 5（runes API：`$state` / `$derived` / `$effect`）+ Vite 6 + Tailwind CSS v4

**UI 层依赖**：
- `bits-ui` v2：无样式 headless primitives（Select / Switch / Slider / Tabs / Tooltip / Dialog / Label / Checkbox / Progress / Separator）
- `tailwind-variants`：组件变体（buttonVariants / badgeVariants / alertVariants）
- `clsx` + `tailwind-merge`：`cn()` 助手合并类名，解决冲突
- `@lucide/svelte`：图标库（按需引入，不是全量 bundle）
- `mode-watcher` v1：明暗模式状态机 + localStorage 持久化 + `<ModeWatcher />` 自动在 `<html>` 上加/去 `.dark` 类

**主题系统**（`ui/src/app.css`）：
- `@import "tailwindcss"`；自定义 `@custom-variant dark (&:where(.dark, .dark *))`
- `:root { --background ... }` / `.dark { --background ... }` 两套 OKLCH 色板
- 业务语义色变量：`--sidebar` / `--activitybar` / `--statusbar` / `--hl-cleaning` / `--hl-heading` / `--hl-volume`
- `@theme inline { --color-background: var(--background); ... }` 把变量暴露给 Tailwind，生成 `bg-background` / `text-foreground` / `bg-sidebar` 等工具类

**明暗模式三态**（`ui/src/lib/components/mode-toggle.svelte`）：
- 状态：`userPrefersMode.current` ∈ `"system" | "light" | "dark"`；`mode.current` 是系统解析后的实际值
- 切换循环：light → dark → system → light
- 图标：system 显示 Monitor，否则按 `mode.current` 显示 Sun/Moon
- 入口：ActivityBar 底部（设置图标上方）

**三栏布局**（App.svelte）：
```
[ActivityBar 56px] [Sidebar 320px] [TextView 余下宽度]
                                              ↑ 右缘附 OverviewRuler 14px
[StatusBar 24px]                                          ← 全宽底栏
```
切换阶段只换 Sidebar 内容和标注层，骨架不动。骨架颜色通过 CSS 变量随 dark 模式切换。

**关键 Store**：

| Store | 内容 |
|-------|------|
| `pipeline.svelte.js` | `{ dto, sourcePath, byteIndex }` —— 整个 PipelineOutput DTO + ByteIndex 实例 |
| `stage.svelte.js` | 当前阶段 1–4 / settings；`STAGE_DEFS` 含 lucide icon 组件引用 |
| `progress.svelte.js` | `{ stage, percent, detail, busy }` |
| `annotations.svelte.js` | `layers[]`（各阶段注入）+ `jumpTo` 信号 |
| `decisions.svelte.js` | `{ map: {[key]: "approved"|"rejected"} }` —— 水印/清洗决策 |
| `llm.svelte.js` | `{ configured, baseUrl, model, keyMasked }` |

**虚拟滚动**（VirtualText.svelte）：
- 按 `\n` 分行，每行存 char/byte 双区间
- 行高估算：`ceil(line_chars / wrapCol) × ROW_HEIGHT`（非精确，CJK 短行场景够用）
- `cumHeights`（Uint32Array）+ 二分定位可视区，O(log L)
- 多层标注：按 span 区间叠加，相同位置取先出现层的 className
- 高亮配色用 RGBA 字面量（不是 CSS 变量），dark 模式下加深透明度让高亮在暗底上仍可见

**OverviewRuler**：
- Canvas 渲染，`ctx.fillStyle` 不接受 `var(--xxx)`，所以传入 `var(--hl-cleaning)` 等 token 时会用 `getComputedStyle(document.documentElement).getPropertyValue(name)` 解析成实际色值
- `$effect` 监听 `mode.current`，模式切换时重绘

**ByteIndex.js**：
- 维护 char ↔ byte 双向索引表
- 处理 ASCII（1B）/ BMP CJK（3B）/ 代理对（4B，emoji 等）
- 6 个 assert 测试覆盖边界情况

**shadcn-svelte 风格组件**（`ui/src/lib/components/ui/`）：
- 手写在仓库内，未接 `npx shadcn-svelte add` CLI——版本升级需手动比对官方仓库
- 文件结构遵循官方约定：每组件一个目录 + `<name>.svelte` + `index.js`（命名导出）
- 调用方式：`import { Button } from "$lib/components/ui/button/index.js"` 或 `import * as Tabs from "$lib/components/ui/tabs/index.js"`

---

## 配置与数据文件

| 路径 | 用途 |
|------|------|
| `%APPDATA%\Endpoint\config.toml` | LLM 配置（base_url / model / api_key） |
| `%APPDATA%\Endpoint\rules.json` | 用户/LLM 归纳的水印规则（跨会话持久化） |
| `src-tauri/resources/fonts/*.ttf` | 嵌入字体（不进 git，用 fetch-fonts.ps1 下载） |
| `src-tauri/resources/themes/*.css` | EPUB 内 CSS 主题预设（easypub / standard / classic / highcontrast） |
| `%TEMP%\endpoint_text_cover.png` | 文字封面临时文件 |
| `localStorage["mode-watcher-mode"]` | UI 明暗模式选择（"light" / "dark" / "system"，mode-watcher 自动管理） |
