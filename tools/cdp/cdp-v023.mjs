// CDP 验证 v0.2.3 波次 3 修复:
// 1. UX-高1:保存设置不丢岛位置(config_load→config_save 往返 island_x/y 保留)
// 2. UX-中1:Esc 焦点豁免(焦点在输入框时 Esc 不劫持)
// 3. 回归:双击呼出/Esc 三级仍正常
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
async function scan() {
  const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
  for (const page of targets.filter((t) => t.type === "page")) {
    const cdp = await connect(page.webSocketDebuggerUrl);
    const ev = async (expr) => {
      const r = await cdp.send("Runtime.evaluate", {
        expression: expr,
        returnByValue: true,
        awaitPromise: true,
      });
      if (r.result?.exceptionDetails)
        throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 200));
      return r.result?.result?.value;
    };
    const hasIsland = await ev(`!!document.querySelector(".island")`);
    const hasPanel = await ev(`!!document.querySelector(".main-panel-root")`);
    if (hasIsland && !island) island = { cdp, ev, page };
    if (hasPanel && !search) search = { cdp, ev, page };
  }
}
await scan();
if (!island) { console.log("NO_ISLAND"); process.exit(1); }
const TAU = `window.__TAURI_INTERNALS__`;

// ===== 1. UX-高1:config 往返岛位置不丢 =====
const before = await island.ev(`${TAU}.invoke("config_load")`);
console.log("CFG_BEFORE island:", JSON.stringify({ x: before?.island_x, y: before?.island_y }));
// 模拟 Settings.saveSafe:整对象传 config_save(带 island_x/y)
const saveRet = await island.ev(
  `${TAU}.invoke("config_save", { cfg: ${JSON.stringify(before)} }).then((r) => "OK:" + JSON.stringify(r)).catch((e) => "ERR:" + String(e))`,
);
console.log("CONFIG_SAVE:", saveRet);
await sleep(500);
const after = await island.ev(`${TAU}.invoke("config_load")`);
const keepPos = before?.island_x !== undefined && after?.island_x === before?.island_x && after?.island_y === before?.island_y;
console.log(`CFG_AFTER island: ${JSON.stringify({ x: after?.island_x, y: after?.island_y })}`);
console.log("SAVE_KEEPS_ISLAND_POS_RESULT:", keepPos ? "PASS" : "FAIL");

// ===== 2. 呼出面板(Esc 测试前置) =====
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(150);
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(800);
await scan();
if (!search) { console.log("NO_SEARCH after dbl"); process.exit(1); }
const panelVis = () => search.ev(`${TAU}.invoke("plugin:window|is_visible", { label: "search" }).then((v) => v).catch(() => null)`);

// ===== 3. UX-中1:焦点在输入框时 Esc 豁免 =====
// 进入搜索态并把焦点放输入框
await search.ev(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }))`);
await sleep(400);
const inSearch1 = await search.ev(`document.querySelector(".view-btn.on")?.getAttribute("title")`);
// 聚焦输入框后派发 Esc(真实用户:输入框聚焦,按 Esc 应取消输入焦点/清空,但面板保持且不关)
await search.ev(`(() => { const i = document.querySelector(".main-panel-root input"); i.focus(); i.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); return document.activeElement === i; })()`);
await sleep(400);
const visAfterEscFocus = await panelVis();
const qAfter = await search.ev(`document.querySelector(".main-panel-root input")?.value ?? "none"`);
console.log(`ESC_IN_INPUT: panelVisible=${visAfterEscFocus}(expect true,不关面板) query=${JSON.stringify(qAfter)}`);
// 关键断言:面板不关(输入框 Esc 不劫持为收岛/关面板)
console.log("ESC_FOCUS_EXEMPT_RESULT:", visAfterEscFocus === true ? "PASS" : "FAIL");

// ===== 4. 回归:Esc 三级递进(搜索态一级清输入) =====
// 焦点移出输入框,发窗口级 Esc
await search.ev(`document.activeElement?.blur()`);
await search.ev(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);
await sleep(400);
const backTo = await search.ev(`document.querySelector(".view-btn.on")?.getAttribute("title")`);
const visL1 = await panelVis();
console.log(`ESC_L1_REGRESSION: backTo=${backTo}(expect 小桌面) panelVisible=${visL1}(expect true)`);
console.log("ESC_L1_RESULT:", backTo === "小桌面" && visL1 === true ? "PASS" : "FAIL");

// ===== 5. 回归:岛展开时 Esc 收岛(二级)+再 Esc 关面板(三级) =====
const islExp = () => island.ev(`document.querySelector(".island").classList.contains("expanded")`);
// 单击岛展开(此时面板开、岛展开)
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(400);
console.log("ISLAND_EXP_BEFORE_ESC:", await islExp());
await search.ev(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);
await sleep(300);
console.log("ISLAND_EXP_AFTER_ESC_L2:", await islExp(), "(expect false)");
console.log("PANEL_VIS_AFTER_L2:", await panelVis(), "(expect true)");
await search.ev(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);
await sleep(500);
console.log("PANEL_VIS_AFTER_L3:", await panelVis(), "(expect false)");

island.cdp.ws.close();
search.cdp.ws.close();
console.log("CDP-V023 DONE");
process.exit(0);
