// 截图功能全链路 CDP 验证(2026-08-18):
// screenshot_begin(创建遮罩窗)→ capture-0 页面拖选 → screenshot_capture(真实截屏)
// → 文件生成 + 剪贴板有图(CF_DIB)+ 岛 hint 事件。
// 前置:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 启动 release exe
// 用法:node cdp-screenshot.mjs
import { execFileSync } from "node:child_process";
import { readdirSync, statSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const targets = await (await fetch("http://127.0.0.1:9222/json")).json();

function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let id = 0;
    const pending = new Map();
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
    };
    ws.onopen = () => resolve({
      ws,
      send: (method, params) => new Promise((res) => {
        const i = ++id; pending.set(i, res);
        ws.send(JSON.stringify({ id: i, method, params }));
      }),
    });
    ws.onerror = reject;
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 找可用窗口:search 优先(面板常开时),island 兜底(启动后常驻;命令全局可 invoke)。
// 页面未就绪(metadata 未注入)的 target 跳过重试。
let search = null;
for (let attempt = 0; attempt < 10 && !search; attempt++) {
  const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
  for (const page of targets.filter((t) => t.type === "page")) {
    const cdp = await connect(page.webSocketDebuggerUrl);
    const ev = async (expr) => {
      const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
      if (r.result?.exceptionDetails) throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 300));
      return r.result?.result?.value;
    };
    try {
      const label = await ev(`window.__TAURI_INTERNALS__.metadata.currentWindow.label`);
      if (label === "search" || label === "island") { search = { cdp, ev, label }; break; }
    } catch { /* 页面未就绪,跳过 */ }
  }
  if (!search) await sleep(600);
}
if (!search) { console.log("NO_WINDOW — 无可用窗口"); process.exit(1); }
console.log("    入口窗口:", search.label);

let pass = 0, fail = 0;
function chk(name, cond, detail = "") {
  if (cond) { pass++; console.log(`PASS ${name}`); }
  else { fail++; console.log(`FAIL ${name} ${detail}`); }
}

// 1. 配置里截图热键字段存在且默认 ctrl+alt+a
const cfg = await search.ev(`window.__TAURI_INTERNALS__.invoke("config_load")`);
chk("S1 screenshot_hotkey 字段=ctrl+alt+a", cfg?.screenshot_hotkey === "ctrl+alt+a", `got=${cfg?.screenshot_hotkey}`);
chk("S2 ai_hotkey 让位=ctrl+alt+e", cfg?.ai_hotkey === "ctrl+alt+e", `got=${cfg?.ai_hotkey}`);

// 2. screenshot_begin → 遮罩窗口创建(多屏时每屏一个,begin 返回屏数)
const begin = await search.ev(`window.__TAURI_INTERNALS__.invoke("screenshot_begin")`);
chk("S3 screenshot_begin 返回屏数(≥1)", begin >= 1, `got=${begin}`);
await sleep(1000);

// capture 窗口 URL 都是 tauri.localhost,按 label 识别(webview label 可从内部 metadata 读)
async function findCapture() {
  const all = await (await fetch("http://127.0.0.1:9222/json")).json();
  for (const page of all.filter((t) => t.type === "page")) {
    try {
      const cdp = await connect(page.webSocketDebuggerUrl);
      const r = await cdp.send("Runtime.evaluate", {
        expression: `window.__TAURI_INTERNALS__.metadata.currentWindow.label`,
        returnByValue: true,
      });
      if (r.result?.result?.value?.startsWith("capture-")) return { cdp, page };
    } catch { /* 连接失败跳过 */ }
  }
  return null;
}
const cap = await findCapture();
chk("S4 capture-0 遮罩窗口已创建", !!cap, "CDP targets 中无 capture 页");
if (!cap) { console.log(`\nSUMMARY ${pass} pass / ${fail} fail`); process.exit(1); }

// 3. capture-0 页面:遮罩元素 + 十字光标
const capCdp = cap.cdp;
const cev = async (expr) => {
  const r = await capCdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.result?.exceptionDetails) throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 300));
  return r.result?.result?.value;
};
const masked = await cev(`!!document.querySelector(".bg-black\\\\/60")`);
chk("S5 ARMED 全屏变暗遮罩渲染", masked);
const cur = await cev(`getComputedStyle(document.querySelector("div[style*='cursor']")).cursor`);
chk("S6 十字光标", cur === "crosshair", `cursor=${cur}`);
// 诊断钩子:记录实际收到的 pointer 事件与窗口/缩放信息
await cev(`window.__diag = [];
window.addEventListener('pointerdown', e => __diag.push(['down', Math.round(e.clientX), Math.round(e.clientY)]));
window.addEventListener('pointermove', e => __diag.push(['move', Math.round(e.clientX), Math.round(e.clientY)]));
window.addEventListener('pointerup', e => __diag.push(['up', Math.round(e.clientX), Math.round(e.clientY)]));
__diag`);

// 4. 拖选 300×200(逻辑像素):JS 合成 PointerEvent 派发到 window
// (CDP Input.dispatchMouseEvent 的 mousePressed 在 WebView2 不派发 pointerdown——
// 已实测两次;真实鼠标留真机验收,此处验证前端→后端全链路逻辑)
const imgDir = path.join(os.homedir(), "Pictures", "Aurora 截图");
const before = readdirSync(imgDir).filter((f) => f.endsWith(".png")).length;
const dragExpr = `(async () => {
  const fire = (type, x, y) => window.dispatchEvent(new PointerEvent(type, {
    clientX: x, clientY: y, bubbles: true, cancelable: true,
    button: 0, buttons: type === 'pointerup' ? 0 : 1, pointerId: 1, isPrimary: true,
  }));
  fire('pointerdown', 120, 120);
  for (let i = 1; i <= 6; i++) { fire('pointermove', 120 + i * 50, 120 + i * 33); await new Promise(r => setTimeout(r, 30)); }
  fire('pointerup', 420, 320);
  return true;
})()`;
await cev(dragExpr);
await sleep(800); // 等 hide→100ms→截屏→保存→事件

