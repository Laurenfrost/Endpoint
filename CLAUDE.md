# CLAUDE.md

本文件指导 Claude Code 开发一款 Windows 桌面应用：将中文网络小说的 txt 文本转换为 epub / Kobo kepub 电子书。开发前请通读本文件，尤其是「核心设计原则」与「开发路线图」两节。

---

## 一、产品概述

一款面向个人及朋友使用的 Windows 桌面工具，把无结构的中文网络小说 txt（常达一两百万字）转换为电子书。定位是「智能默认 + 高级选项」：开箱即用的自动处理，同时为高级用户保留手动调整能力。最终阅读设备为 Kobo 电纸书，因此必须支持 kepubify 格式优化。

核心流程是一条管线：编码探测 → 文本清洗 → 章节识别 → 卷章分层 → 元数据编辑 → EPUB 封装 → kepubify 优化。

### 已确定的需求边界

纳入范围：编码自动探测与手动覆盖、文本排版清洗（空行/全角）、可维护的正则规则库做章节识别、卷章两级 TOC、元数据编辑（书名/作者/封面）、字体嵌入、CSS 自定义、本地 NLP 水印检测、可选的 LLM 兜底、kepubify 优化。

明确排除（或推迟）：繁简转换（不需要）、云端同步（与核心功能正交，推迟）、批量转换（后期迭代）、字体子集化（后期，v1 直接嵌入完整字体）。

---

## 二、技术栈

- 框架：Tauri（Rust 后端 + Web 前端）
- 后端语言：Rust
- 前端：Web 技术栈（HTML/CSS/JS 或框架，需支持虚拟滚动与区间高亮）
- kepubify：作为**外部进程**调用（pgaskin/kepubify 的 Windows 预编译二进制，随应用打包），不做原生集成
- 编码处理：Rust 的 `encoding_rs` / `golang.org/x/text` 思路对应库，自动探测 + 手动覆盖
- 嵌入字体：默认思源宋体 / 思源黑体（Source Han / Noto CJK，开源且允许嵌入，无授权问题）

### 选型理由备忘
- 纯 Windows 场景，Rust 处理两百万字文本性能与内存优势明显。
- kepubify 走外部进程调用比强行原生集成更简单，CLI 稳定。
- 思源系列字体可合法嵌入，是网文电子书最稳妥默认。

---

## 三、核心设计原则

以下原则贯穿全部模块，任何代码改动都不得违背。

### 1. 核心逻辑与界面、外部依赖彻底解耦
转换管线必须实现为一个**纯粹的、不依赖 Tauri、不依赖任何界面**的 Rust 库（crate）。它只关心「输入 txt、输出 epub」，不知道按钮、窗口、桌面应用的存在。Tauri 层只是薄胶水。好处：核心逻辑可脱离界面单独测试、可复用、可替换界面框架。

### 2. 三层架构：薄桥接 + 厚核心
- **前端层（Web UI）**：呈现与收集用户意图，不含转换逻辑。
- **桥接层（Tauri Commands）**：极薄。解析参数、调度后台线程、回传进度。每个命令理想上只是「解析参数 → 调核心库 → 返回结果」。
- **核心库层（纯 Rust crate）**：全部业务逻辑。

### 3. 模块通过共享领域模型通信，而非互相直接调用
各处理模块产出/消费「领域模型」定义的数据结构。例如章节解析产出「书」对象，EPUB 构建消费它，二者彼此不认识，只认识中间的共享类型。这种以数据为中心的解耦是单独替换、测试、并行开发的前提。

### 4. LLM 完全可选，用 trait 表达
LLM 客户端定义为可替换的接口（Rust trait）。核心库依赖「能做灰区仲裁的某种东西」这个抽象，而非具体供应商。未配 API key 时塞空实现直接跳过；测试时塞假实现免联网。**任何功能都不得以「必须有 LLM」为前提**。

### 5. 昂贵的智能只用在不确定的边缘
确定性的批量工作（编码、清洗、正则匹配、频率统计）全部本地化、零成本。LLM 只处理本地粗筛后剩下的少量灰区候选。整本书的 LLM 成本应从「按百万字计」降到「按千字计」。详见「文本智能策略」。

### 6. 核心库输出「带坐标的富标注」——这是界面的地基
核心库的输出不能只是「干净的最终文本」，而必须是**带原文坐标（字符偏移）的结构化标注**：每处清洗删除、每个章节标题、每条水印候选，都要知道自己在原始文本中的精确起止偏移、类型、来源。这份富标注同时驱动三处界面：正文高亮、侧边栏列表、概览标尺。**进入界面开发（阶段二）前，此输出契约必须冻结。**

