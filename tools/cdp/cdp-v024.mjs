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

// 岛状态/面板视图辅助(岛与面板是两个 WebView 窗口,须分窗口查询)
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
// 窗口隐藏探针:win.hide() 后 DOM 元素仍在,.main-panel-root 存在性≠窗口可见;
// document.hidden/visibilityState 在 WebView2 里不随窗口隐藏更新(实测恒 visible),
// 唯一可靠探针=Tauri IPC is_visible(隐藏时返回 false)
const sWindow = () => search.ev(`window.__TAURI_INTERNALS__.invoke("plugin:window|is_visible", { label: "search" })`);
const state = async () => ({ ...(await sIsland()), ...(await sPanel()), winVis: await sWindow() });

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
// 收敛:清空输入框值 + 焦点移到 body(防真实键盘输入污染判定)
await search.ev(`(() => { const i = document.querySelector(".head-input"); if (i) i.value = ""; document.activeElement?.blur?.(); return true; })()`);
await sleep(200);
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
// 前置状态收敛:先单击展开岛,再单击收起(450ms 自动收回已让岛处于收起态,
// 直接"单击收起"会把收起态 toggle 成展开——T3a 第一版在此误判)
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

// ===== T4 低6:按钮聚焦按 Space = 原生激活,打字即搜不得劫持 =====
// 修复语义(MainPanel onWindowKeydown):按钮聚焦按 Space → 打字即搜放行,
// 按钮获得原生激活(视图切换)。断言方向:Space 后视图应切到剪贴板(按钮生效)
// 且输入框无字符(未被劫持成输入)——"视图不变"恰是 bug 行为。
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到剪贴板视图"]').focus()`);
await sleep(150);
await pressKey(search.cdp, " ", "Space", 32);
await sleep(300);
s = await state();
chk("T4 Space 激活按钮(切到剪贴板视图)", s.activeViewBtn === "切换到剪贴板视图", JSON.stringify(s));
chk("T4b 输入框无劫持字符", s.inputVal === "", `val=${s.inputVal}`);

// ===== T5 低10:热键录制连续 3 次不支持键提示 =====
// 前置:切回设置视图(T4 后停在剪贴板视图,录制入口在设置页;
// 注意:录制入口是 readonly input 不是 button,title="点击进入录制模式")
await search.ev(`document.querySelector('.view-switch .view-btn[aria-label="切换到设置视图"]').click()`);
await sleep(300);
await search.ev(`document.querySelector('.main-panel-root input[aria-label="抽屉热键"]').click()`);
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
