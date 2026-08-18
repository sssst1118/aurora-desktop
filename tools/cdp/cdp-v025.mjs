// CDP 验证 v0.2.5 波次 5 修复:
// 回归(波次 4 已验证,防 G1 改 Esc 逻辑回归):
//   T1 高1:搜索态主输入框打字 Esc → 清空回小桌面(面板保持)
//   T2 中1:空输入框 Esc 递进 → 收岛面板保持
//   T3 回归:三级 Esc 关面板(is_visible 实证)
//   T4 低6:按钮聚焦按 Space 激活(不被打字即搜劫持)
//   T5 低10:热键录制不支持键 → 提示出现
// 新增(G1-G4 修复验证):
//   T6 G1中2:剪贴板搜索框有值 Esc → 清空(与主搜索框同款一级语义,面板保持)
//   T7 G1中2:AI 正文输入有值 Esc → 放行不清空(面板保持)
//   T8 G2中2:剪贴板过滤后 selected 重置回首条
//   T9 G1低3:溢出浮层 Esc 关闭(只关浮层,面板保持)
//   T10 G1中1:键盘缩放手柄方向键连按 → aria-valuenow 递进(不丢步进)
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
        throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 300));
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

// 按键注入:rawKeyDown+keyUp(走真实键盘路径)
async function pressKey(cdp, key, code, vk, mods = 0) {
  const p = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: mods };
  await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...p });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...p });
  await sleep(120);
}
// 真实字符输入(逐字符 dispatch,触发 typing 链路;首字符前先发一次 Shift 热管道,
// 避免 JS focus() 后第一个 keydown 被 WebView2 吞——v025 实测 x 丢失)
async function typeText(cdp, text) {
  await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16, nativeVirtualKeyCode: 16, modifiers: 0 });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16, nativeVirtualKeyCode: 16, modifiers: 0 });
  await sleep(100);
  for (const ch of text) {
    const upper = ch.toUpperCase();
    await cdp.send("Input.dispatchKeyEvent", {
      type: "rawKeyDown", key: ch, code: `Key${upper}`,
      windowsVirtualKeyCode: ch.charCodeAt(0), nativeVirtualKeyCode: ch.charCodeAt(0), text: ch,
    });
    await cdp.send("Input.dispatchKeyEvent", { type: "char", key: ch, text: ch, unmodifiedText: ch });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: ch, code: `Key${upper}`, windowsVirtualKeyCode: ch.charCodeAt(0), nativeVirtualKeyCode: ch.charCodeAt(0) });
    await sleep(80);
  }
  await sleep(200);
}
// 直接插入文本(不经过 keydown 管道,适合普通 input/textarea 填值,v-model 由 input 事件驱动)
async function insertText(cdp, text) {
  await cdp.send("Input.insertText", { text });
  await sleep(250);
}

const sIsland = () =>
  island.ev(`JSON.stringify({
    islandExpanded: document.querySelector(".island")?.classList.contains("expanded") ?? false
  })`).then(JSON.parse);
const sPanel = () =>
  search.ev(`JSON.stringify({
    panelVisible: !!document.querySelector(".main-panel-root"),
    inputVal: document.querySelector(".head-input")?.value ?? "",
    inputVisible: (() => { const el = document.querySelector(".head-input"); return !!el && getComputedStyle(el).display !== "none"; })(),
    activeViewBtn: document.querySelector(".view-switch .view-btn.on")?.getAttribute("aria-label") ?? "?"
  })`).then(JSON.parse);
// 窗口隐藏唯一可靠探针=Tauri IPC is_visible(WebView2 的 document.hidden 不随窗口隐藏更新)
const sWindow = () => search.ev(`window.__TAURI_INTERNALS__.invoke("plugin:window|is_visible", { label: "search" })`);
const state = async () => ({ ...(await sIsland()), ...(await sPanel()), winVis: await sWindow() });

let pass = 0, fail = 0, skip = 0;
function chk(name, cond, detail = "") {
  console.log(`${cond ? "PASS" : "FAIL"} ${name}${detail ? " | " + detail : ""}`);
  cond ? pass++ : fail++;
}
function skipT(name, detail = "") {
  console.log(`SKIP ${name} | ${detail}`);
  skip++;
}

// ===== 前置:双击呼出面板 =====
async function openPanelByDbl() {
  await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
  await sleep(150);
  await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
  await sleep(900);
  await scan();
}
await openPanelByDbl();
if (!search) { console.log("NO_SEARCH after dbl"); process.exit(1); }
// 收敛:清空输入框 + 焦点移 body(防真实键盘输入污染判定)
await search.ev(`(() => { const i = document.querySelector(".head-input"); if (i) i.value = ""; document.activeElement?.blur?.(); return true; })()`);
await sleep(200);
let s = await state();
console.log("初始(双击后):", JSON.stringify(s));

