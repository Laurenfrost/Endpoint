// 主管线状态。`load_and_analyze` 返回的 DTO 在这里挂着,Stage 1-4 共用。
//
// `byteIndex` 在 setPipeline 时一次性构建,后续高亮换算都查它(O(log n))。
import { ByteIndex } from "../text/ByteIndex.js";

/** @type {{ dto: any|null, byteIndex: ByteIndex|null, sourcePath: string }} */
export const pipeline = $state({
  dto: null,
  byteIndex: null,
  sourcePath: "",
});

export function setPipeline(dto, sourcePath = "") {
  pipeline.dto = dto;
  pipeline.byteIndex = new ByteIndex(dto.source_text);
  pipeline.sourcePath = sourcePath;
}

export function clearPipeline() {
  pipeline.dto = null;
  pipeline.byteIndex = null;
  pipeline.sourcePath = "";
}
