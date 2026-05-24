// 阶段三 v2.2:用户对自动检测结果的覆盖决策。
//
// 三态:
//   - pending(map 中无键)= 跟默认行为走
//   - "approved"           = 显式接受(确认要删)
//   - "rejected"           = 显式拒绝(确认要留)
//
// 决策**仅本次转换会话**有效——reload(load_and_analyze 重跑)即清空。
// 详见 `docs/stage3-v2-design.md` 第三节 3.3 决策语义。
//
// scope 区分 cleaning vs watermark:
//   - cleaning:来自 pipeline.cleaning 列表(kind 非 watermark_*)
//   - watermark:来自 pipeline.watermark 列表(auto + suspect)
//                     auto 也在 cleaning 中以 watermark_* kind 形式存在,
//                     但 spanKey 仍以 "watermark" 为 scope 索引——前端把"水印类"
//                     都归到水印决策,与后端 `DecisionScope::Watermark` 对齐。
//
// spanKey 编码:`${scope}:${start}-${end}`,保证 cleaning [0,3) 与 watermark [0,3)
// 互不干扰。

const KEY = (scope, span) => `${scope}:${span.start}-${span.end}`;

export const decisions = $state({
  // Map 不是 reactive — 用 plain object 模拟,key 用上面 KEY() 编码
  map: {},
});

/// 拿某 span 的当前 verdict(返回 "approved" / "rejected" / undefined)。
export function getDecision(scope, span) {
  return decisions.map[KEY(scope, span)];
}

/// 切换决策:若已是 want,则取消(回 pending);否则改为 want。
/// 这是"3 态 + 2 按钮"UX 的核心——同按钮第二次点击 = 取消选择。
export function toggleDecision(scope, span, want) {
  const k = KEY(scope, span);
  if (decisions.map[k] === want) {
    delete decisions.map[k];
    decisions.map = { ...decisions.map }; // 触发 reactive 更新
  } else {
    decisions.map = { ...decisions.map, [k]: want };
  }
}

/// 批量设置某 scope 全部条目为 verdict。spans 是当前可见列表(批量栏当前 tab)。
export function bulkSet(scope, spans, verdict) {
  const next = { ...decisions.map };
  for (const span of spans) {
    next[KEY(scope, span)] = verdict;
  }
  decisions.map = next;
}

/// 批量清除某 scope 全部决策(批量栏"重置")。spans 是当前可见列表。
export function bulkClear(scope, spans) {
  const next = { ...decisions.map };
  for (const span of spans) {
    delete next[KEY(scope, span)];
  }
  decisions.map = next;
}

/// 整体清空(reload 时调用)。返回清空前的决策条数,供 UI 提示用。
export function clearAllDecisions() {
  const n = Object.keys(decisions.map).length;
  decisions.map = {};
  return n;
}

/// 序列化为后端期望的形状 —— Vec<UserDecision>(snake_case)。
/// 用于 build_epub 命令传决策列表。
export function serializeForIpc() {
  const out = [];
  for (const [k, verdict] of Object.entries(decisions.map)) {
    const [scope, rest] = k.split(":");
    const [start, end] = rest.split("-").map(Number);
    out.push({
      span: { start, end },
      scope, // "cleaning" | "watermark",已是 snake_case
      verdict, // "approved" | "rejected",已是 snake_case
    });
  }
  return out;
}

/// 决策总数(用于 reload 提示与状态栏显示)。
export function decisionCount() {
  return Object.keys(decisions.map).length;
}
