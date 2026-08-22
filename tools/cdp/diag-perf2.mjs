// 性能诊断 2:启动即武装,复现用户时间线(启动→尽快切设置→停留→切走)
// 记录全量 invoke 时间线(at/cmd/d/返回体积KB)+ longtask + 主线程心跳
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
for (let tries = 0; tries < 20 && !search; tries++) {
  const ts = await (await fetch("http://127.0.0.1:9222/json")).json().catch(() => []);
  for (const page of ts.filter((t) => t.type === "page")) {
    const cdp = await connect(page.webSocketDebuggerUrl).catch(() => null);
    if (!cdp) continue;
    const ev = async (expr) => {
      const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
      return r.result?.result?.value;
    };
    if (await ev(`!!document.querySelector(".main-panel-root")`).catch(() => false)) { search = { cdp, ev }; break; }
  }
  if (!search) await sleep(500);
}
if (!search) { console.log("NO_PANEL"); process.exit(1); }
console.log("ARMED_AT", Date.now() % 100000);

await search.ev(`(() => {
  if (window.__perfArmed) return 'already';
  window.__perfArmed = true;
  window.__t0 = performance.now();
  window.__invokes = [];
  const orig = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  window.__TAURI_INTERNALS__.invoke = (cmd, args) => {
    const st = performance.now();
    return orig(cmd, args).then((v) => {
      window.__invokes.push({ cmd, d: Math.round(performance.now() - st), at: Math.round(st - window.__t0), kb: Math.round(JSON.stringify(v ?? 0).length / 1024) });
      return v;
    }, (e) => { window.__invokes.push({ cmd, err: String(e).slice(0, 50), d: Math.round(performance.now() - st), at: Math.round(st - window.__t0) }); throw e; });
  };
  window.__longtasks = [];
  new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__longtasks.push({ d: Math.round(e.duration), at: Math.round(e.startTime - window.__t0) }); }).observe({ entryTypes: ['longtask'] });
  window.__gaps = [];
  let last = performance.now();
  (function beat(t) { const g = t - last; if (g > 150) window.__gaps.push({ gap: Math.round(g), at: Math.round(t - window.__t0) }); last = t; requestAnimationFrame(beat); })(last);
  return 'armed';
})()`);

await search.ev(`window.__TAURI_INTERNALS__.invoke("open_search")`);
await sleep(400);
console.log("== 切设置 ==");
await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到设置')); b.click(); return 'ok'; })()`);
await sleep(5000);
console.log("== 切小桌面 ==");
await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到小桌面')); b.click(); return 'ok'; })()`);
await sleep(3000);

console.log("INVOKES", await search.ev(`JSON.stringify(window.__invokes)`));
console.log("LONGTASKS", await search.ev(`JSON.stringify(window.__longtasks)`));
console.log("GAPS", await search.ev(`JSON.stringify(window.__gaps)`));
process.exit(0);
