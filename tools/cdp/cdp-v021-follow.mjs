// 复测 F1「面板实时跟随岛」:注入 island-geometry 计数器 + 观察 search 窗口位置
const targets = (await (await fetch("http://127.0.0.1:9222/json")).json()).filter((t) => t.type === "page");
function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let id = 0; const pending = new Map();
    ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
    ws.onopen = () => resolve({ send: (method, params) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); }), close: () => ws.close() });
    ws.onerror = reject;
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let island, search;
for (const page of targets) {
  const cdp = await connect(page.webSocketDebuggerUrl);
  const ev = async (expr) => (await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;
  const hasIsland = await ev(`!!document.querySelector(".island")`);
  const hasPanel = await ev(`!!document.querySelector(".main-panel-root")`);
  if (hasIsland && !island) island = { cdp, ev, page };
  if (hasPanel && !search) search = { cdp, ev, page };
  if (!hasIsland && !hasPanel) cdp.close();
}
async function bounds(cdp, targetId) {
  const r = await cdp.send("Browser.getWindowForTarget", { targetId });
  const b = await cdp.send("Browser.getWindowBounds", { windowId: r.result.windowId });
  return b.result.bounds;
}
const dbl = () => island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
// 1. 确保面板 show:双击呼出
await dbl(); await sleep(150); await dbl(); await sleep(1000);
// 2. 注入事件计数器(search 窗口)
const injected = await search.ev(`(() => { window.__geoCount = 0; window.__geoLast = null;
  window.__TAURI_INTERNALS__.invoke('plugin:event|listen', { event: 'island-geometry', handler: (ev) => { window.__geoCount++; window.__geoLast = JSON.stringify(ev.payload || {}); } }).catch(e => { window.__geoErr = String(e); });
  return "injected"; })()`);
console.log("INJECT:", injected);
await sleep(300);
console.log("GEO_AFTER_INJECT(expect 0):", await search.ev(`window.__geoCount`));
// 3. 移动 island
const i0 = await bounds(island.cdp, island.page.id);
const s0 = await bounds(search.cdp, search.page.id);
console.log(`BEFORE: island@(${i0.left},${i0.top}) search@(${s0.left},${s0.top})`);
const TAU = `window.__TAURI_INTERNALS__`;
for (let k = 1; k <= 3; k++) {
  await island.ev(`${TAU}.invoke("plugin:window|set_position", { label: "island", value: { Physical: { x: ${i0.left + k * 40}, y: ${i0.top + k * 30} } } })`);
  await sleep(100);
}
await sleep(400);
const geo = await search.ev(`window.__geoCount`);
const geoLast = await search.ev(`window.__geoLast`);
const i1 = await bounds(island.cdp, island.page.id);
const s1 = await bounds(search.cdp, search.page.id);
console.log(`GEO_COUNT(expect >=1): ${geo} last=${geoLast}`);
console.log(`AFTER: island@(${i1.left},${i1.top}) search@(${s1.left},${s1.top})`);
console.log(`DELTA: island(${i1.left - i0.left},${i1.top - i0.top}) search(${s1.left - s0.left},${s1.top - s0.top})`);
// 4. 复位
await island.ev(`${TAU}.invoke("plugin:window|set_position", { label: "island", value: { Physical: { x: ${i0.left}, y: ${i0.top} } } })`);
await sleep(200);
console.log("FOLLOW-RETEST DONE");
process.exit(0);