### 7. 长任务必须可被打断
核心库的长循环要定期检查取消信号（如每处理若干行检查一次），不得「一口气闷头跑完」。v1 可不实装取消功能，但接口要预留。

---

## 四、模块划分

### 核心库内部模块（纯 Rust，与界面无关）

| 模块 | 职责 | 备注 |
|------|------|------|
| 领域模型 | 定义贯穿全程的核心数据结构 | 无逻辑，只有类型定义。所有模块的「通用语言」 |
| 编码探测 | 原始字节 → 统一文本 + 探测到的编码 | 支持 GBK/GB2312/UTF-8(BOM)/UTF-16，须可手动覆盖 |
| 预处理清洗 | 文本 → 规整文本 | 确定性格式整理（空行、全角）。与水印检测分开 |
| 章节解析 | 清洗后文本 + 规则库 → 书树 | 核心库的心脏。四阶段，详见下文 |
| 水印检测 | 文本行 → 每行可疑度分数与分类 | 多特征打分漏斗。必须能独立于 LLM 工作 |
| 规则库 | 加载/保存/增删改查规则（JSON） | 被章节解析和水印检测共享的基础设施 |
| LLM 客户端 | 封装外部大模型 HTTP API | trait 可替换接口，完全可选 |
| EPUB 构建 | 书树 + 元数据 → 标准 epub | 生成 XHTML、OPF、目录、打包 zip、嵌入字体 |
| Kobo 优化 | epub → .kepub.epub | 封装对外部 kepubify 进程的调用 |

规则库和 LLM 客户端是**被多个处理模块共享的基础设施**，因此独立成模块，不得塞进任一消费者内部。

---

## 五、领域模型设计

### 内容树：书 → 卷/章 → 段落
一本「书」包含一个**有序的条目列表**，每个条目要么是「卷」（含一组章），要么是直接挂在书下的「章」。这样无卷的书是一串章条目，有卷的是一串卷条目，混合结构（如开头楔子不属任何卷）也能自然表达。在 Rust 里用 enum（`BookEntry` 有 `Volume` 和 `Chapter` 两个变体）表达，编译器强制处理每种情况。

字段要点：
- **章（Chapter）**：标题、段落列表、`source_start` / `source_end`（在原始 txt 中的字符偏移）、出处标记（见下）。
- **卷（Volume）**：卷标题、章列表。
- **段落（Paragraph）**：纯文本，**不含任何 XHTML**。段落 → XHTML 的转换留给 EPUB 构建模块，保持领域模型与输出格式无关。

### 出处标记
每一章应附带「它是怎么来的」：正则匹配（命中哪条规则）/ 结构分析补的 / LLM 判定 / 降级切分。预览阶段据此让用户分辨高置信度章节与程序猜测的章节。

### 元数据
独立于内容树，是书对象上的独立部分。字段：书名、作者、封面（图片二进制或路径，**可选**）、语言、简介（可选）。改元数据不触碰内容树。

### 规则（JSON 存储）
本地 JSON 文件即可，无需数据库。统一规则类型 + 类型字段区分用途。每条规则字段：唯一标识、正则模式、规则类型（章节/水印/卷）、是否启用、优先级、来源（内置/用户自建/LLM 生成）、可读描述。`source` 字段对应「LLM 生成的规则回流到规则库」的设计。

---

## 六、章节解析模块（核心库的心脏）

多趟处理 + 逐步修正，分四阶段。注意：解析不是一遍扫描，而是先得到初步树、再回头修正。在 Rust 里建议「临时可变结构 → 分析后产出最终不可变树」，更符合借用检查习惯。

1. **候选行扫描**：用规则库所有章节/卷规则逐行匹配，标记疑似标题行。叠加结构约束——标题行须独占一行且足够短（去除标题模式后基本无剩余、长度在二三十字内），避免把正文里出现的「第三章」字样误判。
2. **层级归属**：把候选组织成层级，每个章归属前面最近的卷；卷之前的章（楔子/序章）挂书根下。无卷则全部挂书根。
3. **超长区间检测**：算出每章字数，用**中位数**（对离群值稳健，优于平均值）估典型长度。某章显著超长（如中位数两三倍）则怀疑内含未识别标题，在其内部找疑似标题（被空行包围的短行、位置规律），补进树或送 LLM 灰区仲裁。
4. **兜底降级**：若一个候选都没扫到，**不要硬塞 LLM 处理全文**，而是按空行或固定字数粗切，并明确标记「未能可靠识别章节结构」，让预览界面提示用户手动干预。

