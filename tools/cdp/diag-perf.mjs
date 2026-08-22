// 性能诊断:包装 invoke 记录每条命令耗时 + longtask 观察,复现 设置→小桌面 序列
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
    if (r.result?.exceptionDetails) throw new Error("EVAL FAIL " + JSON.stringify(r.result.exceptionDetails).slice(0, 200));
    return r.result?.result?.value;
  };
  if (await ev(`!!document.querySelector(".main-panel-root")`)) { search = { cdp, ev }; break; }
}
if (!search) { console.log("NO_PANEL"); process.exit(1); }

// 注入:invoke 包装 + longtask + rAF 主线程心跳(心跳间隔>200ms=主线程卡)
await search.ev(`(() => {
  if (window.__perfArmed) return 'already';
  window.__perfArmed = true;
  window.__t0 = performance.now();
  window.__invokes = [];
  const orig = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  window.__TAURI_INTERNALS__.invoke = (cmd, args) => {
    const st = performance.now();
    return orig(cmd, args).then((v) => {
      const d = performance.now() - st;
      if (d > 80) window.__invokes.push({ cmd, d: Math.round(d), at: Math.round(st - window.__t0) });
      return v;
    }, (e) => { window.__invokes.push({ cmd, err: String(e).slice(0, 60), d: Math.round(performance.now() - st), at: Math.round(st - window.__t0) }); throw e; });
  };
  window.__longtasks = [];
  new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__longtasks.push({ d: Math.round(e.duration), at: Math.round(e.startTime - window.__t0) }); }).observe({ entryTypes: ['longtask'] });
  window.__gaps = [];
  let last = performance.now();
  (function beat(t) { const g = t - last; if (g > 200) window.__gaps.push({ gap: Math.round(g), at: Math.round(t - window.__t0) }); last = t; requestAnimationFrame(beat); })(last);
  return 'armed';
})()`);

await search.ev(`window.__TAURI_INTERNALS__.invoke("open_search")`);
await sleep(800);
console.log("== 打开面板,切到设置 ==");
await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到设置')); b.click(); return 'ok'; })()`);
await sleep(4000);
console.log("== 设置停留 4s 完,切到小桌面 ==");
await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到小桌面')); b.click(); return 'ok'; })()`);
await sleep(3000);
console.log("== 再切回设置,再切搜索(对照) ==");
await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到设置')); b.click(); return 'ok'; })()`);
await sleep(3000);
await search.ev(`(() => { const b = [...document.querySelectorAll('button')].find(b => (b.getAttribute('aria-label')||'').includes('切换到搜索')); b.click(); return 'ok'; })()`);
await sleep(2000);

console.log("SLOW_INVOKES", await search.ev(`JSON.stringify(window.__invokes)`));
console.log("LONGTASKS", await search.ev(`JSON.stringify(window.__longtasks.slice(0, 30))`));
console.log("MAIN_GAPS", await search.ev(`JSON.stringify(window.__gaps.slice(0, 30))`));
process.exit(0);
