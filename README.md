# Endpoint

将中文网络小说 `.txt` 文件转换为 EPUB / Kobo kepub 电子书的 Windows 桌面工具。

## 功能

- **编码自动识别**：支持 UTF-8、GBK/GB18030、UTF-16，可手动覆盖
- **格式清洗**：空行压缩、全角缩进、控制字符、行尾空白（默认全关，用户主动开启）
- **水印检测**：基于行频统计 + 非中文占比 + 关键词正则的本地三特征融合，自动 / 灰区双阈值分流
- **章节识别**：正则规则库（第X章 / 第X回 / 第X卷 等），支持卷章两级 TOC
- **超长章节拆分**：中位数算法自动识别遗漏标题
- **导出精修**：自定义 CSS 主题（EasyPub / 标准 / 古风 / 高对比度）、封面图片嵌入、文字封面自动生成、字体嵌入（霞鹜文楷 opt-in）
- **LLM 增强（可选）**：水印灰区仲裁、元数据建议、规则归纳持久化；未配置 LLM 时所有功能正常工作
- **kepubify 优化**：调用外部 kepubify 生成 Kobo 优化格式
- **明暗模式**：UI 支持浅色 / 深色 / 跟随系统三态切换（活动栏底部按钮，自动持久化）

## 环境要求

- Windows 10/11（x64）
- [Rust](https://rustup.rs/) stable toolchain
- [Node.js](https://nodejs.org/) 18+
- [Tauri CLI](https://tauri.app/start/prerequisites/)：`cargo install tauri-cli`
- （可选）字体文件：运行 `scripts/fetch-fonts.ps1` 下载内置霞鹜文楷
- （可选）[kepubify](https://pgaskin.net/kepubify/) Windows 二进制，放到任意位置后在应用内指定路径

## 开发运行

```powershell
# 下载内置字体（字体嵌入功能需要）
.\scripts\fetch-fonts.ps1

# 开发模式（热重载）
cargo tauri dev

# 构建发行版
cargo tauri build
```

## 使用流程

1. **阶段 1 · 选择文件**：选择 `.txt` 文件，确认或手动覆盖编码
2. **阶段 2 · 文本处理**：查看清洗标注（红）和水印候选（橙/黄），可按需开启清洗策略，对灰区候选逐条接受/拒绝
3. **阶段 3 · 章节分析**：查看卷-章结构，验证识别结果
4. **阶段 4 · 导出**：填写书名/作者，选择封面和主题，生成 EPUB（可选 kepubify 转 kepub）

## LLM 配置（可选）

点击活动栏底部的 ⚙ 图标进入设置面板，填写兼容 OpenAI 接口的 base_url / model / API key（如 DeepSeek）。配置后可在阶段 2 使用「询问 LLM」仲裁水印灰区，以及在阶段 4 使用「从正文建议」自动填写元数据。同一面板下还能配置可选的 Brave 搜索后端和 kepubify.exe 路径。

## 项目结构

```
crates/core/          纯 Rust 核心库（不依赖 Tauri）
src-tauri/            Tauri 桥接层（命令、状态、LLM 客户端）
ui/                   Svelte 5 + Vite + Tailwind v4 + shadcn-svelte 风格组件
scripts/              辅助脚本（fetch-fonts.ps1）
docs/                 架构文档与变更日志
```
