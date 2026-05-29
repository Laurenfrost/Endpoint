import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// Tauri 会通过 beforeDevCommand 启动这里的 dev server,通过 frontendDist 取
// build 产物。详见 `docs/stage2-design.md` 第一节与 `tauri.conf.json`。
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  resolve: {
    alias: {
      $lib: path.resolve(__dirname, "src/lib"),
    },
  },
  // 在 Tauri 中通过 file:// 加载产物,base 必须是相对路径。
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Tauri 桌面里没有 CDN 兜底,sourcemap 留着方便排查。
    sourcemap: true,
    target: "es2022",
  },
  server: {
    port: 5173,
    strictPort: true,
    // 阻止 Vite 把请求自动转发到代理,Tauri 内嵌窗口直接访问 localhost:5173。
    host: "127.0.0.1",
  },
  clearScreen: false,
});
