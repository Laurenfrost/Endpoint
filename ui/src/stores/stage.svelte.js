// 当前所在视图。
// view = "stage":侧边栏显示阶段面板,stage.id ∈ {1,2,3,4}。
//   1 = 文本选择 / 2 = 文本处理 / 3 = 章节分析 / 4 = 样式预览与导出
// view = "settings":侧边栏显示设置面板(LLM / 搜索 / kepubify 等)。
//   stage.id 保持不变,从设置返回时高亮恢复。

export const stage = $state({ id: 1, view: "stage" });

export function setStage(n) {
  if (n >= 1 && n <= 4) {
    stage.id = n;
    stage.view = "stage";
  }
}

export function openSettings() {
  stage.view = "settings";
}

export function closeSettings() {
  stage.view = "stage";
}

export function toggleSettings() {
  stage.view = stage.view === "settings" ? "stage" : "settings";
}

export const STAGE_DEFS = [
  { id: 1, label: "文本选择", icon: "📁" },
  { id: 2, label: "文本处理", icon: "🧹" },
  { id: 3, label: "章节分析", icon: "📑" },
  { id: 4, label: "样式预览与导出", icon: "📤" },
];