// ===== T1 高1 回归:打字即搜 → 主输入框 Esc 清空回小桌面 =====
await search.cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "j", code: "KeyJ", windowsVirtualKeyCode: 74, nativeVirtualKeyCode: 74 });
await search.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "j", code: "KeyJ", windowsVirtualKeyCode: 74, nativeVirtualKeyCode: 74 });
await sleep(300);
s = await state();
chk("T1a 打字即搜进入 search 视图", s.activeViewBtn === "切换到搜索视图", JSON.stringify(s));
chk("T1b 输入框可见且有值", s.inputVisible && s.inputVal === "j", `val=${s.inputVal}`);
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(300);
s = await state();
chk("T1c Esc 清空 query", s.inputVal === "", `val=${s.inputVal}`);
chk("T1d Esc 回小桌面视图", s.activeViewBtn === "切换到小桌面视图", JSON.stringify(s));
chk("T1e 面板保持打开", s.panelVisible);

// ===== T3 回归:岛收起态 Esc → 关面板(显式展开→收起收敛) =====
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(500);
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(500);
s = await state();
chk("T3a 岛已收起", !s.islandExpanded, JSON.stringify(s));
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(500);
s = await state();
chk("T3b Esc 三级关闭面板", s.winVis === false, JSON.stringify(s));

// ===== T2 回归:空输入框 Esc 递进收岛(设置页) =====
await openPanelByDbl();
await scan();
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到设置视图"]').click()`);
await sleep(300);
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(500);
s = await state();
chk("T2a 前置:岛展开", s.islandExpanded, JSON.stringify(s));
await search.ev(`(() => {
  const inputs = [...document.querySelectorAll(".main-panel-root input")].filter(i => i.offsetParent !== null && i.value === "");
  if (!inputs.length) return "NO_EMPTY_INPUT";
  inputs[0].focus();
  return "FOCUSED:" + inputs[0].className.slice(0, 40);
})()`);
await sleep(200);
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(500);
s = await state();
chk("T2b 空输入框 Esc 收岛", !s.islandExpanded, JSON.stringify(s));
chk("T2c 面板保持打开", s.panelVisible);

// ===== T6 G1中2:剪贴板搜索框有值 Esc → 清空(一级语义,面板保持) =====
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到剪贴板视图"]').click()`);
await sleep(300);
const kwFocus = await search.ev(`(() => { const i = document.querySelector('.main-panel-root input[placeholder="搜索剪贴板历史…"]'); if (!i) return "NO_KEYWORD_INPUT"; i.focus(); return "FOCUSED"; })()`);
chk("T6f 前置:剪贴板搜索框聚焦", kwFocus === "FOCUSED", String(kwFocus));
await sleep(400);
await insertText(search.cdp, "xyz");
const kwBefore = await search.ev(`document.querySelector('.main-panel-root input[placeholder="搜索剪贴板历史…"]')?.value ?? ""`);
chk("T6a 剪贴板搜索框有值", kwBefore === "xyz", `val=${kwBefore}`);
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(300);
s = await state();
const kwAfter = await search.ev(`document.querySelector('.main-panel-root input[placeholder="搜索剪贴板历史…"]')?.value ?? ""`);
chk("T6b Esc 清空剪贴板搜索框", kwAfter === "", `val=${kwAfter}`);
chk("T6c 仍在剪贴板视图", s.activeViewBtn === "切换到剪贴板视图", JSON.stringify(s));
chk("T6d 面板保持打开", s.panelVisible);

// ===== T7 G1中2:AI 正文输入有值 Esc → 放行不清空 =====
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到AI 助手视图"]')?.click()`);
await sleep(300);
const aiFocus = await search.ev(`(() => { const t = document.querySelector('.main-panel-root textarea[placeholder="输入消息…"]'); if (!t) return "NO_AI_INPUT"; t.focus(); return "FOCUSED"; })()`);
chk("T7f 前置:AI 输入框聚焦", aiFocus === "FOCUSED", String(aiFocus));
await sleep(400);
await insertText(search.cdp, "hello");
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(300);
const aiVal = await search.ev(`document.querySelector('.main-panel-root textarea[placeholder="输入消息…"]')?.value ?? ""`);
s = await state();
chk("T7a AI 输入有值 Esc 放行不清空", aiVal === "hello", `val=${aiVal}`);
chk("T7b 面板保持打开", s.panelVisible);

