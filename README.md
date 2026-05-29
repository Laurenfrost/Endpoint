# Endpoint

将中文网络小说 `.txt` 文件转换为 EPUB / Kobo kepub 的 Windows 桌面工具。

**核心理念**：智能默认，不动原文。所有清洗策略默认关闭，用户主动开启；LLM 功能完全可选，未配置时一切正常运行。

---

## 功能

- **编码自动识别** — 支持 UTF-8、GBK / GB18030、UTF-16（BOM / chardetng 双重探测），可手动覆盖
- **格式清洗** — 空行压缩、全角缩进、控制字符、行尾空白（标注预览后按需开启）
- **水印检测** — 行频统计 + 非中文占比 + 关键词正则三特征本地融合，Auto / 灰区双阈值分流，支持 LLM 仲裁
- **章节识别** — 正则规则库（第X章 / 第X回 / 第X卷等），支持卷章两级 TOC，超长章节中位数算法自动补标题
- **EPUB 构建** — 自定义 CSS 主题（EasyPub / 标准 / 古风 / 高对比度）、封面图片嵌入、文字封面自动生成、字体嵌入
- **kepubify 优化** — 调用外部 kepubify 生成 Kobo 优化的 `.kepub.epub`
- **LLM 增强（可选）** — 水印灰区仲裁、元数据建议（书名 / 作者 / 简介）、水印规则归纳持久化
- **明暗模式** — UI 支持浅色 / 深色 / 跟随系统三态切换，自动持久化

---

## 使用流程

```
阶段 1 · 选择文件   →   选择 .txt，确认或覆盖编码
阶段 2 · 文本处理   →   查看清洗/水印标注，按需开启策略，对灰区水印逐条决策
阶段 3 · 章节分析   →   验证卷章结构，查看 TOC 预览
阶段 4 · 导出       →   填写书名/作者，选封面和主题，生成 EPUB（可选 kepubify）
```

---

## 环境要求

| 依赖 | 说明 |
|---|---|
| Windows 10 / 11 x64 | 运行环境 |
| [Rust](https://rustup.rs/) stable | 编译核心库与 Tauri 壳 |
| [Node.js](https://nodejs.org/) 18+ | 前端构建 |
| [Tauri CLI v2](https://tauri.app/start/prerequisites/) | `cargo install tauri-cli` |
| kepubify（可选） | [pgaskin.net/kepubify](https://pgaskin.net/kepubify/)，用于生成 Kobo 格式 |

---

## 开发

```powershell
# 1. 下载内置字体（封面生成需要）
.\scripts\fetch-fonts.ps1

# 2. 安装前端依赖
cd ui; npm install; cd ..

# 3. 开发模式（Vite 热重载 + Tauri 窗口）
cargo tauri dev

# 4. 运行核心库测试
cargo test --workspace

# 5. 构建发行版
cargo tauri build
```

仅调前端时（无需 Tauri 窗口）：

```powershell
cd ui
npm run dev    # 开发服
npm run build  # 编译检查
```

---

## LLM 配置（可选）

点击活动栏底部 ⚙ 图标，填写兼容 OpenAI 接口的服务信息：

| 字段 | 说明 |
|---|---|
| base_url | 接口地址，如 `https://api.deepseek.com` |
| model | 模型名，如 `deepseek-chat` |
| API Key | 明文存于 `%APPDATA%\Endpoint\config.toml`，仅限本机 |

支持任何 OpenAI 兼容服务（DeepSeek、本地 Ollama、OpenAI 等）。未配置时所有 LLM 功能静默跳过，不影响基础流程。

同一面板可配置可选的 Brave 搜索后端（补全冷门作品元数据）和 kepubify.exe 路径。

---

## 数据文件

| 路径 | 用途 |
|---|---|
| `%APPDATA%\Endpoint\config.toml` | LLM / 搜索配置 |
| `%APPDATA%\Endpoint\rules.json` | 用户 / LLM 归纳的水印规则（跨会话持久化） |
| `src-tauri/resources/fonts/` | 内置字体（不进 git，`fetch-fonts.ps1` 下载） |
| `src-tauri/resources/themes/` | EPUB 内 CSS 主题预设 |

---

## 项目结构

```
crates/core/     纯 Rust 核心库（零 Tauri 依赖）
src-tauri/       Tauri 桥接层（命令、状态、LLM 客户端）
ui/              Svelte 5 + Vite 6 + Tailwind CSS v4 前端
scripts/         辅助脚本（fetch-fonts.ps1）
docs/            架构文档与变更日志
```

技术栈：Tauri 2 · Rust · Svelte 5 · Tailwind CSS v4 · bits-ui

详细架构见 [`docs/architecture.md`](docs/architecture.md)。

---

## 字体

封面生成使用 [思源宋体 CN](https://github.com/adobe-fonts/source-han-serif)（Adobe，SIL OFL 1.1）。字体文件不含于本仓库，运行 `scripts/fetch-fonts.ps1` 下载。

---

## 许可证

[MIT](https://opensource.org/licenses/MIT) OR [Apache-2.0](https://opensource.org/licenses/Apache-2.0)
