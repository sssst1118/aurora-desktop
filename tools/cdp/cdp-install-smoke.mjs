// 安装版冒烟(B4-1 NSIS 安装链路验证)
// 前置:已用 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222
//       启动 %LOCALAPPDATA%\Aurora\aurora-desktop.exe
// 目标:验证真实安装产物非残缺 —— 窗口渲染正确/连上真实配置/核心交互链路通/零配置污染
// 用法:node cdp-install-smoke.mjs
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();

function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let id = 0;
    const pending = new Map();
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
    };
    ws.onopen = () => resolve({
      ws,
      send: (method, params) => new Promise((res) => {
        const i = ++id; pending.set(i, res);
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
    const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.result?.exceptionDetails) throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 300));
    return r.result?.result?.value;
  };
  const hasIsland = await ev(`!!document.querySelector(".island")`);
  const hasPanel = await ev(`!!document.querySelector(".main-panel-root")`);
  if (hasIsland && !island) island = { cdp, ev, page };
  if (hasPanel && !search) search = { cdp, ev, page };
}
if (!island) { console.log("NO_ISLAND — 安装版岛窗口 DOM 缺失"); process.exit(1); }

let pass = 0, fail = 0;
function chk(name, cond, detail = "") {
  if (cond) { pass++; console.log(`PASS ${name}`); }
  else { fail++; console.log(`FAIL ${name} ${detail}`); }
}
const sWin = (label) => search.ev(`window.__TAURI_INTERNALS__.invoke("plugin:window|is_visible", { label: "${label}" })`);
const sIslandWin = () => island.ev(`window.__TAURI_INTERNALS__.invoke("plugin:window|is_visible", { label: "island" })`);

// S1 窗口 DOM 正确(tauri.localhost 页面,非 devUrl/错误页)
chk("S1 岛窗口 DOM(.island 渲染)", true);
chk("S1b 主面板窗口 DOM(.main-panel-root 渲染)", !!search);
const pageUrlOk = targets.filter((t) => t.type === "page").every((t) => t.url === "http://tauri.localhost/");
chk("S1c 全部页面 URL=tauri.localhost(打包资源,非 devUrl)", pageUrlOk, JSON.stringify(targets.map((t) => t.url)));

// S2 岛窗口常驻可见
chk("S2 岛窗口 is_visible=true", await sIslandWin());

// S3 连上真实配置(安装版读 %APPDATA%\com.aurora.desktop\config.json)
const cfg = await search.ev(`window.__TAURI_INTERNALS__.invoke("config_load")`);
chk("S3 config_load 成功且返回对象", !!cfg && typeof cfg === "object");
const snap = {
  x: cfg?.island_x, y: cfg?.island_y,
  skin: cfg?.skin, theme: cfg?.theme_mode,
  hotkey: cfg?.hotkey_drawer ?? cfg?.hotkeys?.drawer,
};
chk("S3b 岛位置记忆字段存在(非 None)", Number.isFinite(snap.x) && Number.isFinite(snap.y), JSON.stringify(snap));
console.log("    config 快照:", JSON.stringify(snap));

// S4 呼出主面板(open_search = 双击岛/热键同链路)
await search.ev(`window.__TAURI_INTERNALS__.invoke("open_search")`);
await sleep(900);
chk("S4 呼出后 search 窗口 is_visible=true", await sWin("search"));

// S5 面板五视图按钮 + 默认视图小桌面
const panel = await search.ev(`JSON.stringify({
  viewBtns: [...document.querySelectorAll(".view-switch .view-btn")].length,
  activeView: document.querySelector(".view-switch .view-btn.on")?.getAttribute("aria-label") ?? "?",
  viewId: document.querySelector(".view-switch .view-btn.on")?.dataset?.view ?? "?",
})`).then(JSON.parse);
chk("S5 视图按钮齐全(5 视图+关闭≥6)", panel.viewBtns >= 6, JSON.stringify(panel));
chk("S5b 默认视图=小桌面", panel.activeView.includes("小桌面"), JSON.stringify(panel));

// S6 切换到搜索视图,head-input 出现(打字即搜链路)
await search.ev(`[...document.querySelectorAll(".view-switch .view-btn")].find((b) => b.dataset?.view === "search")?.click()`);
await sleep(600);
const sv = await search.ev(`JSON.stringify({
  input: !!document.querySelector(".head-input"),
  inputVal: document.querySelector(".head-input")?.value ?? "",
})`).then(JSON.parse);
chk("S6 搜索视图输入框渲染", sv.input, JSON.stringify(sv));

// S7 Esc 关面板(is_visible 实证)
const pressKey = async (key, code, vk) => {
  const p = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: 0 };
  await search.cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...p });
  await search.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...p });
  await sleep(250);
};
await pressKey("Escape", "Escape", 27);
chk("S7 Esc 后 search 窗口 is_visible=false", !(await sWin("search")));

// S8 配置零污染(再次读取对比)
const cfg2 = await search.ev(`window.__TAURI_INTERNALS__.invoke("config_load")`);
const same = JSON.stringify({ x: cfg2?.island_x, y: cfg2?.island_y, skin: cfg2?.skin, theme: cfg2?.theme_mode }) ===
             JSON.stringify({ x: snap.x, y: snap.y, skin: snap.skin, theme: snap.theme });
chk("S8 冒烟后配置零污染(位置/皮肤/主题不变)", same);

console.log(`\nSUMMARY ${pass} pass / ${fail} fail`);
process.exit(fail ? 1 : 0);
