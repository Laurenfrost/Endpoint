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

export const loadAndAnalyze = (inputPath, encodingOverride) =>
  invoke("load_and_analyze", {
    inputPath,
    encodingOverride: encodingOverride ?? null,
  });

export const buildEpub = ({ outputPath, title, author, kepubifyPath }) =>
  invoke("build_epub", {
    outputPath,
    title,
    author,
    kepubifyPath: kepubifyPath ?? null,
  });

export const cancelTask = (taskId) => invoke("cancel_task", { taskId });

/// 监听后台进度事件。payload 形状:
///   { task_id, stage, percent, detail }
///   stage ∈ "decoding" | "cleaning" | "chapter" | "epub" | "kepubify"
export const onProgress = (handler) =>
  listen("endpoint://progress", (evt) => handler(evt.payload));
