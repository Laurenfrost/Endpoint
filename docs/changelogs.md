# 变更日志

记录各阶段的需求调整、关键设计决策和实际交付内容。

---

## 阶段零：Walking Skeleton（最小闭环）

**目标**：端到端跑通，质量不论。读 UTF-8 txt → 最简正则切章 → 套默认 CSS → 生成 EPUB → 调 kepubify。

**实际交付**：
- Cargo workspace：`crates/core`（纯 Rust，零 Tauri 依赖）+ `src-tauri` + `ui/`（静态 HTML）
- 核心库 5 模块：domain / reader / chapter / epub / kepubify；14 个单元测试
- 章节识别只认「第X章」单条正则；整本未命中时整本作为 Fallback 单章
- EPUB 3 输出：mimetype / container / OPF / NCX / nav.xhtml / 章节 XHTML + 默认 CSS
- kepubify 外部进程调用，`-o` 传目录，产出 `<stem>_converted.kepub.epub`
- 文件对话框走 Rust 端 Tauri 命令（`withGlobalTauri: true` 只暴露 core API，无 bundler 的静态 HTML 无法用 plugin JS）

**真机验证**：通过（kepub 在 Kobo 上可正常阅读）

---

## 阶段一：核心库文本处理

**目标**：加固确定性低风险部分——编码探测、文本清洗、章节解析前两阶段（规则库 + 卷章层级）。冻结富标注输出契约。

**实际交付**：
- 核心库新增 3 模块：
  - `encoding`：BOM sniff + chardetng 探测 + encoding_rs 解码 + 手动覆盖（GBK 探测升 GB18030）
  - `cleaning`：产出 `CleaningAnnotation` 列表（不预先修改文本），覆盖空行压缩/行尾空白/行首全角/控制字符
  - `rules`：`Rule`/`RuleSet`、9 条内置默认规则、JSON load/save、按优先级降序
- `domain.rs` 重写并**冻结富标注契约**：`Span`（UTF-8 字节半开区间）/ `CleaningAnnotation` / `Chapter.heading_span` + `body_span` + `origin` + `matched_rule_id` / `PipelineOutput`
- 章节两阶段：多规则候选扫描 + 卷章层级归属；卷前章挂书根；卷头到首章之间若有实质内容作为「(卷前)」Fallback 章保留
- `epub.rs` 升级支持卷：卷 XHTML 分隔页 + nav.xhtml/toc.ncx 两级嵌套；修复阶段零 `flatten_chapters` bug（原实现丢卷标题）
- 53 个核心库单测全绿

**真机验证**：通过（带卷小说在 Kobo 上目录正确显示两级层级）

**契约冻结要点**（详见 `crates/core/src/domain.rs` 顶部 doc comment）：
1. 偏移单位 = UTF-8 字节，非 char 计数
2. 端点语义 = 半开区间 `[start, end)`，必须落在字符边界
3. 坐标参照系 = decoded source（编码探测后、清洗前）
4. 清洗以标注列表存在，不预先 materialize 清洗后文本
5. 每个 Chapter/Volume 必带 `origin` + `matched_rule_id`

---

## 阶段二：VS Code 式界面骨架

**目标**：三栏布局 + 虚拟滚动文本区 + 概览标尺 + 桥接层异步任务与进度回传。

**关键决策**：
- UI 框架选 **Svelte 5 + Vite**（编译期蒸发虚拟 DOM，runes 适合"一份富标注驱动三处视图"场景）
- `withGlobalTauri: true` + 直接调 `window.__TAURI__.core.invoke`，无需 @tauri-apps/api 包
- 大块 PipelineOutput 只在加载阶段传一次，后续阶段只传标注数据（小结构）
- 虚拟滚动：按行估算高度 + cumHeights Uint32Array + 二分定位，不做精确 ResizeObserver（CJK 网文短行场景够用）
- VirtualText 行内多层标注：线性区间合并，相同位置取先出现层