---

## 七、文本智能策略（水印检测 + 候选抽取）

核心认知：**大部分活儿不该交给 LLM**。两百万字整本过 LLM 既贵又没必要。统一模式是「本地廉价计算把海量正文压缩成少量候选 → LLM 只对候选做语义仲裁或规则归纳 → 规则回流本地库复用」。

### 水印检测：多特征打分漏斗
本地计算每行多个廉价特征，归一化加权融合成 0–1 可疑度，双阈值分流：
- **重复性**：行频统计（正文不重复，水印反复出现）、SimHash/MinHash 近似重复（应对变体水印）、n-gram 高频子串。
- **字符构成**：非中文字符占比（URL/域名/数字串异常高）、Unicode 区块混杂度、特殊符号检测。
- **统计特征**：行长度偏离、困惑度打分（轻量 n-gram 语言模型如 KenLM，「不像人话」的行困惑度飙高）、位置周期性。

分流：高于上阈值 → 自动判定水印直接删；低于下阈值 → 正文保留；中间灰区（通常仅几十行）→ 交 LLM 仲裁。

### LLM 当「规则生成器」，而非「逐行处理器」
让 LLM 看少量候选样本，输出识别该类水印的正则/规则，存回规则库由本地引擎批量执行。LLM 调用一次，规则可复用到以后所有书。

### 实现优先级
1. 先做：行频统计 + 非中文占比 + 关键词正则（已能干掉绝大多数真实水印，零依赖、可解释）。
2. 再做：困惑度打分（处理「不像人话但无明显关键词」的隐蔽情况）。
3. 后期/可能用不上：TF-IDF + 孤立森林/LOF 离群检测。

可解释性优先：传统方法每步都能告诉用户「这行被标记是因为出现了 87 次」或「因为 60% 是英文字符」，优于 LLM 黑盒。

---

## 八、界面设计（VS Code 式三栏）

骨架四阶段通用：最左活动栏（四个阶段图标，约 56px）+ 中间上下文侧边栏（随阶段切换）+ 右侧文本区（始终展示原始全文，处理结果作为高亮层叠加其上）。切换阶段时只换侧边栏内容和高亮色层，骨架不动。

四个阶段：
1. **文本选择**：侧边栏选文本、自动/手动调编码；文本区展示小说。
2. **文本处理**：侧边栏列出被删除（红）、待确认灰区（黄）的内容，可点击导航；文本区对应高亮。颜色语义直接映射水印检测漏斗输出（红=自动删，黄=灰区）。
3. **章节分析**：侧边栏展示章节树，可点击导航；文本区高亮章节名。
4. **样式预览与导出**：展示封面 + 前几章前几页；确认元数据后生成 epub，kepubify 为可选项。

### 关键技术点
- **右侧文本区始终展示原始全文**，各阶段只叠加不同高亮标注层（清洗层、章节层），而非每阶段重渲染处理后文本。前端持有原始文本 + 一组标注区间（起止偏移、类型、颜色）。
- **虚拟滚动**：两百万字一次性塞进 DOM 会崩。必须只渲染视口可见部分，滚动时动态替换。选前端库时把「是否方便做虚拟滚动 + 区间高亮」作为考量点。
- **概览标尺（overview ruler）**：文本区右缘一条代表全文的竖条，把所有标注按「偏移占全文百分比 × 标尺高度」画成色块。它让用户不滚动也能纵览全局分布（如章节间空隙过大 = 可能漏标题），点击跳转。它不渲染文字，只画色块，渲染无压力。与侧边栏列表互补：侧边栏是精确列表，标尺是全局空间分布。
- **同一份富标注数据，三处消费**：正文高亮（按实际位置）、侧边栏列表、概览标尺（按比例缩放）。这再次说明核心库输出契约是地基。

---

## 九、桥接层并发模型

核心矛盾：重计算（两百万字处理可达数秒）不能卡界面。

