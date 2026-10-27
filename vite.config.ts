import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// 开发端口固定 1420，与 tauri.conf.json 的 devUrl 对齐；端口被占用时直接失败，
// 避免 Tauri 连到一个"看起来正常但其实是别的服务"的端口。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    target: "chrome105",
    emptyOutDir: true,
    // 图片一律输出成真实文件（不内联成 data URL）：
    // 顶部导航的彩色图标要能通过 /assets/icons/nav/*.png 断言与缓存，体积只有几 KB。
    assetsInlineLimit: 0,
  },
});