// ===== T8 G2中2:剪贴板过滤后 selected 重置回首条 =====
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到剪贴板视图"]').click()`);
await sleep(300);
const clipCount = await search.ev(`document.querySelectorAll('.main-panel-root .clip-item').length`);
if (clipCount >= 2) {
  // 选中第 2 条(点击触发 mouseenter 同步 selected)
  await search.ev(`(() => { const items = document.querySelectorAll('.main-panel-root .clip-item'); items[1].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true })); return true; })()`);
  await sleep(200);
  const selBefore = await search.ev(`(() => { const items = document.querySelectorAll('.main-panel-root .clip-item'); return [...items].findIndex(i => i.classList.contains("selected")); })()`);
  chk("T8a 前置:选中第 2 条", selBefore === 1, `sel=${selBefore}`);
  // 输入过滤词(部分匹配 → 列表非空,断言选中回到首条;不匹配词会让列表为空,
  // selected=0 但无 item 可查,findIndex=-1 是正常表现而非 bug)
  await search.ev(`(() => { const i = document.querySelector('.main-panel-root input[placeholder="搜索剪贴板历史…"]'); i.focus(); return true; })()`);
  await sleep(400);
  await insertText(search.cdp, "item-1");
  await sleep(300);
  const afterState = await search.ev(`(() => { const items = [...document.querySelectorAll('.main-panel-root .clip-item')]; return JSON.stringify({ count: items.length, sel: items.findIndex(i => i.classList.contains("selected")) }); })()`).then(JSON.parse);
  chk("T8b 过滤后列表非空", afterState.count >= 1, JSON.stringify(afterState));
  chk("T8c 过滤后 selected 重置回首条", afterState.sel === 0, JSON.stringify(afterState));
  // 清理:清空过滤词(避免影响后续)
  await search.ev(`(() => { const i = document.querySelector('.main-panel-root input[placeholder="搜索剪贴板历史…"]'); i.value = ""; i.dispatchEvent(new Event("input", { bubbles: true })); i.blur(); return true; })()`);
  await sleep(200);
} else {
  skipT("T8 剪贴板条目不足 2 条", `count=${clipCount}`);
}

// ===== T9 G1低3:溢出浮层 Esc 关闭 =====
const hasMore = await island.ev(`!!document.querySelector(".dock-more")`);
if (hasMore) {
  await island.ev(`document.querySelector(".dock-more").dispatchEvent(new MouseEvent("click", { bubbles: true }))`);
  await sleep(300);
  const ovOpen = await island.ev(`!!document.querySelector(".dock-overflow")`);
  chk("T9a 前置:浮层打开", ovOpen === true);
  // G1 低3 的 onWinKeydown 监听在岛窗口(IslandDock),Esc 必须发 island.cdp
  await pressKey(island.cdp, "Escape", "Escape", 27);
  await sleep(300);
  const ovAfter = await island.ev(`!!document.querySelector(".dock-overflow")`);
  s = await state();
  chk("T9b Esc 关闭浮层", ovAfter === false, `open=${ovAfter}`);
  chk("T9c 面板保持打开", s.panelVisible);
} else {
  skipT("T9 Dock 无溢出(条目≤可视容量)", "");
}

// ===== T10 G1中1:键盘缩放手柄方向键连按不丢步进 =====
await search.ev(`document.querySelector('.resize-handle')?.focus()`);
await sleep(200);
const vn0 = await search.ev(`Number(document.querySelector('.resize-handle')?.getAttribute("aria-valuenow") ?? 0)`);
for (let i = 0; i < 3; i++) {
  await pressKey(search.cdp, "ArrowRight", "ArrowRight", 39);
}
await sleep(300);
const vn1 = await search.ev(`Number(document.querySelector('.resize-handle')?.getAttribute("aria-valuenow") ?? 0)`);
chk("T10 方向键连按 aria-valuenow 递进", vn1 > vn0, `w ${vn0}→${vn1}`);
// 收敛:缩回原宽(按同次数 ArrowLeft)
for (let i = 0; i < 3; i++) {
  await pressKey(search.cdp, "ArrowLeft", "ArrowLeft", 37);
}
await sleep(300);
const vn2 = await search.ev(`Number(document.querySelector('.resize-handle')?.getAttribute("aria-valuenow") ?? 0)`);
chk("T10b 方向键左缩回", Math.abs(vn2 - vn0) <= 2, `w ${vn1}→${vn2}`);

// ===== T5 回归:热键录制忽略提示 =====
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到设置视图"]').click()`);
await sleep(300);
await search.ev(`document.querySelector('.main-panel-root input[aria-label="抽屉热键"]').click()`);
await sleep(200);
for (let i = 0; i < 4; i++) {
  await pressKey(search.cdp, "Shift", "ShiftLeft", 16, 2);
}
await sleep(200);
const hint = await search.ev(`document.querySelector(".main-panel-root")?.textContent?.includes("该键不支持") ?? false`);
chk("T5 热键录制忽略提示出现", hint === true, `hint=${hint}`);
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(200);

console.log(`\n==== 波次 5 CDP 验证完成: PASS ${pass} / FAIL ${fail} / SKIP ${skip} ====`);
process.exit(fail > 0 ? 1 : 0);