- **重计算放后台线程**：任何可能超过几十毫秒的核心库调用都声明为异步 Tauri 命令，框架放后台线程池。纯函数式、无全局可变状态的核心库天然线程安全。
- **进度回传**：后台任务通过 Tauri 事件机制不断发射**结构化**进度事件（哪个阶段 + 百分比 + 可选描述，而非裸数字），前端监听更新。异步单向，互不阻塞——这是界面不卡的根本。
- **大块结果传输**：原文只在加载阶段传一次，前端持有后，后续各阶段核心库只回传**标注数据**（小结构），不重传原文。标注用紧凑结构（几个数字 + 类型），不要把「标注 + 对应文本」一起传（文本前端凭偏移自取）。进阶优化：序列化也在后台完成，主线程只接收已序列化数据（v1 不必过早处理）。
- **取消**：前端发起任务拿到句柄，取消时通知后台，后台长循环定期检查信号。v1 可简化，接口预留。

---

## 十、EPUB 构建与字体

- EPUB 本质是 zip + XML：各章 XHTML、`content.opf`（元数据 manifest）、`toc.ncx` + `nav.xhtml`（目录）、`mimetype`、容器文件。
- **字体嵌入（v1 直接做）**：字体文件放进 epub 包，CSS 用 `@font-face` 指向，正文 `font-family` 引用。默认思源系列（合法可嵌入）。文件体积不是问题，子集化留待后期。
- **粗体**：v1 暂不处理，伪粗体可接受。
- **CSS 自定义**：epub 内部 CSS 可由用户编辑/选预设。提供几套预设主题 + 高级用户直接编辑源码。默认 CSS 可后续再调。
- **Kobo 注意**：Kobo 对 `font-family: serif;` 不会自动用 CJK 衬线字体，故 CSS 不应只依赖通用族名。Kobo 配合 kepub 比纯 epub 稳定（纯 epub 有封面缺失、崩溃的报告），故 kepubify 优化对稳定阅读有实际价值，非仅锦上添花。
- **真机验证**：生成的 kepub 必须尽早、持续地拷进真实 Kobo 设备验证。epub/kepub 兼容性问题模拟器和预览骗不了，只有真机说了算。

---

## 十一、开发路线图

**核心思路：最快路径打通端到端，再逐层加智能。** 风险自下而上递增，开发自上而下推进——总是先解决低风险、有了地基再碰高风险。每个阶段交付的都是完整可用的工具，而非半成品零件。

### 阶段零：走通最小闭环（Walking Skeleton）— ✅ 已完成

**目标（原定）**：端到端跑通一次，质量不论。读 UTF-8 txt（先不做编码探测）、最简正则切章（只认「第X章」）、套死默认 CSS、生成 epub、调 kepubify。界面简陋到只有「选文件→生成」按钮即可。

**实际交付**：
- Cargo workspace：`crates/core`（纯 Rust 核心库，零 Tauri 依赖）+ `src-tauri`（Tauri 2 应用）+ `ui/`（静态 HTML/JS，无 bundler）。
- 核心库 5 模块：`domain` / `reader` / `chapter` / `epub` / `kepubify`，14 个单元测试全部通过。
- 领域模型字段已按第五节立起来（`source_start`/`source_end`/`origin` 等），但仅在阶段零所需位置粗略填充——**真正的富标注契约冻结是阶段一末尾的任务**，并非已完成项。
- 章节识别只认「第X章」单条正则；卷不识别；整本未命中任何标题时整本作为 `ChapterOrigin::Fallback` 单章兜底。
- EPUB 3 输出：mimetype/container/OPF/NCX/nav.xhtml/章节 XHTML + 写死默认 CSS，不嵌入字体。
- kepubify 走外部进程，默认行为（`-o` 传目录，产出 `<stem>_converted.kepub.epub`）。
- 前端文件对话框走 **Rust 端 Tauri 命令**而非 plugin 的 JS（`withGlobalTauri: true` 只暴露 core API，plugin JS 在无 bundler 的静态 HTML 中不可用）。
- `src-tauri/icons/icon.ico` 是 16x16 占位图，正式打包前需替换。

**真机验证**：已通过（kepub 在 Kobo 上可正常阅读）。

### 阶段一：核心库文本处理做扎实 — ✅ 已完成

**目标（原定）**：加固确定性、低风险部分：编码探测、文本清洗、章节解析前两阶段（规则库 + 卷章层级）。**冻结富标注输出契约。**

