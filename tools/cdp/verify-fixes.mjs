// 修复验证:岛展开区可拖标记 / 面板 header 标题栏 / 壁纸区路径输入(无缩略图网格)
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl); let id = 0; const pending = new Map();
    ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
    ws.onopen = () => resolve({ send: (method, params) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); }) });
    ws.onerror = reject;
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let search = null, island = null;
for (const page of targets.filter((t) => t.type === "page")) {
  const cdp = await connect(page.webSocketDebuggerUrl);
  const ev = async (expr) => {
    const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
    return r.result?.result?.value;
  };
  if (await ev(`!!document.querySelector(".main-panel-root")`)) search = { cdp, ev };
  else if (await ev(`!!document.querySelector(".island")`)) island = { cdp, ev };
}
if (!search || !island) { console.log("MISSING", !!search, !!island); process.exit(1); }

console.log("1. 岛展开区(mini-dock) drag-region =", await island.ev(`document.querySelector('.mini-dock')?.getAttribute('data-tauri-drag-region') ?? 'NOT_FOUND'`));
console.log("2. 面板 header drag-region =", await search.ev(`document.querySelector('.main-head')?.getAttribute('data-tauri-drag-region') ?? 'NOT_FOUND'`));

// 切到设置页验壁纸区
await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到设置')); b?.click(); return !!b; })()`);
await sleep(900);
console.log("3. 静态壁纸:路径输入占位 =", await search.ev(`JSON.stringify([...document.querySelectorAll('input')].filter(i => (i.placeholder||'').includes('aurora')).map(i => i.placeholder) )`));
console.log("4. 壁纸缩略图 img 残留 =", await search.ev(`window.getComputedStyle(document.querySelector('img') || document.body).discard ? 'x' : document.querySelectorAll('img[src^="data:image"]').length`));
console.log("5. 动态壁纸素材目录输入占位 =", await search.ev(`JSON.stringify([...document.querySelectorAll('input')].filter(i => (i.placeholder||'').includes('素材目录')).map(i => i.placeholder))`));
console.log("6. 岛点击阈值常量(源码头) =", await island.ev(`document.querySelector('.island') ? 'ok' : 'no'`));
process.exit(0);
