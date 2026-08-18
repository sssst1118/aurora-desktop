// 效果评估脚本:逐状态×逐皮肤截图(CDP Page.captureScreenshot)+ 动效帧率探针
// 截图写入 WSL UNC 路径(绕 Windows 盘加密,供 Claude 直接 view)
// 只读 DOM/注入 dataset.skin,不写用户配置
import fs from "node:fs";

const OUT = "\\\\wsl.localhost\\Ubuntu-20.04\\home\\gzk\\aurora-shots";
const PRE = process.argv[2] || "";
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();

function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let id = 0;
    const pending = new Map();
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) {
        pending.get(msg.id)(msg);
        pending.delete(msg.id);
      }
    };
    ws.onopen = () =>
      resolve({
        ws,
        send: (method, params) =>
          new Promise((res) => {
            const i = ++id;
            pending.set(i, res);
            ws.send(JSON.stringify({ id: i, method, params }));
          }),
      });
    ws.onerror = reject;
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let island, search;
for (const page of targets.filter((t) => t.type === "page")) {
  const cdp = await connect(page.webSocketDebuggerUrl);
  const ev = async (expr) => {
    const r = await cdp.send("Runtime.evaluate", {
      expression: expr,
      returnByValue: true,
      awaitPromise: true,
    });
    if (r.result?.exceptionDetails)
      throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 300));
    return r.result?.result?.value;
  };
  const hasIsland = await ev(`!!document.querySelector(".island")`);
  const hasPanel = await ev(`!!document.querySelector(".main-panel-root")`);
  if (hasIsland && !island) island = { cdp, ev, page };
  if (hasPanel && !search) search = { cdp, ev, page };
}
if (!island || !search) {
  console.log("MISSING_WINDOW island=" + !!island + " search=" + !!search);
  process.exit(1);
}

// 屏幕信息
console.log("SCREEN", await search.ev(`JSON.stringify({dpr: devicePixelRatio, w: screen.width, h: screen.height})`));
console.log("ISLAND_STYLE", await island.ev(`(() => { const el = document.querySelector('.island'); const cs = getComputedStyle(el); return JSON.stringify({ backdrop: cs.backdropFilter || cs.webkitBackdropFilter, bg: cs.backgroundColor, clip: cs.clipPath, w: el.offsetWidth, h: el.offsetHeight }); })()`));
console.log("PANEL_STYLE", await search.ev(`(() => { const el = document.querySelector('.aurora-panel') || document.querySelector('.main-panel-root'); if(!el) return 'NO_PANEL_EL'; const cs = getComputedStyle(el); return JSON.stringify({ backdrop: cs.backdropFilter || cs.webkitBackdropFilter, bg: cs.backgroundColor, radius: cs.borderRadius }); })()`));

let shot = 0;
async function snap(win, name) {
  const r = await win.cdp.send("Page.captureScreenshot", { format: "png" });
  const file = `${OUT}\\${PRE}${String(++shot).padStart(2, "0")}-${name}.png`;
  fs.writeFileSync(file, Buffer.from(r.result.data, "base64"));
  console.log("SNAP", name, r.result.data.length >> 10, "KB");
}

// 帧率探针:注入 rAF 时间戳记录器
async function armFps(win, tag) {
  await win.ev(`window.__fps_${tag} = []; (function loop(t){ if(window.__fpsStop_${tag}) return; window.__fps_${tag}.push(t); requestAnimationFrame(loop); })(performance.now()); 'armed'`);
}
async function readFps(win, tag) {
  const frames = await win.ev(`window.__fpsStop_${tag}=true; JSON.stringify(window.__fps_${tag})`);
  const ts = JSON.parse(frames);
  if (ts.length < 3) return `${tag}: TOO_FEW(${ts.length})`;
  const deltas = [];
  for (let i = 1; i < ts.length; i++) deltas.push(ts[i] - ts[i - 1]);
  const avg = deltas.reduce((a, b) => a + b, 0) / deltas.length;
  const worst = Math.max(...deltas);
  const dropped = deltas.filter((d) => d > 34).length; // >2 帧@60Hz
  const jank = deltas.filter((d) => d > 50).length; // >3 帧
  return `${tag}: n=${ts.length} avg=${avg.toFixed(1)}ms worst=${worst.toFixed(1)}ms dropped(>34ms)=${dropped} jank(>50ms)=${jank}`;
}

// ---- 1. 当前状态:岛收起 + dawn 皮肤 ----
await snap(island, "island-collapsed-dawn");

// ---- 2. 呼出主面板(open_search 真实链路),测打开动效帧率 ----
await armFps(search, "open");
await search.ev(`window.__TAURI_INTERNALS__.invoke("open_search")`);
await sleep(700);
console.log("FPS_OPEN_PANEL", await readFps(search, "open"));
await snap(search, "panel-smalldesktop-dawn");