**实际交付**：
- 核心库新增 3 模块：`encoding`（BOM sniff + chardetng 探测 + encoding_rs 解码 + 手动覆盖，GBK 探测统一升 GB18030）/ `cleaning`（产出 `CleaningAnnotation` 列表 + 按需 `apply()`，覆盖空行压缩/行尾空白/行首全角空格/控制字符）/ `rules`（`Rule`/`RuleSet`、9 条内置默认规则、JSON load/save、按优先级降序）。
- `domain` 重写并**冻结富标注契约**：`Span`（UTF-8 字节、半开区间）/ `CleaningAnnotation` / `Chapter` + `Volume` 改用 `heading_span`+`body_span`+`matched_rule_id` / `PipelineOutput`。模块顶部长 doc comment 即正式契约说明。
- `chapter` 改为两阶段流水线：多规则候选扫描 + 卷章层级组织。卷前章挂书根；卷头到首章之间若有实质内容，作为「(卷前)」Fallback 章保留入卷以免文本丢失。
- `lib.rs` 重新分层：`run_pipeline()`（给阶段二界面用）+ `build_epub_from()` + `convert()`（一站式兼容入口）；新增 `ConvertOptions`（编码覆盖 / 规则文件 / kepubify 路径）。
- `epub.rs` 跟着升级支持卷：每卷生成独立的 `volume_NNNN.xhtml` 分隔页（`h1.volume-title`），nav.xhtml/toc.ncx 改为两级嵌套，`dtb:depth` 动态算到 2。**修复了阶段零留下的 `flatten_chapters` bug——原实现会丢掉卷标题文本与 `Volume.title`**。
- `reader.rs` 删除，功能并入 `encoding.rs`。
- 桥接层最小化改动：`commands.rs` 内部使用 `ConvertOptions::default()`，Tauri 命令签名与前端代码不动。
- 53 个核心库单测全部通过。

**真机验证**：已通过（带卷的小说在 Kobo 上目录正确显示两级层级）。

**契约冻结要点**（详见 `crates/core/src/domain.rs` 顶部 doc comment）：
1. 偏移单位 = **UTF-8 字节**，不是 char 计数。
2. 端点语义 = **半开区间 `[start, end)`**，必须落在字符边界。
3. 坐标参照系 = **decoded source**（编码探测后、清洗前的字符串）。
4. 清洗以**标注列表**存在，不预先 materialize 清洗后文本——`PipelineOutput.cleaning` 与正文 paragraph 是两条并行的视图。
5. 每个 Chapter/Volume 必带 `origin` + `matched_rule_id`。
6. 阶段二之后修改契约需特别谨慎；新增字段优于改动已有字段。

### 阶段二：VS Code 式界面骨架 — ✅ 已完成

**目标（原定）**：三栏布局 + 虚拟滚动文本区 + 概览标尺 + 桥接层异步任务与进度回传。排在核心库之后：界面是核心库输出的消费者，先有稳定数据契约，界面才不返工。

**子阶段切分与冻结契约见** `docs/stage2-design.md`（阶段二跨多 session 接力开发，本文档是契约 + 子阶段表 + 真机 checklist 的唯一真相源）。

