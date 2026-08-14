// CDP 验证 v0.2.1 波次 1 修复:
// 1. 岛回归:clip-path / 双击展开→450ms 收回 / dragging class(回归 25 条)
// 2. 面板实时跟随岛拖动(F1 高优):island set_position 连续移动 → search 窗口位置跟随变化
// 3. 最近访问引导空态(F3 高优,用户真机反馈):清 recents → 搜索视图显示「输入以搜索」→ 恢复
// 4. enable_dock 门控(F1):config enable_dock=false → 展开态无 .mini-dock → 恢复
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
const pages = targets.filter((t) => t.type === "page");

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
        send: (method, params) =>
          new Promise((res) => {
            const i = ++id;
            pending.set(i, res);
            ws.send(JSON.stringify({ id: i, method, params }));
          }),
        close: () => ws.close(),
      });
    ws.onerror = reject;
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let island, search;
async function scan() {
  const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
  const pages = targets.filter((t) => t.type === "page");
  for (const page of pages) {
    const cdp = await connect(page.webSocketDebuggerUrl);
    const ev = async (expr) => {
      const r = await cdp.send("Runtime.evaluate", {
        expression: expr,
        returnByValue: true,
        awaitPromise: true,
      });
      if (r.result?.exceptionDetails) throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 300));
      return r.result?.result?.value;
    };
    const hasIsland = await ev(`!!document.querySelector(".island")`);
    const hasPanel = await ev(`!!document.querySelector(".main-panel-root")`);
    if (hasIsland && !island) island = { cdp, ev, page };
    if (hasPanel && !search) search = { cdp, ev, page };
    if (!hasIsland && !hasPanel) cdp.close();
  }
}
await scan();
if (!island) { console.log("NO_ISLAND_WINDOW"); process.exit(1); }
if (!search) {
  // 主面板窗口隐藏未加载:双击岛呼出
  await island.ev(
    `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`,
  );
  await sleep(150);
  await island.ev(
    `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`,
  );
  await sleep(1200);
  await scan();
}
if (!search) { console.log("NO_SEARCH_WINDOW even after dblclick"); process.exit(1); }
console.log("WINDOWS: island + search OK");

// ===== 1. 岛回归:clip-path / 双击展开收回 / dragging =====
// 状态收敛:单击一次并等动画完全结束(模拟真实用户时序,避免上次测试的动画残留干扰双击判定)
await island.ev(
  `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`,
);
await sleep(350);
await island.ev(
  `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`,
);
await sleep(500);
const clip = await island.ev(
  `(() => { const cs = getComputedStyle(document.querySelector(".island")); return JSON.stringify({ clipPath: cs.clipPath }); })()`,
);
console.log("CLIP:", clip);
const dbl = (x) =>
  island.ev(
    `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: ${x}, clientY: 23 }))`,
  );
await dbl(60); await sleep(150); await dbl(60); await sleep(400);
const afterDbl = await island.ev(`document.querySelector(".island").classList.contains("expanded")`);
console.log("DBL_EXPAND(expect true):", afterDbl);
await sleep(500);
const autoCollapsed = await island.ev(`document.querySelector(".island").classList.contains("expanded")`);
console.log("AUTO_COLLAPSE_450ms(expect false):", autoCollapsed);
await island.ev(
  `(() => { const el = document.querySelector(".island");
    el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }));
    el.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: 80, clientY: 23 })); })()`,
);
await sleep(50);
console.log("DRAGGING_ON(expect true):", await island.ev(`document.querySelector(".island").classList.contains("dragging")`));
await sleep(250);
console.log("DRAGGING_OFF(expect false):", await island.ev(`document.querySelector(".island").classList.contains("dragging")`));

