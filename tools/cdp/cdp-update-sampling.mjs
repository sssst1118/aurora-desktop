// C1 精测:点"检查更新"按钮,逐秒采样 UI 状态直到变化(上限 45s)
// 用法:node cdp-update-sampling.mjs
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
let found = false;
for (const page of targets.filter((t) => t.type === "page")) {
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 0; const pending = new Map();
  ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
  const send = (method, params) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
  const ev = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;
  if (!(await ev(`!!document.querySelector(".main-panel-root")`))) continue;
  found = true;
  await ev(`window.__TAURI_INTERNALS__.invoke("open_search")`);
  await new Promise((r) => setTimeout(r, 800));
  await ev(`[...document.querySelectorAll(".view-switch .view-btn")].find((b) => b.getAttribute("aria-label")?.includes("设置"))?.click()`);
  await new Promise((r) => setTimeout(r, 800));
  const hasBtn = await ev(`!!document.querySelector("button[aria-label='检查更新']")`);
  console.log("设置视图:", hasBtn ? "已就绪" : "未找到按钮");
  if (!hasBtn) process.exit(1);
  await ev(`document.querySelector("button[aria-label='检查更新']")?.click()`);
  for (let i = 0; i < 45; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    const o = JSON.parse(await ev(`(() => {
      const t = document.body.innerText;
      return JSON.stringify({
        btn: document.querySelector("button[aria-label='检查更新']")?.textContent?.trim(),
        err: (t.match(/检查失败[^\\n]*/) ?? t.match(/更新失败[^\\n]*/) ?? t.match(/检查更新失败[^\\n]*/) ?? [null])[0],
        latest: t.includes("已是最新版本") ? t.match(/已是最新版本[^\\n]*/)[0] : null,
      });
    })()`));
    console.log(`t=${i}s btn=${o.btn} err=${o.err ? o.err.slice(0, 50) : "null"} latest=${o.latest ? "yes" : "null"}`);
    if (o.btn !== "检查中…" || o.err || o.latest) break;
  }
  ws.close();
  break;
}
if (!found) { console.log("NO_PANEL"); process.exit(1); }
process.exit(0);
