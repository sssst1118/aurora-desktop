// CDP 实测:岛展开态 Dock 动态可视容量 + 溢出「…」浮层
// 前置:config dock_items 已注入 10 条;exe 以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222" 启动
const targets = await (await fetch("http://127.0.0.1:9222/json")).json();
const pages = targets.filter((t) => t.type === "page");

async function connect(wsUrl) {
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

for (const page of pages) {
  const cdp = await connect(page.webSocketDebuggerUrl);
  const evalJs = async (expr) => {
    const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true });
    return r.result?.result?.value;
  };
  const hasIsland = await evalJs(`!!document.querySelector(".island")`);
  if (!hasIsland) {
    cdp.close();
    continue;
  }
  // 模拟单击(岛空白处 x=8)→ 240ms 判定 + 280ms 展开动画
  await evalJs(
    `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 8, clientY: 23 }))`,
  );
  await sleep(400);
  await sleep(400);
  const probe = await evalJs(`(() => {
    const dock = document.querySelector(".mini-dock");
    const tiles = [...document.querySelectorAll(".mini-dock .dock-tile")];
    const visible = tiles.filter(t => { const r = t.getBoundingClientRect(); return r.left >= 0 && r.right <= window.innerWidth && r.width > 0; });
    return JSON.stringify({
      winW: window.innerWidth,
      expanded: document.querySelector(".island").classList.contains("expanded"),
      dockClientW: Math.round(dock.clientWidth),
      tileCount: tiles.length,
      tilesVisible: visible.length,
      moreBtn: !!document.querySelector(".dock-more"),
      addBtn: !!document.querySelector(".dock-add-mini"),
      tileTitles: tiles.map(t => t.getAttribute("title")),
      clipped: tiles.filter(t => { const r = t.getBoundingClientRect(); return r.right > window.innerWidth; }).map(t => t.getAttribute("title")),
    });
  })()`);
  console.log("EXPANDED:", probe);
  // 点「…」→ 浮层应列全 10 条目
  if (await evalJs(`!!document.querySelector(".dock-more")`)) {
    await evalJs(`document.querySelector(".dock-more").click()`);
    await sleep(300);
    const ov = await evalJs(`(() => {
      const rows = [...document.querySelectorAll(".dock-overflow-row")];
      return JSON.stringify({
        overflowOpen: !!document.querySelector(".dock-overflow"),
        rowCount: rows.length,
        names: rows.map(r => r.querySelector(".dock-overflow-name")?.textContent),
        winH: window.innerHeight,
      });
    })()`);
    console.log("OVERFLOW:", ov);
    await evalJs(`document.querySelector(".dock-more").click()`); // 收起浮层
  }
  cdp.close();
}
process.exit(0);
