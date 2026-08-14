// CDP 验证第二轮修复:关闭按钮/Settings 预热挂载/阴影移除/面板光晕
const ts = await (await fetch("http://127.0.0.1:9222/json")).json();
const pages = ts.filter((t) => t.type === "page");
const connect = (u) =>
  new Promise((res, rej) => {
    const w = new WebSocket(u);
    let id = 0;
    const p = new Map();
    w.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.id && p.has(m.id)) {
        p.get(m.id)(m);
        p.delete(m.id);
      }
    };
    w.onopen = () =>
      res({
        send: (me, pa) =>
          new Promise((r) => {
            const i = ++id;
            p.set(i, r);
            w.send(JSON.stringify({ id: i, method: me, params: pa }));
          }),
      });
    w.onerror = rej;
  });
for (const pg of pages) {
  const c = await connect(pg.webSocketDebuggerUrl);
  const r = await c.send("Runtime.evaluate", {
    expression: `JSON.stringify({
      island: !!document.querySelector(".island"),
      panel: !!document.querySelector(".main-head"),
      closeBtn: !!document.querySelector(".close-btn"),
      settingsMounted: document.querySelectorAll("section, [class*='s-row'], [class*='settings']").length > 0,
      settingsVisibleText: (document.body.innerText || "").includes("皮肤包") ? "settings-rendered" : "settings-hidden",
      islandShadow: document.querySelector(".island") ? getComputedStyle(document.querySelector(".island")).boxShadow : null,
      panelShadow: document.querySelector(".aurora-panel") ? getComputedStyle(document.querySelector(".aurora-panel")).boxShadow : null,
      panelHasRadial: document.querySelector(".aurora-panel") ? getComputedStyle(document.querySelector(".aurora-panel")).backgroundImage.includes("radial") : null,
    })`,
    returnByValue: true,
  });
  const v = r.result?.result?.value;
  if (v) console.log(v);
}
process.exit(0);