// ---- 3. 五视图逐一截图 ----
// 视图按钮:header 上 5 个视图按钮,用 aria/文本定位
const VIEWS = [
  ["search", `document.querySelector('button[aria-label*="搜索"], button[title*="搜索"]') || [...document.querySelectorAll('.main-panel-root button')].find(b => b.textContent.includes('搜索'))`],
  ["clipboard", `[...document.querySelectorAll('.main-panel-root button')].find(b => b.textContent.includes('剪贴板'))`],
  ["ai", `[...document.querySelectorAll('.main-panel-root button')].find(b => b.textContent.includes('AI'))`],
  ["settings", `[...document.querySelectorAll('.main-panel-root button')].find(b => b.textContent.includes('设置'))`],
];
for (const [name, sel] of VIEWS) {
  await armFps(search, "sw_" + name);
  const ok = await search.ev(`(() => { const b = ${sel}; if (!b) return 'NO_BTN'; b.click(); return 'clicked'; })()`);
  await sleep(600);
  console.log("VIEW_SWITCH", name, ok, "|", await readFps(search, "sw_" + name));
  await snap(search, "panel-" + name + "-dawn");
}

// 搜索视图:空态引导 + 打字后结果
await search.ev(`(() => { const b = [...document.querySelectorAll('.main-panel-root button')].find(b => b.textContent.includes('搜索')); if (b) b.click(); return 'ok'; })()`);
await sleep(500);
await snap(search, "panel-search-empty-dawn");
await search.ev(`(() => { const inp = document.querySelector('.main-panel-root input[type="text"], .main-panel-root input:not([type])'); if (inp) { inp.focus(); inp.value='we'; inp.dispatchEvent(new Event('input', {bubbles:true})); return 'typed'; } return 'NO_INPUT'; })()`);
await sleep(700);
await snap(search, "panel-search-typed-dawn");

// ---- 4. 岛展开态 ----
await island.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: 189, y: 23, button: "left", clickCount: 1 });
await island.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 189, y: 23, button: "left", clickCount: 1 });
await sleep(500);
console.log("ISLAND_EXPANDED", await island.ev(`document.querySelector('.island')?.offsetWidth`));
await snap(island, "island-expanded-dawn");

// ---- 5. 皮肤遍历(deep/midnight/verdant),面板小桌面视图 ----
await search.ev(`(() => { const b = [...document.querySelectorAll('.main-panel-root button')].find(b => b.textContent.includes('小桌面') || b.getAttribute('aria-label')?.includes('小桌面')); if (b) { b.click(); return 'ok'; } return 'NO_BTN'; })()`);
await sleep(500);
// 清掉搜索输入残留
await search.ev(`(() => { const inp = document.querySelector('.main-panel-root input'); if (inp) { inp.value=''; inp.dispatchEvent(new Event('input', {bubbles:true})); } return 'cleared'; })()`);
for (const skin of ["deep", "midnight", "verdant"]) {
  await search.ev(`document.documentElement.dataset.skin = '${skin}'; 'set'`);
  await island.ev(`document.documentElement.dataset.skin = '${skin}'; 'set'`);
  await sleep(400);
  await snap(search, "panel-smalldesktop-" + skin);
  await snap(island, "island-collapsed-" + skin);
}
// 还原 dawn(仅 DOM 层)
await search.ev(`document.documentElement.dataset.skin = 'dawn'; 'restored'`);
await island.ev(`document.documentElement.dataset.skin = 'dawn'; 'restored'`);

// ---- 6. 岛展开动画帧率(收起→展开)----
await island.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: 189, y: 23, button: "left", clickCount: 1 });
await island.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 189, y: 23, button: "left", clickCount: 1 });
await sleep(400); // 此刻是收起还是展开取决于上一状态,先看宽度
let w = await island.ev(`document.querySelector('.island')?.offsetWidth`);
if (w > 500) {
  // 当前展开,点一下收起,再装探针展开
  await island.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: 189, y: 23, button: "left", clickCount: 1 });
  await island.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 189, y: 23, button: "left", clickCount: 1 });
  await sleep(500);
}
await armFps(island, "expand");
await island.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: 189, y: 23, button: "left", clickCount: 1 });
await island.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 189, y: 23, button: "left", clickCount: 1 });
await sleep(600);
console.log("FPS_ISLAND_EXPAND", await readFps(island, "expand"));

// 收尾:岛收回
w = await island.ev(`document.querySelector('.island')?.offsetWidth`);
if (w > 500) {
  await island.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: 189, y: 23, button: "left", clickCount: 1 });
  await island.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 189, y: 23, button: "left", clickCount: 1 });
  await sleep(500);
}
console.log("DONE");
process.exit(0);
