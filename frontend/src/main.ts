import "./lib/styles/global.css";
import "./lib/styles/glass.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

// WebView 挂死看门狗心跳（配合 Rust 侧 window_cmd::watch_webview_hangs）：
// 引擎重建等场景会让 WebView2 浏览器/渲染侧整体挂死——JS 定时器、输入、
// 呈现一起停摆，页面内任何自愈都跑不起来。前端唯一能做的就是把「我还活着」
// 持续送到原生侧；断流 >8s 且窗口可见未被遮挡时，由原生看门狗重建窗口。
const heartbeatLabel = getCurrentWindow().label;
// rAF 帧计数：合成器一旦停止出帧（呈现停滞/浏览器侧挂死），计数冻结——
// 心跳即使仍在发（setInterval 不受合成影响），载荷不再变化也会被原生看门狗识破。
let hbFrames = 0;
const hbCount = () => {
  hbFrames += 1;
  requestAnimationFrame(hbCount);
};
requestAnimationFrame(hbCount);
window.setInterval(() => {
  emit("panel-heartbeat", { label: heartbeatLabel, frames: hbFrames }).catch(() => undefined);
}, 500);

// WebView2 合成器停滞自愈（bug「控制面板底部大量空白/滚动条失灵」2026-11-02）。
//
// 实测：玻璃引擎重建或高 GPU 负载时滚动长页面，WebView2 合成器会停止提交
// 新帧——画面冻结、输入被吞，但 JS 定时器与导航仍然活着（Ctrl+R 立刻恢复）。
// rAF 回调依赖合成帧：可见状态下 rAF 心跳停滞超过 2.5s 即判定画面已死，
// 主动 reload 自愈；60s 内最多自愈一次，避免坏状态下反复重载。
let lastFrame = performance.now();
const heartbeat = () => {
  lastFrame = performance.now();
  requestAnimationFrame(heartbeat);
};
requestAnimationFrame(heartbeat);
window.setInterval(() => {
  if (document.visibilityState !== "visible") return;
  if (performance.now() - lastFrame <= 2500) return;
  const key = "ikun-compositor-heal";
  const last = Number(sessionStorage.getItem(key) ?? "0");
  if (performance.now() - last < 60_000) return;
  sessionStorage.setItem(key, String(performance.now()));
  location.reload();
}, 1000);

const target = document.getElementById("app");
if (!target) {
  throw new Error("缺少应用挂载节点 #app");
}

const app = mount(App, {
  target,
});

export default app;
