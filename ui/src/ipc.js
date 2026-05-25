// Tauri 2 IPC 薄封装。`withGlobalTauri: true` 已在 tauri.conf.json 启用,
// 所以可以直接走 window.__TAURI__.core,不需要 @tauri-apps/api 包。
//
// 也支持非 Tauri 环境(纯浏览器跑 npm run dev)时给出明确报错,避免一片 undefined。

function ensureTauri() {
  if (typeof window === "undefined" || !window.__TAURI__) {
    throw new Error(
      "未检测到 Tauri 运行时:请使用 `cargo tauri dev` 启动,而非直接在浏览器打开。"
    );
  }
  return window.__TAURI__;
}

export async function invoke(cmd, args) {
  return ensureTauri().core.invoke(cmd, args);
}

/// 监听 Tauri 事件,返回一个 unlisten 函数。
export async function listen(event, handler) {
  return ensureTauri().event.listen(event, handler);
}

// —— 命令封装(签名锁定在 docs/stage2-design.md 第四节) ——

export const pickInputFile = () => invoke("pick_input_file");
export const pickOutputFile = (defaultPath) =>
  invoke("pick_output_file", { defaultPath: defaultPath ?? null });
export const pickExecutableFile = () => invoke("pick_executable_file");
export const pickCoverFile = () => invoke("pick_cover_file");
export const pickFontFile = () => invoke("pick_font_file");

/// 加载 txt + 跑完整管线。
/// `cleaningConfig`:阶段三 v2 新增,前端清洗策略面板的勾选状态(可选,缺省走 default)。
///   形状:{ blank_line_compression, leading_fullwidth_space, inline_fullwidth_space,
///          control_char, trailing_whitespace } —— 都是 bool;后端 #[serde(default)]
///          允许只传部分字段。
/// `watermarkConfig`:阶段三 v2.1 新增,水印阈值/权重面板的设置(可选,缺省走 default)。
///   形状:{ auto_threshold, suspect_threshold, w_repeat, w_non_cjk, w_keyword,
///          repeat_count_min, min_line_chars, enabled };同样 #[serde(default)]。
export const loadAndAnalyze = (
  inputPath,
  encodingOverride,
  cleaningConfig,
  watermarkConfig,
) =>
  invoke("load_and_analyze", {
    inputPath,
    encodingOverride: encodingOverride ?? null,
    cleaningConfig: cleaningConfig ?? null,
    watermarkConfig: watermarkConfig ?? null,
  });

/// 构建 EPUB。
/// `decisions`(v2.2 新增,可选):用户决策列表,形状 `[{ span:{start,end}, scope, verdict }]`。
/// `coverPath`(4.0 新增,可选):封面图片绝对路径。
/// `cssOverride`(4.0 新增,可选):自定义 CSS 字符串,替换内置默认样式。
export const buildEpub = ({ outputPath, title, author, kepubifyPath, decisions, coverPath, cssOverride, embedFonts, fontPath }) =>
  invoke("build_epub", {
    outputPath,
    title,
    author,
    kepubifyPath: kepubifyPath ?? null,
    decisions: decisions ?? null,
    coverPath: coverPath ?? null,
    cssOverride: cssOverride ?? null,
    embedFonts: embedFonts ?? false,
    fontPath: fontPath ?? null,
  });

export const cancelTask = (taskId) => invoke("cancel_task", { taskId });

/// 列出可用主题名称(standard/classic/highcontrast + 用户自定义)。
export const listThemes = () => invoke("list_themes");
/// 读取指定主题 CSS 文本内容。name 不含 .css 扩展名。
export const loadTheme = (name) => invoke("load_theme", { name });

/// 4.3:用字体渲染文字封面。返回 `{ path, dataUrl }`。
/// style: "default" | "gradient"；fontPath 为 null 时用内置霞鹜文楷。
export const generateTextCover = (title, author, style, fontPath) =>
  invoke("generate_text_cover", { title, author, style, fontPath: fontPath ?? null });

/// 4.5:读取 LLM 配置。返回 `{ base_url, model, key_set, key_masked }`。
export const getLlmConfig = () => invoke("get_llm_config");

/// 4.5:保存 LLM 配置并重建客户端。api_key 传空字符串清除 key。
export const setLlmConfig = (baseUrl, model, apiKey) =>
  invoke("set_llm_config", { baseUrl, model, apiKey });

/// 4.6:从缓存 source_text 前约 1 万字推断元数据。
/// 返回 `{ title?, author?, description?, cover_keywords? }` 或 `null`(未配置 LLM / 无法推断)。
export const suggestMetadata = () => invoke("suggest_metadata");

/// 4.7:向 LLM 提交 suspect 候选行,裁定为水印则升级到 auto。
/// spans = [{ start, end }](字节偏移)。
/// 返回 `{ updated_watermarks, new_cleaning }` 或抛错。
export const adjudicateWatermarks = (spans) =>
  invoke("adjudicate_watermarks", { spans });

/// 4.8:向 LLM 提交被拒绝的水印行,归纳一条正则规则。
/// spans = [{ start, end }]。返回 Rule 对象或 null(无法归纳)。
export const induceWatermarkRule = (spans) =>
  invoke("induce_watermark_rule", { spans });

/// 4.8:把规则对象持久化到 rules.json。
export const saveInducedRule = (rule) =>
  invoke("save_induced_rule", { rule });

/// 监听后台进度事件。payload 形状:
///   { task_id, stage, percent, detail }
///   stage ∈ "decoding" | "cleaning" | "chapter" | "watermark" | "epub" | "kepubify"
///   (watermark 为阶段三 3.0 起新增。)
export const onProgress = (handler) =>
  listen("endpoint://progress", (evt) => handler(evt.payload));