**实际交付**：
- **契约扩展（2.0）**：给 `domain.rs` 中 `Book` / `BookEntry` / `Volume` / `Chapter` / `Paragraph` / `Metadata` / `ChapterOrigin` 加 serde derive；`BookEntry` 用 `tag = "type"` 内嵌；`Chapter.paragraphs` 与 `Metadata.cover` 用 `#[serde(skip)]` 不进 IPC；新增 `pipeline_output_json_shape_is_frozen` 测试锁定字段名（任何偏离会被该测试拦下）。
- **桥接层重写（2.1）**：`src-tauri/src/state.rs` 新增 `AppState`（缓存最近一次 `PipelineOutput` + task_id 计数器 + cancel 注册表）；`commands.rs` 新增 `load_and_analyze` / `build_epub` / `cancel_task`，旧 `convert` 保留作回归保险；核心库 `lib.rs` 加 `ProgressSink` trait + `NoopSink` + `ConvertOptions.cancel_token`，`run_pipeline` / `build_epub_from` 签名追加 `progress: &dyn ProgressSink`；`cleaning.rs` / `chapter.rs` 主扫描循环加 `TODO(cancel)` 注释（取消接口预留但 v1 不实装）。
- **前端工程链（2.2）**：`ui/` 转为 Vite 6 + Svelte 5 项目；`tauri.conf.json` 接 `beforeDevCommand` / `beforeBuildCommand` / `devUrl` / `frontendDist: "../ui/dist"`；窗口尺寸调到 1280×800。
- **VS Code 式三栏（2.3-2.5）**：`App.svelte` = 标题栏（含全局进度条）+ ActivityBar（56px，四阶段图标）+ Sidebar（320px，按 stage 切组件）+ TextView；`stores/` 4 个 svelte runes store（pipeline / stage / progress / annotations）；`text/ByteIndex.js` 处理 ASCII / BMP CJK / 代理对全谱 + 6 个 assert 测试；`text/VirtualText.svelte` 行级虚拟滚动（按 `\n` 分行 + 动态 wrapCol 估算行高 + cumHeights 二分定位 + 多层 ann 叠加渲染）；`text/OverviewRuler.svelte` Canvas DPR 自适应右缘标尺。
- **四阶段 UI**：Stage1 文件 + 编码探测 / 手动覆盖 → 自动跳阶段 2；Stage2 红色清洗高亮 + 侧边栏列表 + kind chips + 上下文预览 + 懒加载 200/批，灰区/可疑列表保留空占位（阶段三填）；Stage3 卷-章双级树（折叠/展开）+ stats chips + origin badge + 三层 ann 叠加（红清洗 + 蓝章 + 绿卷）；Stage4 元数据表单 + kepubify 可选，正式调 `build_epub`。
- **进度回传**：`endpoint://progress` 事件 `{ task_id, stage, percent, detail? }`，stage ∈ `decoding`/`cleaning`/`chapter`/`epub`/`kepubify`；App.svelte 全局订阅。
- **共享富标注架构**：`annotations` store 同时驱动 VirtualText / Sidebar / Stage 列表 / OverviewRuler——契约第 6 条「同一份富标注，三处消费」的兑现点。
- **测试**：54 个核心库单测 + 6 个 ByteIndex 测试全绿；`cargo check --workspace` 干净；`npm run build` 产物 62KB JS / 10KB CSS（gzip 24KB / 2.4KB）。

**真机验证**：已通过（`docs/stage2-design.md` 第六节 checklist 全项过，含 200 万字 GBK 网文加载/滚动/卷-章跳转/三层标尺色块/EPUB+kepub 出书）。

**已知限制**：
- VirtualText 行高靠估算，长段落实际渲染可能略超估算槽位（CJK 网文中肉眼难察；若后续发现明显问题再加 ResizeObserver 二次校正）。
- Cancel 接口预留但未实装（核心库长循环只挂 TODO 注释）。

### 阶段三：本地水印智能 + 用户控制权 — ✅ 已完成(v1 + v2)

**v1(3.0-3.4)目标**：核心库填本地水印检测(行频 / 非中文占比 / 关键词正则三特征 + 加权融合 + 双阈值分流);Stage2 把"灰区/可疑(占位)"填实成 auto(橙)+ suspect(黄)+ tab + signals 卡片;auto 自动镜像到 cleaning,EPUB 自然扣除;suspect 保留在 EPUB 等待用户决策。
**详情** `docs/stage3-design.md`(子阶段 + 契约 + 真机 checklist 真相源)。

**v2(v2.0-v2.2)目标(基于 v1 真机反馈)**:补全"高级选项 / 手动调整"那一半——v1 只做了"智能默认",没给用户控制权。v2 把:cleaning 策略拆细 + 默认全关 + 用户主动开;watermark 精度修复(对白行豁免、CJK 标点扩展、min_line_chars=10、suspect_threshold=0.42);每张候选卡片接受/拒绝按钮(三态)+ 批量栏 + 决策叠加到 EPUB 输出。
**详情** `docs/stage3-v2-design.md`(决策表 + 子阶段 + 实际交付)。

