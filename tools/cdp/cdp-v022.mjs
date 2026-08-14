// CDP 验证 v0.2.2 波次 2 修复:
// 1. Esc 三级递进(G1):①搜索态清输入回小桌面 ②岛展开先收岛(面板保持) ③岛收起再关面板
// 2. reduced-motion(G2):emulate prefers-reduced-motion → Settings/SmallDesktop 过渡归零
// 3. 回归:双击呼出面板
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
// 呼出面板(双击岛)
const dbl = async () => {
  await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
  await sleep(150);
  await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
};
await dbl();
await sleep(250); // 面板呼出+岛展开(450ms 自动收回窗口内)
await scan();
if (!search) { console.log("NO_SEARCH after dbl"); process.exit(1); }
const TAU = `window.__TAURI_INTERNALS__`;
const islExp = () => island.ev(`document.querySelector(".island").classList.contains("expanded")`);
const panelVis = () => search.ev(`${TAU}.invoke("plugin:window|is_visible", { label: "search" }).then((v) => v).catch(() => null)`);
const esc = () => search.ev(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);

console.log("STATE_AFTER_DBL: islandExpanded=", await islExp(), " panelVisible=", await panelVis());

// ===== 1. Esc 二级:岛展开时先收岛,面板保持 =====
// 关键时序:在 450ms 自动收回前发 Esc(现在距双击 ~250ms,还有 ~200ms 窗口)
await esc();
await sleep(150);
const expAfterEsc1 = await islExp(); // 距双击 400ms<450ms:若 Esc 生效已收,否则仍展开
const visAfterEsc1 = await panelVis();
console.log(`ESC_L2: islandExpanded=${expAfterEsc1}(expect false) panelVisible=${visAfterEsc1}(expect true)`);
console.log("ESC_L2_RESULT:", expAfterEsc1 === false && visAfterEsc1 === true ? "PASS" : "FAIL");
await sleep(400); // 等可能存在的自动收回定时器走完,避免干扰下一级

// ===== 2. Esc 三级:岛已收起,再 Esc 关面板 =====
await esc();
await sleep(500);
const visAfterEsc2 = await panelVis();
console.log(`ESC_L3: panelVisible=${visAfterEsc2}(expect false)`);
console.log("ESC_L3_RESULT:", visAfterEsc2 === false ? "PASS" : "FAIL");

// ===== 3. 重新呼出+一级回归:搜索态 Esc 清输入回小桌面 =====
await dbl();
await sleep(700);
await scan();
if (!search) { console.log("NO_SEARCH on reopen"); process.exit(1); }
// 打字进入搜索态:打字即搜走 keydown 可见字符(MainPanel onWindowKeydown),派发 keydown "a"
await search.ev(`window.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }))`);
await sleep(400);
const inSearch = await search.ev(`document.querySelector(".view-btn.on")?.getAttribute("title")`);
console.log("IN_SEARCH_VIEW:", inSearch, "(expect 搜索)");
await esc();
await sleep(400);
const q = await search.ev(`document.querySelector(".main-panel-root input")?.value ?? "none"`);
const backDesktop = await search.ev(`document.querySelector(".view-btn.on")?.getAttribute("title")`);
const visAfterEsc3 = await panelVis();
console.log(`ESC_L1: query=${JSON.stringify(q)}(expect 空) backToDesktop=${backDesktop}(expect 小桌面) panelVisible=${visAfterEsc3}(expect true)`);
console.log("ESC_L1_RESULT:", q === "" && backDesktop === "小桌面" && visAfterEsc3 === true ? "PASS" : "FAIL");

// ===== 4. reduced-motion(Settings 按钮过渡归零) =====
await search.cdp.send("Emulation.setEmulatedMedia", {
  features: [{ name: "prefers-reduced-motion", value: "reduce" }],
});
// 打开设置视图
await search.ev(`document.querySelector(".main-panel-root button[title=设置]")?.click()`);
await sleep(500);
const btnTrans = await search.ev(`(() => { const b = document.querySelector(".main-panel-root button"); const cs = getComputedStyle(b); return JSON.stringify({ duration: cs.transitionDuration, prop: cs.transitionProperty }); })()`);
console.log("REDUCE_MOTION_BTN:", btnTrans);
await search.cdp.send("Emulation.setEmulatedMedia", { features: [] });
const btnTrans2 = await search.ev(`(() => { const b = document.querySelector(".main-panel-root button"); const cs = getComputedStyle(b); return JSON.stringify({ duration: cs.transitionDuration, prop: cs.transitionProperty }); })()`);
console.log("NORMAL_BTN:", btnTrans2);
const rmPass = JSON.parse(btnTrans).duration !== JSON.parse(btnTrans2).duration && parseFloat(JSON.parse(btnTrans).duration) < 0.01;
console.log("REDUCE_MOTION_RESULT:", rmPass ? "PASS" : "FAIL(注意:非reduce下也可能部分归零,看对比)");

// 复位:回小桌面
await search.ev(`document.querySelector(".main-panel-root button[title=小桌面]")?.click()`);
island.cdp.ws.close();
search.cdp.ws.close();
console.log("CDP-V022 DONE");
process.exit(0);
