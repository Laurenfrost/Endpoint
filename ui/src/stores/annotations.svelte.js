// 当前激活的高亮层 + 跳转信号。
// 每个 stage 组件在 mount 时调 `setLayers(...)`,TextView / OverviewRuler 订阅。
//
// layer 形状:
//   { id, color, className, items: [{ span: { start, end (bytes) }, data?, label? }] }
// items 必须按 span.start 升序(消费方依赖此性质做二分)。
//
// 跳转信号:`jumpTo` 是 { offset, version }。version 单调递增,即便同 offset 也能触发。

export const annotations = $state({
  layers: [],
  jumpTo: { offset: 0, version: 0 },
});

export function setLayers(layers) {
  annotations.layers = layers;
}

export function clearLayers() {
  annotations.layers = [];
}

export function jumpToByteOffset(offset) {
  annotations.jumpTo = {
    offset,
    version: annotations.jumpTo.version + 1,
  };
}
