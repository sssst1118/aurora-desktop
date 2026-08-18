// 补拍:剪贴板/AI/设置 三视图(dawn)+ midnight 设置页
import fs from "node:fs";
const OUT = "\\\\wsl.localhost\\Ubuntu-20.04\\home\\gzk\\aurora-shots";
const PRE = process.argv[2] || "";
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
let n = 40;
async function snap(name) {
  const r = await search.cdp.send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(OUT + "\\" + PRE + (n++) + "-" + name + ".png", Buffer.from(r.result.data, "base64"));
  console.log("SNAP", name);
}
for (const [kw, file] of [["剪贴板", "panel-clipboard-dawn"], ["AI", "panel-ai-dawn"], ["设置", "panel-settings-dawn"]]) {
  const ok = await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到${kw}')); if (b) { b.click(); return 'ok'; } return 'NO_BTN'; })()`);
  console.log("SWITCH", kw, ok);
  await sleep(700);
  await snap(file);
}
await search.ev(`document.documentElement.dataset.skin = 'midnight'; 'set'`);
await sleep(400);
await snap("panel-settings-midnight");
await search.ev(`document.documentElement.dataset.skin = 'dawn'; 'restored'`);
console.log("DONE");
process.exit(0);