**实际交付**:
- 核心库新增 `watermark` 模块:`analyze`(三特征 + 双阈值)/ `merge_auto_into_cleaning`(auto → cleaning 镜像)/ `apply_user_decisions`(v2.2 用户决策叠加)/ `WatermarkConfig`(可序列化,默认值 v2 调过)
- 核心库 `cleaning` 拆细:5 个 kind(BlankLineCompression / Leading 与 Inline 拆出的 FullwidthSpace / ControlChar / TrailingWhitespace)+ `CleaningConfig`(每 kind 一个 enabled,默认全关);TrailingWhitespace 字符集扩到含 `\r` / NBSP / 零宽
- 核心库 `chapter::parse` 拆分:只识别边界,`materialize_paragraphs` 独立成函数(被 watermark::analyze 之后调,让镜像与决策都能影响 paragraphs)
- 领域模型契约扩展:`CleaningKind` 8 项(原 `fullwidth_space` 消失,拆 leading/inline + 加 3 项 watermark 镜像)/ `WatermarkAnnotation` / `WatermarkSignal` / `WatermarkVerdict` / `WatermarkSignalKind` / `UserDecision` / `DecisionScope` / `DecisionVerdict`
- 规则库:`builtin_rules()` 加 7 条内置 `RuleKind::Watermark` 规则(URL / www / TG / 域名 / 首发 / 盗版 / 免费阅读)
- 桥接层:`load_and_analyze` 加 `cleaning_config` + `watermark_config` 参数;`build_epub` 加 `decisions` 参数(后端调 `apply_user_decisions` + 重 materialize 后再 EPUB)
- 前端 Stage2:策略折叠面板(5 checkbox)+ 阈值高级折叠面板(7 滑块)+ 三层标注(红/橙/黄)+ 水印列表(tab + 卡片 + signals 进度条)+ 卡片三态按钮 `[✓接受][✗拒绝]`(同按钮再点 = 取消)+ 批量栏 + reload 提示
- 前端 Stage4:build 时把 `decisions` 序列化传给 `build_epub`,按钮下方显示决策计数
- 进度事件 stage 枚举扩到 6 项(`decoding` / `cleaning` / `chapter` / `watermark` / `epub` / `kepubify`)
- 测试统计:118 个核心库测试 + 6 个 ByteIndex 前端测试全绿;`cargo check --workspace` 干净;`npm run build` 干净
- 真机验证:v1 + v2 已通过

**v2 起的关键设计变化**(后续阶段必须遵守):
1. **"智能默认 = 不动用户文本"**:`CleaningConfig::default()` 全关,清洗策略改为用户主动开启(详见 `docs/stage3-v2-design.md` 第二节决策 10)
2. **单特征不再触发 suspect**:`WatermarkConfig::default().suspect_threshold = 0.42`,需要两个独立特征(详见决策 11)
3. **`fullwidth_space` snake_case 已消失**:拆为 `leading_fullwidth_space`(段首)+ `inline_fullwidth_space`(行内连续 ≥2)。v2 唯一一处刻意契约破坏
4. **用户决策仅本次会话**:不持久化,reload 即丢;持久化版本(规则回流)是阶段四的事

### 阶段四:LLM 兜底 + 规则回流 + 导出精修

**目标**:把"前面几阶段刻意推迟的高风险 / 可选 / 重 UI"功能补齐。LLM 是可选增强——所有功能必须保持**没有 LLM 也能完整工作**(CLAUDE.md 第三节第 4 条)。

**核心方向**(按从底到上、低风险到高风险排序):

1. **LLM 客户端模块**(纯 Rust trait + 1-2 个具体实现 + 配置入口)
   - trait `LlmClient`,核心库依赖此抽象;`NoopLlmClient` 实现给"未配 API key"场景
   - 至少接入 Anthropic Claude(用户使用频次最高);OpenAI 兼容接口可二期
   - API key 存放:OS keychain 优先,fallback 到本地配置文件(明文 + 警告)
   - 前端 Stage 1 或独立设置面板给"配置 LLM 提供商 + API key + 模型选型"入口

2. **LLM 灰区仲裁**(消费 v2 的 suspect 列表)
   - 把 watermark suspect 行 + 上下文 batch 发给 LLM,返回 verdict(是水印 / 是正文)+ 解释
   - 章节解析的超长区间补漏(原 CLAUDE.md 第六节阶段三描述但 v1/v2 未做)
   - 元数据抓取(从前 N 章正文里推断书名 / 作者,与用户填的元数据比对)

3. **规则回流**(把决策抽象成可复用规则)
   - 用户决策(尤其 watermark 拒绝)经 LLM 归纳 → 生成 `RuleKind::Watermark` 规则
   - 存回 `RuleSet.json`(`source: RuleSource::LlmGenerated`),下次扫描自动跳过同类
   - 把"仅本次会话决策"升级为"持久化偏好"

4. **导出精修**
   - **字体嵌入**:默认思源宋体 / 思源黑体(Source Han / Noto CJK)放进 epub zip + CSS `@font-face`(完整字体先,子集化推后)
   - **CSS 主题预设**:3-5 套预设(标准 / 古风 / 高对比度 / ...)+ 高级用户可直接编辑源码
   - **封面 UI**:Stage4 加封面图片选择 / 预览 / 进 EPUB manifest;`Metadata.cover: Option<Vec<u8>>` 已预留
   - **元数据编辑增强**:简介(Description)/ 出版社 / ISBN 等可选字段
   - **kepubify 选项暴露**:目前只能"开/关",可考虑暴露常用 CLI flags

