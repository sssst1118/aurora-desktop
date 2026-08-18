// 诊断 v0.2.4 三个悬案:
// D1 T3b:win.hide() 后 search 窗口的可见性探针(document.hidden / visibilityState / panelVisible)
// D2 T2b:inputVal "ng'zi" 来源(head-input 值 + 焦点元素)
// D3 T5:设置视图录制按钮选择器是否命中
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
const pages = targets.filter((t) => t.type === "page");
function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let id = 0;
    const pending = new Map();
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
    };
    ws.onopen = () => resolve({ ws, send: (m, p) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method: m, params: p })); }) });
    ws.onerror = reject;
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let island, search;
async function scan() {
  const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
  for (const page of targets.filter((t) => t.type === "page")) {
    const cdp = await connect(page.webSocketDebuggerUrl);
    const ev = async (expr) => {
      const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
      if (r.result?.exceptionDetails) throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 200));
      return r.result?.result?.value;
    };
    const hasIsland = await ev(`!!document.querySelector(".island")`);
    const hasPanel = await ev(`!!document.querySelector(".main-panel-root")`);
    if (hasIsland && !island) island = { cdp, ev };
    if (hasPanel && !search) search = { cdp, ev };
  }
}
await scan();
if (!search) { console.log("NO_SEARCH"); process.exit(1); }
async function pressKey(cdp, key, code, vk, mods = 0) {
  const p = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: mods };
  await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...p });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...p });
  await sleep(120);
}
const st = () => search.ev(`JSON.stringify({
  panelVisible: !!document.querySelector(".main-panel-root"),
  hidden: document.hidden,
  vis: document.visibilityState,
  inputVal: document.querySelector(".head-input")?.value ?? "",
  activeEl: document.activeElement?.tagName + "." + (document.activeElement?.className || "").slice(0, 30),
  activeView: document.querySelector(".view-switch .view-btn.on")?.getAttribute("aria-label") ?? "?"
})`).then(JSON.parse);

// D1:岛收起态 Esc → win.hide 后窗口可见性探针
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(300);
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(400);
let s = await st();
console.log("D1 前(岛应收起):", JSON.stringify(s));
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(500);
s = await st();
console.log("D1 后(Esc 三级):", JSON.stringify(s));
// 呼出回来(T2 用)
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(150);
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(800);

// D2:切设置视图,查空输入框与 head-input
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到设置视图"]').click()`);
await sleep(300);
s = await st();
console.log("D2 设置视图:", JSON.stringify(s));
const empties = await search.ev(`JSON.stringify([...document.querySelectorAll(".main-panel-root input")].filter(i => i.offsetParent !== null).map(i => ({ cls: i.className.slice(0, 30), val: i.value.slice(0, 20), placeholder: i.placeholder.slice(0, 20) })))`);
console.log("D2 可见输入框:", empties);

// D5:注入派发探针——window 捕获监听计数,确认 CDP 合成键事件是否到达页面
await search.ev(`window.__d5 = { count: 0, target: null, phase: null };
window.addEventListener("keydown", (e) => { window.__d5.count++; window.__d5.target = (e.target && (e.target.tagName + "." + (e.target.className || "").toString().slice(0, 25))) || String(e.target); window.__d5.phase = "bubble"; }, true);
true`);
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(300);
const d5 = await search.ev(`JSON.stringify(window.__d5)`);
console.log("D5 Esc 注入探针:", d5);
// D7:T2 场景复现——设置视图 + 岛展开 + Esc(带注入计数探针)
// 先呼出面板(窗口当前隐藏)
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(150);
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(900);
await scan();
// 切设置视图
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到设置视图"]').click()`);
await sleep(300);
// 展开岛(单击)
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(500);
// 挂探针
await search.ev(`window.__d7 = { count: 0 };
window.addEventListener("keydown", (e) => { if (e.key === "Escape") window.__d7.count++; }, true);
true`);
// 注入 Esc
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(500);
const d7 = await search.ev(`JSON.stringify({
  count: window.__d7.count,
  islandExpanded: document.querySelector(".island")?.classList.contains("expanded") ?? false,
  activeEl: document.activeElement?.tagName + "." + (document.activeElement?.className || "").toString().slice(0, 30),
  activeView: document.querySelector(".view-switch .view-btn.on")?.getAttribute("aria-label") ?? "?"
})`);
console.log("D7 T2场景(设置视图+岛展开+Esc):", d7);
// D6:窗口真实可见性探针——Tauri IPC is_visible(DOM/visibilityState 均不可靠)
async function winVisible() {
  const r = await search.ev(`window.__TAURI_INTERNALS__.invoke("plugin:window|is_visible", { label: "search" })`);
  return r;
}
console.log("D6 search 窗口 is_visible:", await winVisible());
// D4:Esc 不生效根因——焦点元素与两次注入
// 前置:岛收起 + 小桌面视图(复用 D1 结束状态:面板开、设置视图;先回小桌面)
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到小桌面视图"]').click()`);
await sleep(300);
// 确保岛收起:展开再收起
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(300);
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(400);
let s2 = await st();
console.log("D4 前:", JSON.stringify(s2));
// 第一次 Esc
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(400);
s2 = await st();
console.log("D4 第一次 Esc 后:", JSON.stringify(s2));
// 第二次 Esc
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(400);
s2 = await st();
console.log("D4 第二次 Esc 后:", JSON.stringify(s2));
process.exit(0);
