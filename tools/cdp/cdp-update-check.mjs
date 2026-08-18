// C1 自动更新检查链路验证(设置页手动检查 → update_check 真实网络 → 已是最新)
// 前置:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 启动 release exe
// 用法:node cdp-update-check.mjs
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

let search;
for (const page of targets.filter((t) => t.type === "page")) {
  const cdp = await connect(page.webSocketDebuggerUrl);
  const ev = async (expr) => {
    const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.result?.exceptionDetails) throw new Error("EVAL FAIL: " + JSON.stringify(r.result.exceptionDetails).slice(0, 300));
    return r.result?.result?.value;
  };
  if (await ev(`!!document.querySelector(".main-panel-root")`)) search = { cdp, ev };
}
if (!search) { console.log("NO_PANEL — 主面板窗口未找到"); process.exit(1); }

let pass = 0, fail = 0;
function chk(name, cond, detail = "") {
  if (cond) { pass++; console.log(`PASS ${name}`); }
  else { fail++; console.log(`FAIL ${name} ${detail}`); }
}

// 1. 呼出面板并切到设置视图
await search.ev(`window.__TAURI_INTERNALS__.invoke("open_search")`);
await sleep(800);
await search.ev(`[...document.querySelectorAll(".view-switch .view-btn")].find((b) => b.getAttribute("aria-label")?.includes("设置"))?.click()`);
await sleep(800);
const onSettings = await search.ev(`!!document.querySelector("button[aria-label='检查更新']")`);
chk("C1a 设置视图已打开且更新区块渲染", onSettings);

// 2. 点击手动检查(真实网络:拉 latest.json → 版本比较)
await search.ev(`document.querySelector("button[aria-label='检查更新']").click()`);
chk("C1b 点击后按钮进入检查中态", await search.ev(`document.querySelector("button[aria-label='检查更新']")?.textContent?.includes("检查中")`));

// 3. 轮询等待三态结果(最长 30s,覆盖网络波动)
let statusText = "timeout";
for (let i = 0; i < 30; i++) {
  await sleep(1000);
  const t = await search.ev(`document.body.innerText`);
  if (t.includes("已是最新版本")) { statusText = "latest"; break; }
  if (t.includes("发现新版本")) { statusText = "available"; break; }
  // 精确匹配错误条文案(勿用整页正则,"无法操作管理员窗口"等正常文案含"无法")
  if (/检查更新失败|更新源响应异常|更新源格式不合法|更新失败|自动更新已关闭/.test(t)) { statusText = "error"; break; }
  if (!(await search.ev(`document.querySelector("button[aria-label='检查更新']")?.textContent?.includes("检查中")`))) { statusText = "idle-no-result"; break; }
}
chk("C1c 检查结果=已是最新版本(更新源 0.2.5)", statusText === "latest", `status=${statusText}`);
if (statusText === "latest") {
  const ver = await search.ev(`document.body.innerText.split("\\n").find((l) => l.includes("已是最新"))`);
  console.log("    文本:", ver);
}

// 4. 收尾:Esc 关面板,不污染配置
await search.cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27 });
await search.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27 });
await sleep(300);

console.log(`\nSUMMARY ${pass} pass / ${fail} fail`);
process.exit(fail ? 1 : 0);
