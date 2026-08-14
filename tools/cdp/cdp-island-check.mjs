// CDP 实测:岛收起态隐藏验证 + 模拟单击展开 + 展开态瓦片可见性验证
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
  // 1. 收起态:mini-dock 应隐藏(opacity 0)
  const collapsed = await evalJs(`JSON.stringify({
    w: window.innerWidth,
    expandedClass: document.querySelector(".island").classList.contains("expanded"),
    dockOpacity: getComputedStyle(document.querySelector(".mini-dock")).opacity,
    dockPointer: getComputedStyle(document.querySelector(".mini-dock")).pointerEvents,
    searchEntryOpacity: getComputedStyle(document.querySelector(".search-entry")).opacity,
    dividerOpacity: getComputedStyle(document.querySelector(".divider")).opacity,
  })`);
  console.log("COLLAPSED:", collapsed);
  // 2. 模拟单击(pointerdown 在岛空白处)→ 240ms 后应展开
  await evalJs(
    `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 8, clientY: 23 }))`,
  );
  await sleep(400); // 单击判定 240ms
  await sleep(400); // 展开动画 280ms
  const expanded = await evalJs(`JSON.stringify({
    w: window.innerWidth,
    expandedClass: document.querySelector(".island").classList.contains("expanded"),
    dockOpacity: getComputedStyle(document.querySelector(".mini-dock")).opacity,
    dockRect: (() => { const r = document.querySelector(".mini-dock").getBoundingClientRect(); return { x: Math.round(r.x), w: Math.round(r.width) }; })(),
    tileCount: document.querySelectorAll(".mini-dock .dock-tile").length,
    tilesVisible: [...document.querySelectorAll(".mini-dock .dock-tile")].filter(t => { const r = t.getBoundingClientRect(); return r.left >= 0 && r.right <= window.innerWidth && r.width > 0; }).length,
    tileTitles: [...document.querySelectorAll(".mini-dock .dock-tile")].map(t => t.getAttribute("title")),
  })`);
  console.log("EXPANDED:", expanded);
  cdp.close();
}
process.exit(0);
