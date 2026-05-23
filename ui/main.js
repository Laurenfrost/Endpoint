// Tauri 2 全局桥接(withGlobalTauri: true 已在 tauri.conf.json 启用)。
// 文件选择走 Rust 端的命令,避免依赖 plugin 的 JS bundle。
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const statusEl = $("status");

function setStatus(text, kind = "") {
  statusEl.textContent = text;
  statusEl.className = "status" + (kind ? " " + kind : "");
}

$("pickInput").addEventListener("click", async () => {
  try {
    const path = await invoke("pick_input_file");
    if (typeof path === "string") {
      $("input").value = path;
      if (!$("output").value) {
        $("output").value = path.replace(/\.txt$/i, "") + ".epub";
      }
    }
  } catch (e) {
    setStatus("选择文件失败:" + e, "err");
  }
});

$("pickOutput").addEventListener("click", async () => {
  try {
    const path = await invoke("pick_output_file", {
      defaultPath: $("output").value || null,
    });
    if (typeof path === "string") {
      $("output").value = path;
    }
  } catch (e) {
    setStatus("选择保存位置失败:" + e, "err");
  }
});

$("pickKepubify").addEventListener("click", async () => {
  try {
    const path = await invoke("pick_executable_file");
    if (typeof path === "string") {
      $("kepubify").value = path;
    }
  } catch (e) {
    setStatus("选择 kepubify 失败:" + e, "err");
  }
});

$("generate").addEventListener("click", async () => {
  const input = $("input").value.trim();
  const output = $("output").value.trim();
  const title = $("title").value.trim();
  const author = $("author").value.trim();
  const kepubify = $("kepubify").value.trim();

  if (!input) return setStatus("请先选择输入 txt 文件。", "err");
  if (!output) return setStatus("请先选择输出 epub 位置。", "err");
  if (!title) return setStatus("请填写书名。", "err");
  if (!author) return setStatus("请填写作者。", "err");

  setStatus("正在生成...(大文件请耐心等待,阶段零无进度条)", "");
  $("generate").disabled = true;

  try {
    const finalPath = await invoke("convert", {
      input,
      output,
      title,
      author,
      kepubifyPath: kepubify || null,
    });
    setStatus("生成成功:\n" + finalPath, "ok");
  } catch (e) {
    setStatus("生成失败:\n" + e, "err");
  } finally {
    $("generate").disabled = false;
  }
});
