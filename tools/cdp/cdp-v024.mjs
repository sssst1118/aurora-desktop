// CDP 验证 v0.2.4 波次 4 修复:
// T1 高1:搜索态焦点在主输入框 Esc → 清空 query 回小桌面(波次 3 豁免回归)
// T2 中1:空输入框 Esc 递进 → 收岛面板保持(波次 3 全放行卡死修复)
// T3 回归:三级 Esc 关面板(岛已收起)
// T4 低6:按钮聚焦按 Space 不被打字即搜劫持
// T5 低10:热键录制连续 3 次不支持键 → 提示出现
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

// 按键注入:rawKeyDown+keyUp(用 Input.dispatchKeyEvent 走真实键盘路径)
async function pressKey(cdp, key, code, vk, mods = 0) {
  const p = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: mods };
  await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...p });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...p });
  await sleep(120);
}

// 岛状态/面板视图辅助
const state = () =>
  island.ev(`JSON.stringify({
    islandExpanded: document.querySelector(".island")?.classList.contains("expanded") ?? false,
    panelVisible: !!document.querySelector(".main-panel-root"),
    inputVal: document.querySelector(".head-input")?.value ?? "",
    inputVisible: getComputedStyle(document.querySelector(".head-input")).display !== "none",
    activeViewBtn: document.querySelector(".view-switch .view-btn.on")?.getAttribute("aria-label") ?? "?"
  })`).then(JSON.parse);

let pass = 0, fail = 0;
function chk(name, cond, detail = "") {
  console.log(`${cond ? "PASS" : "FAIL"} ${name}${detail ? " | " + detail : ""}`);
  cond ? pass++ : fail++;
}

// ===== 前置:双击呼出面板(参考 v023 手法,pointerdown 计数判定) =====
async function openPanelByDbl() {
  await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
  await sleep(150);
  await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
  await sleep(900);
  await scan();
}
await openPanelByDbl();
if (!search) { console.log("NO_SEARCH after dbl"); process.exit(1); }
let s = await state();
console.log("初始(双击后):", JSON.stringify(s));

// ===== T1 高1:打字即搜 → 主输入框 Esc 清空回小桌面 =====
// 打字 "j"(可见字符,焦点在 body → 打字即搜 → search 视图 + 输入框聚焦)
await search.cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "j", code: "KeyJ", windowsVirtualKeyCode: 74, nativeVirtualKeyCode: 74 });
await search.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "j", code: "KeyJ", windowsVirtualKeyCode: 74, nativeVirtualKeyCode: 74 });
await sleep(300);
s = await state();
chk("T1a 打字即搜进入 search 视图", s.activeViewBtn === "切换到搜索视图", JSON.stringify(s));
chk("T1b 输入框可见且有值", s.inputVisible && s.inputVal === "j", `val=${s.inputVal}`);
// Esc(焦点在主输入框,有值 → 高1 分支:清空回小桌面,面板保持)
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(300);
s = await state();
chk("T1c Esc 清空 query", s.inputVal === "", `val=${s.inputVal}`);
chk("T1d Esc 回小桌面视图", s.activeViewBtn === "切换到小桌面视图", JSON.stringify(s));
chk("T1e 面板保持打开", s.panelVisible);

// ===== T3 三级:岛收起态 Esc → 关面板 =====
// 确保岛收起(单击收起)
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(500);
s = await state();
chk("T3a 岛已收起", !s.islandExpanded, JSON.stringify(s));
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(500);
s = await state();
chk("T3b Esc 三级关闭面板", !s.panelVisible, JSON.stringify(s));

// ===== T2 中1:空输入框 Esc 递进收岛(设置页输入框聚焦空值) =====
// 重新呼出 + 展开岛 + 切设置视图 + 焦点放一个空输入框
await openPanelByDbl();
await scan();
// 切设置视图
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到设置视图"]').click()`);
await sleep(300);
// 展开岛(单击)
await island.ev(`document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }))`);
await sleep(500);
s = await state();
chk("T2a 前置:岛展开", s.islandExpanded, JSON.stringify(s));
// 焦点放设置页第一个空 input(避免 AI key 等有值输入框,取值为空的)
await search.ev(`(() => {
  const inputs = [...document.querySelectorAll(".main-panel-root input")].filter(i => i.offsetParent !== null && i.value === "");
  if (!inputs.length) return "NO_EMPTY_INPUT";
  inputs[0].focus();
  return "FOCUSED:" + inputs[0].className.slice(0, 40);
})()`);
await sleep(200);
// Esc:空值非组合 → 递进 → 收岛面板保持
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(500);
s = await state();
chk("T2b 空输入框 Esc 收岛", !s.islandExpanded, JSON.stringify(s));
chk("T2c 面板保持打开", s.panelVisible);

// ===== T4 低6:按钮聚焦按 Space 不切视图 =====
// 当前在设置视图;Tab 到视图按钮或直接 focus 一个 view-btn,按 Space → 视图不变
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到剪贴板视图"]').focus()`);
await sleep(150);
await pressKey(search.cdp, " ", "Space", 32);
await sleep(300);
s = await state();
chk("T4 Space 不劫持(仍设置视图)", s.activeViewBtn === "切换到设置视图", JSON.stringify(s));

// ===== T5 低10:热键录制连续 3 次不支持键提示 =====
// 点抽屉热键录制按钮(title="点击进入录制模式")
await search.ev(`document.querySelector('.main-panel-root button[title="点击进入录制模式"]').click()`);
await sleep(200);
// 连续 4 次纯修饰键 Shift(不支持 → 计数,≥3 提示)
for (let i = 0; i < 4; i++) {
  await pressKey(search.cdp, "Shift", "ShiftLeft", 16, 2);
}
await sleep(200);
const hint = await search.ev(`document.querySelector(".main-panel-root")?.textContent?.includes("该键不支持") ?? false`);
chk("T5 热键录制忽略提示出现", hint === true, `hint=${hint}`);
// 退出录制(Esc)
await pressKey(search.cdp, "Escape", "Escape", 27);
await sleep(200);

console.log(`\n==== 波次 4 CDP 验证完成: PASS ${pass} / FAIL ${fail} ====`);
process.exit(fail > 0 ? 1 : 0);