// ===== 2. 面板实时跟随岛(F1 高优)=====
// 注:set_position 程序性移动不触发 tauri://move(tao 抑制),CDP 无法模拟系统拖动;
// 正确验证 = 直接 emit island-geometry(等价岛拖动时 Island.emitGeometry 的输出)→ 断言面板消费端跟随
// (该链路消费端 = MainPanel listen → visible 门控 → followIsland → setPosition)
async function bounds(cdp, targetId) {
  const r = await cdp.send("Browser.getWindowForTarget", { targetId });
  const b = await cdp.send("Browser.getWindowBounds", { windowId: r.result.windowId });
  return b.result.bounds;
}
const TAU = `window.__TAURI_INTERNALS__`;
const s0 = await bounds(search.cdp, search.page.id);
console.log(`INIT: search@(${s0.left},${s0.top})`);
// 先确保面板 visible(tauri://show 已消费):hide→show 走官方事件
await island.ev(`${TAU}.invoke("plugin:window|hide", { label: "search" }).catch(() => {})`);
await sleep(400);
await island.ev(`${TAU}.invoke("plugin:window|show", { label: "search" }).catch(() => {})`);
await sleep(800);
// 两次不同位置的 emit,面板都应实时跟随
let followPass = true;
for (const [i, px] of [[1, 300], [2, 900]]) {
  const before = await bounds(search.cdp, search.page.id);
  await island.ev(`${TAU}.invoke("plugin:event|emit", { event: "island-geometry", payload: { x: ${px}, y: ${300 + i * 100}, w: 680, h: 46 } }).catch((e) => "ERR:" + e)`);
  await sleep(700);
  const after = await bounds(search.cdp, search.page.id);
  const dx = after.left - before.left, dy = after.top - before.top;
  console.log(`FOLLOW#${i}: emit@(${px},${300 + i * 100}) search Δ(${dx},${dy}) bounds=(${after.left},${after.top})`);
  if (dx === 0 && dy === 0) followPass = false;
}
console.log("FOLLOW_RESULT(expect search moved twice):", followPass ? "PASS" : "FAIL");

// ===== 3. 最近访问引导空态(F3 高优)=====
const KEY = "aurora-recent-apps";
const backup = await search.ev(`localStorage.getItem(${JSON.stringify(KEY)})`);
await search.ev(`localStorage.removeItem(${JSON.stringify(KEY)})`);
// 切到搜索视图(按钮 title=搜索;SearchView 不缓存,每次进入全新挂载→onMounted loadRecents)
await search.ev(`document.querySelector('.main-panel-root button[title="搜索"]')?.click()`);
await sleep(400);
const guide = await search.ev(
  `(() => { const es = document.querySelector(".empty-state"); return es ? es.innerText.replace(/\\n/g, "|") : "NO_EMPTY_STATE"; })()`,
);
console.log("EMPTY_GUIDE:", guide);
console.log("GUIDE_RESULT(expect 输入以搜索):", guide.includes("输入以搜索") ? "PASS" : "FAIL");
// 恢复
if (backup === null) await search.ev(`localStorage.removeItem(${JSON.stringify(KEY)})`);
else await search.ev(`localStorage.setItem(${JSON.stringify(KEY)}, ${JSON.stringify(backup)})`);
await search.ev(`document.querySelector('.main-panel-root button[title="小桌面"]')?.click()`);
console.log("RECENTS_RESTORED");

// ===== 4. enable_dock 门控(F1)=====
const cfg = await island.ev(`${TAU}.invoke("config_load")`);
if (cfg && typeof cfg.enable_dock === "boolean") {
  const orig = cfg.enable_dock;
  await island.ev(`${TAU}.invoke("config_save", { cfg: ${JSON.stringify({ ...cfg, enable_dock: false })} })`);
  await sleep(600); // config-saved → island applyConfig
  // 展开岛查 Dock 是否消失
  await dbl(60); await sleep(150); await dbl(60); await sleep(400);
  const dockHidden = await island.ev(`document.querySelectorAll(".mini-dock").length`);
  console.log("DOCK_WHEN_DISABLED(expect 0):", dockHidden);
  await island.ev(`${TAU}.invoke("config_save", { cfg: ${JSON.stringify({ ...cfg, enable_dock: orig })} })`);
  await sleep(600);
  await dbl(60); await sleep(150); await dbl(60); await sleep(400);
  const dockBack = await island.ev(`document.querySelectorAll(".mini-dock").length`);
  console.log("DOCK_WHEN_ENABLED(expect 1):", dockBack);
  console.log("ENABLE_DOCK_RESULT:", dockHidden === 0 && dockBack === 1 ? "PASS" : "FAIL");
} else {
  console.log("CONFIG_LOAD:", cfg ? "missing enable_dock field" : "failed", "- skip");
}

island.cdp.close(); search.cdp.close();
console.log("CDP-V021-R1 DONE");
process.exit(0);