**实际交付**：
- `ui/` 转为 Vite 6 + Svelte 5 项目；`tauri.conf.json` 接 beforeDevCommand/devUrl/frontendDist
- 三栏骨架：ActivityBar（56px）+ Sidebar（320px）+ TextView；切换阶段只换 Sidebar
- ByteIndex.js：char ↔ byte 双向索引，处理 ASCII/BMP CJK/代理对，6 个测试
- VirtualText.svelte：行级虚拟滚动 + 多层标注叠加渲染
- OverviewRuler.svelte：Canvas DPR 自适应，色块 + 点击跳转
- 四阶段 UI：Stage1 文件加载/编码；Stage2 清洗红色高亮；Stage3 卷章树；Stage4 元数据+导出
- 进度事件：`endpoint://progress`，6 个 stage 值
- IPC 契约测试：`pipeline_output_json_shape_is_frozen`（字段名偏离即失败）

**真机验证**：通过（200 万字 GBK 网文加载/滚动/卷章跳转/三层标尺色块/EPUB+kepub 出书）

---

## 阶段三 v1：本地水印检测

**目标**：核心库填本地水印检测；Stage2 把灰区占位填实；auto 自动镜像到 cleaning，EPUB 自然扣除。

**实际交付**：
- 核心库新增 `watermark` 模块：三特征（行频 / 非中文占比 / 关键词正则）+ 加权融合 + 双阈值分流
- `CleaningKind` 新增 3 个水印镜像变体：`watermark_keyword` / `watermark_repetition` / `watermark_non_cjk`
- `PipelineOutput.watermark: Vec<WatermarkAnnotation>` 新字段（不动既有字段）
- auto 水印通过 `merge_auto_into_cleaning` 镜像入 cleaning，EPUB 构建时自动扣除
- 规则库加 7 条内置水印规则（URL / www / TG / 域名 / 首发 / 盗版 / 免费阅读）
- Stage2 三层标注（红/橙/黄）+ 水印列表（tab + 卡片 + signals 进度条）

**真机验证**：通过

---

## 阶段三 v2：用户控制权

**缘起（真机反馈）**：v1 只有"智能默认"，缺"高级选项"——段首全角被误删、水印误判率高（省略号"……"被计非中文、对白"嗯"重复被计行频）、无法手动覆盖检测结果。

**关键决策**：

| 编号 | 决策 | 理由 |
|------|------|------|
| D10 | `CleaningConfig::default()` 全关 | "智能默认 = 不动用户文本"；清洗策略改为用户主动开启 |
| D11 | `suspect_threshold` 从 0.35 升到 0.42 | 单特征（得分约 0.40）不再触发灰区，需要两个独立特征 |
| D12 | `fullwidth_space` 拆为 `leading_fullwidth_space` + `inline_fullwidth_space` | 段首全角是中文写作惯例，不应默认删除 |
| D13 | 用户决策仅本次会话，不持久化 | 简单够用；持久化版本（规则回流）放到阶段四 |

**实际交付**：
- `cleaning` 拆细为 5 种 kind + `CleaningConfig`（每 kind 一个 enabled 开关，默认全关）
- `TrailingWhitespace` 字符集扩展：含 `\r` / NBSP / 零宽字符
- `min_line_chars` 从 4 升到 10；CJK 标点扩展豁免（`…` `—` 等不计非中文）；对白行豁免重复特征
- 水印阈值高级 UI（7 个滑块）+ 清洗策略面板（5 个 checkbox）
- 每张候选卡片三态决策按钮 `[✓接受][✗拒绝]` + 批量栏
- `build_epub` 命令接收 `decisions` 参数，后端调 `apply_user_decisions` + 重 materialize

**真机验证**：通过

---

## 阶段四：导出精修 + LLM 兜底 + 规则回流

### 4.0：封面嵌入 + CSS 覆盖接口

- `epub.rs` 重构为 `EpubOptions`（css_override / cover / font_bytes）
- `pick_cover_file` 命令：弹出图片对话框，读取字节，返回 `{ path, dataUrl }`
- Stage4 封面选择/预览/清除

### 4.1：字体嵌入 opt-in

- 内置霞鹜文楷（LXGWWenKai-Regular.ttf，fetch-fonts.ps1 下载）+ 用户自定义路径
- EPUB 中 `OEBPS/fonts/` + `@font-face` CSS + OPF manifest
- Stage4 字体勾选框（默认不勾）

### 4.2：CSS 主题预设 + 编辑器

- 3 套主题 CSS（standard / classic / highcontrast）存 `src-tauri/resources/themes/`
- `list_themes` / `load_theme` Tauri 命令
- Stage4 主题单选栏 + 高级 CSS 折叠编辑器；手动编辑后进入"自定义"态

### 4.3：文字封面自动生成

