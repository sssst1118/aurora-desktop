// CDP 实测:双击岛 → 主面板位置是否在岛正下方(问题 7 修复验证)
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

const conns = {};
for (const page of pages) {
  const cdp = await connect(page.webSocketDebuggerUrl);
  const evalJs = async (expr) => {
    const r = await cdp.send("Runtime.evaluate", { expression: expr, returnByValue: true });
    return r.result?.result?.value;
  };
  const kind = await evalJs(
    `document.querySelector(".island") ? "island" : document.querySelector(".main-head") ? "panel" : "other"`,
  );
  conns[kind] = { cdp, evalJs };
}

// 岛位置(窗口左上 + 尺寸)
const islandGeo = await conns.island.evalJs(`JSON.stringify({
  screenX: window.screenX, screenY: window.screenY,
  w: window.innerWidth, h: window.innerHeight,
  dpr: window.devicePixelRatio
})`);
console.log("ISLAND_GEO:", islandGeo);

// 双击岛(两次 pointerdown 间隔 100ms)
await conns.island.evalJs(
  `(() => { const el = document.querySelector(".island"); el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 8, clientY: 23 })); setTimeout(() => el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 8, clientY: 23 })), 100); })()`,
);
await sleep(1200); // 等呼出+定位

const panelState = await conns.panel.evalJs(`JSON.stringify({
  screenX: window.screenX, screenY: window.screenY,
  w: window.innerWidth, h: window.innerHeight,
  dpr: window.devicePixelRatio,
  bodyText: document.body.innerText.slice(0, 80)
})`);
console.log("PANEL_STATE:", panelState);

const g = JSON.parse(islandGeo);
const p = JSON.parse(panelState);
// 期望:面板水平中心 ≈ 岛中心;面板顶部 ≈ 岛底 + 12(逻辑像素,均乘 dpr 换算物理)
const islandCx = g.screenX + (g.w * g.dpr) / 2;
const panelCx = p.screenX + (p.w * p.dpr) / 2;
const expectTop = g.screenY + g.h * g.dpr + 12 * g.dpr;
console.log(
  `CHECK: 水平偏差=${Math.round(panelCx - islandCx)}px, 面板顶=${Math.round(p.screenY)}, 期望顶=${Math.round(expectTop)}, 偏差=${Math.round(p.screenY - expectTop)}px`,
);
process.exit(0);