5. **测试样本回流 + 微调默认**
   - 用户多本网文实测后,如发现误删率 / 漏删率不平衡,微调 `WatermarkConfig::default()` 与
     `builtin_rules()` 中的 watermark 规则集
   - 阈值默认调整记录回写 `docs/stage3-v2-design.md` 第二节决策表(沿用 v2 决策 10/11 风格)

**已知技术债**(可在阶段四顺手清,也可不动):
- VirtualText 行高靠估算,长段落实际渲染可能略超估算槽位——若用户没遇到明显问题,可不加 ResizeObserver
- Cancel 接口预留但未实装(`TODO(cancel)` 散落核心库)——若没遇到需要中断的场景,可不补
- 字体子集化(原约定 v1 / v2 不做,体积大但简单)

**禁区**:
- 不动阶段二契约(`Span` / `Chapter.heading_span` / `Volume.heading_span` / `BookEntry` 枚举形状)
- 不动阶段三 v2 契约(`CleaningKind` 8 项 / `WatermarkAnnotation` / `UserDecision` 3 类型)
- 不动 `VirtualText` / `OverviewRuler` / 三栏骨架核心交互(Stage4 内部可改、Stage1/2/3 不动)
- LLM 不得变为强依赖——所有功能必须可降级到"未配 API key"路径

---

## 十二、给 Claude Code 的工作约定

- 动手前确认当前处于路线图哪个阶段,不要跨阶段实现未到的功能。**阶段三 v2 已完成,当前在阶段四**。
- 任何核心库代码不得 import Tauri 或前端相关依赖,保持核心库可独立编译与测试。
- 为核心库的每个模块编写单元测试,尤其编码探测、章节解析、水印检测——这些逻辑密集且易回归。
- **富标注输出契约**:
  - 阶段一末冻结(详见 `crates/core/src/domain.rs` 模块顶部 doc comment)
  - 阶段三 v1 扩展:`PipelineOutput.watermark` / `CleaningKind` 加 3 项 `Watermark*`
  - 阶段三 v2 扩展:`CleaningKind` 8 项(`fullwidth_space` 已消失,拆为
    `leading_fullwidth_space` + `inline_fullwidth_space`);新增 `UserDecision` /
    `DecisionScope` / `DecisionVerdict`
  - 任何对**已有字段**的改动必须先呈用户。新增字段更安全。
  - `pipeline_output_json_shape_is_frozen` 测试 +
    `cleaning_kind_serializes_all_eight_variants_snake_case` 等是 IPC 契约的代码级锚点,
    任何偏离会被这些测试拦下,**不要**调整测试期望"让它过"。
- 涉及 EPUB 构建的改动,提示用户做真机验证。
- 遵循「智能默认 + 高级选项」:每个自动化功能都应有手动覆盖入口。
  - **v2 起更激进**:默认 = "不动用户文本"(`CleaningConfig::default()` 全关、
    `WatermarkConfig::suspect_threshold = 0.42` 让单特征不再触发)。用户必须主动
    开启策略 / 接受候选才生效。新功能应继承这种"用户优先"的默认哲学。
- **LLM 不得变为强依赖**(CLAUDE.md 第三节第 4 条):核心库的 LLM 客户端是 trait,
  未配 API key 时塞 `NoopLlmClient`,所有依赖 LLM 的功能(灰区仲裁 / 规则回流 /
  元数据抓取)都必须可降级到"无 LLM 路径"。
- **用户决策语义**(阶段三 v2.2 起):决策仅本次会话有效(`UserDecision` 不持久化);
  持久化版本是规则回流——**只在阶段四做**——把 reject 模式抽象为
  `RuleKind::Watermark` 规则 + `source: LlmGenerated`,存回 `RuleSet.json`。
- 提交信息和注释使用中文或英文均可,保持与现有代码一致。
- **不动阶段二骨架**:`VirtualText` / `OverviewRuler` / `ActivityBar` / `Sidebar` /
  四阶段切换的核心交互。阶段三 v2 末尾这些组件已稳定,阶段四的新功能(封面预览 /
  CSS 编辑等)应该作为 Stage4 / 新设置面板的**内部组件**实现,不动外壳。
