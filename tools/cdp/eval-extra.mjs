// 补拍:剪贴板/AI 视图(aria-label 定位)+ 面板关闭动效帧率
import fs from "node:fs";
const OUT = "\\wsl.localhost\Ubuntu-20.04\home\gzk\aurora-shots";
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl); let id = 0; const pending = new Map();
    ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
    ws.onopen = () => resolve({ ws, send: (method, params) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); }) });
    ws.onerror = reject;
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let search;
for (const page of targets.filter((t) => t.type === "page")) {
  const cdp = await connect(page.webSocketDebuggerUrl);
  const ev = async (expr) => {
    const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.result?.exceptionDetails) throw new Error("EVAL FAIL");
    return r.result?.result?.value;
  };
  if (await ev(`!!document.querySelector(".main-panel-root")`)) { search = { cdp, ev }; break; }
}
if (!search) { console.log("NO_PANEL"); process.exit(1); }
console.log("VIEW_BTNS", await search.ev(`JSON.stringify([...document.querySelectorAll('.main-panel-root header button, .mp-header button, button[aria-label*="视图"]')].map(b => b.getAttribute('aria-label')))`));
let n = 30;
async function snap(name) {
  const r = await search.cdp.send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(`${OUT}\${n++}-${name}.png`, Buffer.from(r.result.data, "base64"));
  console.log("SNAP", name);
}
for (const kw of ["剪贴板", "AI"]) {
  const ok = await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('${kw}')); if (b) { b.click(); return 'clicked:' + b.getAttribute('aria-label'); } return 'NO_BTN'; })()`);
  console.log("SWITCH", kw, ok);
  await sleep(700);
  await snap("panel-" + (kw === "AI" ? "ai" : "clipboard") + "-dawn");
}
// 面板关闭动效
await search.ev(`window.__fps_close = []; window.__fpsStop_close = false; (function loop(t){ if(window.__fpsStop_close) return; window.__fps_close.push(t); requestAnimationFrame(loop); })(performance.now()); 'armed'`);
await search.ev(`window.__TAURI_INTERNALS__.invoke("hide_search").catch(() => window.__TAURI_INTERNALS__.invoke("close_search")).catch(() => 'no_cmd')`);
await sleep(500);
const frames = await search.ev(`window.__fpsStop_close=true; JSON.stringify(window.__fps_close)`).catch(() => "[]");
const ts = JSON.parse(frames || "[]");
if (ts.length > 2) {
  const d = ts.slice(1).map((t, i) => t - ts[i]);
  console.log("FPS_CLOSE", `n=${ts.length} avg=${(d.reduce((a,b)=>a+b,0)/d.length).toFixed(1)} worst=${Math.max(...d).toFixed(1)} dropped=${d.filter(x=>x>34).length}`);
} else console.log("FPS_CLOSE window_hidden_before_read");
// 重新呼出恢复现场
await search.ev(`window.__TAURI_INTERNALS__.invoke("open_search")`);
await sleep(600);
console.log("DONE");
process.exit(0);
