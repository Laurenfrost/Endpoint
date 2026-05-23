// 当前所在阶段。
// 1 = 文本选择 / 2 = 文本处理 / 3 = 章节分析 / 4 = 样式预览与导出。

export const stage = $state({ id: 1 });

export function setStage(n) {
  if (n >= 1 && n <= 4) stage.id = n;
}

export const STAGE_DEFS = [
  { id: 1, label: "文本选择", icon: "📁" },
  { id: 2, label: "文本处理", icon: "🧹" },
  { id: 3, label: "章节分析", icon: "📑" },
  { id: 4, label: "样式预览与导出", icon: "📤" },
];