- `crates/core/src/cover_gen.rs`：`image` + `ab_glyph`，1400×2100 PNG，两种渐变风格
- Tauri 命令写临时文件 + 返回 data URL
- Stage4 无封面时显示"生成文字封面"选项

### 4.4：超长章节结构检测

- `detect_oversized_chapters`：中位数 × 2.5 阈值，空行包围短行（≤30字）或命中章节规则 → 拆分
- 新章 `origin = Structural`
- 无需前端改动，Stage3 树自动展示

### 4.5：LLM 客户端 + 配置 UI

**关键决策**：
- LLM 客户端在核心库只放 `LlmClient` trait + `NoopLlmClient`；HTTP 实现（reqwest）在桥接层，避免核心库引入网络依赖
- 使用 OpenAI 兼容接口（`/v1/chat/completions`），DeepSeek 优先，不绑定特定厂商
- API key 存 `%APPDATA%\Endpoint\config.toml` 明文（个人工具，不做 OS keychain）
- 持锁调用 LLM（阻塞单个 tokio 线程 ≤30s，个人工具可接受；spawn_blocking 移动 MutexGuard 在 Rust 中不可行）

**实际交付**：
- `crates/core/src/llm.rs`：`LlmClient` trait（3 方法）/ `NoopLlmClient` / `AdjudicationVerdict { IsWatermark{reason}, IsContent, Uncertain }` / `MetadataSuggestion` / `WatermarkCandidate`
- `domain.rs` 新增 `WatermarkSignalKind::LlmAdjudication`（snake_case：`llm_adjudication`）
- `src-tauri/src/openai_client.rs`：reqwest blocking 实现
- `src-tauri/src/llm_config.rs`：config.toml 读写 + `user_rules_path()`
- `AppState.llm_client: Mutex<Box<dyn LlmClient>>`，startup 初始化，`set_llm_config` 热替换
- ActivityBar 底部 LLM 状态圆点（灰/绿）

### 4.6：LLM 元数据建议

- `suggest_metadata` 命令：正文前 1 万字 → LLM → 解析「书名:/作者:/简介:/封面关键词:」
- `NotConfigured` 静默返回 null（不报错）
- Stage4 "从正文建议 ▸" 按钮 + 建议面板（逐字段独立"采用"）

### 4.7：LLM 水印仲裁

**关键决策**：
- `AdjudicationVerdict` 选择 `IsWatermark { reason: String }` 而非 unit 变体，让 reason 显示在 signals detail
- 前端原地 patch `pipeline.dto`（不重新 `load_and_analyze`），Svelte 5 深层响应自动更新

**实际交付**：
- `adjudicate_watermarks` 命令：取 suspect 行 + 上下文 → LLM → IsWatermark 升 Auto + LlmAdjudication signal + CleaningAnnotation → patch 缓存 → 返回 diff `{ updated_watermarks, new_cleaning }`
- Stage2 批量"询问 LLM ▸"按钮（tab 栏右侧）+ 每张 suspect 卡片"?"单条按钮
- `applyAdjudicationResult`：原地 splice `pipeline.dto.watermark/cleaning`

### 4.8：规则回流

**关键决策**：
- 规则文件路径 `%APPDATA%\Endpoint\rules.json`（与 config.toml 同目录，非 data_local_dir）
- `load_and_analyze` 自动合并用户规则文件（若存在），无需用户重启或额外操作

**实际交付**：
- `induce_watermark_rule`：拒绝行文本 → LLM 归纳正则 → 编译校验 → 返回 Rule JSON
- `save_induced_rule`：upsert 到 `rules.json`
- `load_and_analyze` 自动传入 `rules_path: user_rules_path().filter(|p| p.exists())`
- Stage2 水印区底部"归纳规则 ▸"按钮（统计 rejected 决策数）+ 规则预览面板（pattern + description）+ "保存规则"

---

## 已知技术债

| 项目 | 状态 | 说明 |
|------|------|------|
| VirtualText 行高估算 | 保留 | 估算非精确，CJK 短行场景未发现明显问题 |
| Cancel 接口 | 预留未实装 | `TODO(cancel)` 散落核心库，长循环未检查信号 |
| 字体子集化 | 推迟 | 嵌入完整字体体积约 16MB，合法但较大 |
| OpenAI 以外的 provider | 未做 | Anthropic 原生 API 等，目前只支持 OpenAI 兼容接口 |