// 5b. 岛窗口提示(screenshot-done 事件的最终落点:hint 渲染在药丸内 .island-hint,
// innerText 靠后;hint 3 秒消失——必须最先查,PS 冷启动检查会拖过窗口)
let islandText = "NO_ISLAND";
for (const page of (await (await fetch("http://127.0.0.1:9222/json")).json()).filter((t) => t.type === "page")) {
  try {
    const cdp = await connect(page.webSocketDebuggerUrl);
    const r = await cdp.send("Runtime.evaluate", {
      expression: `window.__TAURI_INTERNALS__.metadata.currentWindow.label`,
      returnByValue: true,
    });
    if (r.result?.result?.value !== "island") continue;
    const t = await cdp.send("Runtime.evaluate", { expression: `document.body.innerText`, returnByValue: true });
    islandText = t.result?.result?.value || "NO_TEXT";
    break;
  } catch { /* 跳过 */ }
}
const islandFull = islandText.replace(/\n/g, " | ");
console.log("    岛提示文本:", islandFull.slice(0, 200));
chk("S10b 岛收到截图提示(含复制状态)", islandText.includes("已截图") && (islandText.includes("已复制") || islandText.includes("复制失败")), `text=${islandText.slice(0, 80)}`);

// 5. 剪贴板有图片:CF_DIB 写入后系统自动提供 Bitmap 格式,用 .NET GetImage 读取
// (PS 5.1 的 Get-Clipboard -Format Image 读 CF_DIB 有已知坑返回空,勿用)
const clip = (execFileSync("powershell", ["-NoProfile", "-Command",
  `Add-Type -AssemblyName System.Windows.Forms; $img = [System.Windows.Forms.Clipboard]::GetDataObject().GetImage(); if ($img) { Write-Output ($img.Width.ToString() + 'x' + $img.Height.ToString()) } else { Write-Output 'EMPTY' }`]).toString().trim() || "EMPTY");
chk("S10 剪贴板含位图(可粘贴)", /^\d+x\d+$/.test(clip), `clip=${clip}`);

// 5c. 新文件生成 + 尺寸≈300×200(物理 1:1 缩放下)
const after = readdirSync(imgDir).filter((f) => f.endsWith(".png")).sort();
chk("S7 截图文件已生成", after.length > before, `before=${before} after=${after.length}`);
if (after.length > before) {
  const latest = path.join(imgDir, after[after.length - 1]);
  console.log("    文件:", latest);
  const size = statSync(latest).size;
  chk("S8 文件非空", size > 1000, `size=${size}`);
  // PNG IHDR 尺寸(字节 16-23,大端)
  const buf = execFileSync("powershell", ["-NoProfile", "-Command",
    `$b=[IO.File]::ReadAllBytes('${latest}'); Write-Output ($b[16]*16777216+$b[17]*65536+$b[18]*256+$b[19]); Write-Output ($b[20]*16777216+$b[21]*65536+$b[22]*256+$b[23])`]).toString().trim().split(/\r?\n/);
  const pw = +buf[0], ph = +buf[1];
  chk("S9 尺寸≈300×200", Math.abs(pw - 300) <= 8 && Math.abs(ph - 200) <= 8, `got=${pw}x${ph}`);
}

// 7. 遮罩窗已隐藏(截完复用不销毁;窗口存在但不可见)
// 探针用 is_visible(tauri 窗口 API):document.visibilityState 在 WebView2 隐藏窗口时
// 不更新(实测保持 visible),不可靠
await sleep(300);
const v1 = await cev(`window.__TAURI_INTERNALS__.invoke('plugin:window|is_visible', {})`);
await sleep(500);
const v2 = await cev(`window.__TAURI_INTERNALS__.invoke('plugin:window|is_visible', {})`);
chk("S11 遮罩窗已隐藏", v1 === false && v2 === false, `is_visible=${v1},${v2}`);
// 诊断输出:pointer 事件序列 + 缩放
const diag = await cev(`({ ev: __diag, dpr: window.devicePixelRatio, iw: window.innerWidth, ih: window.innerHeight })`);
console.log("    diag:", JSON.stringify(diag));

// 8. 设置页有截图热键录制项(前端配置渲染)
await search.ev(`document.querySelector("button[aria-label='呼出主面板']") || (window.__TAURI_INTERNALS__.invoke("open_search"), true)`);
await sleep(600);
await search.ev(`[...document.querySelectorAll(".view-switch .view-btn")].find((b) => b.getAttribute("aria-label")?.includes("设置"))?.click()`);
await sleep(600);
const hotkeyInput = await search.ev(`[...document.querySelectorAll("input")].some((i) => i.getAttribute("aria-label") === "截图热键" && i.value === "ctrl+alt+a")`);
chk("S12 设置页截图热键项渲染", hotkeyInput);

// 收尾:Esc 关面板
await search.cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27 });
await search.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27 });
await sleep(300);

console.log(`\nSUMMARY ${pass} pass / ${fail} fail`);
process.exit(fail ? 1 : 0);
