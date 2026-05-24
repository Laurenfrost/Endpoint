// 后台任务进度。`endpoint://progress` 事件 → 这里。
// 顶栏/底栏的进度条订阅本 store。

export const progress = $state({
  taskId: "",
  stage: "", // decoding / cleaning / chapter / watermark / epub / kepubify
  percent: 0,
  detail: null,
  busy: false,
});

const STAGE_LABELS = {
  decoding: "解码",
  cleaning: "清洗",
  chapter: "章节",
  watermark: "水印", // 阶段三 3.4 新增
  epub: "EPUB",
  kepubify: "kepubify",
};

export function setBusy(b) {
  progress.busy = b;
  if (!b) {
    progress.percent = 0;
    progress.stage = "";
    progress.detail = null;
  }
}

export function applyProgressEvent(payload) {
  progress.taskId = payload.task_id;
  progress.stage = payload.stage;
  progress.percent = payload.percent;
  progress.detail = payload.detail ?? null;
}

export function stageLabel(s) {
  return STAGE_LABELS[s] ?? s;
}
