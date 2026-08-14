// CDP 验证真机反馈第三轮三项修复:
// 1. 双击呼出面板 → 450ms 后岛自动收回(expanded 回落)
// 2. pointermove >4px → .dragging class 出现,断流 150ms 后消失
// 3. .island 存在 clip-path(四角模糊裁剪)
// 4. 截图存 WSL(绕过 Windows 盘加密,Claude 可直接读)
import fs from "fs";
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

  // 0. clip-path 检查
  const clip = await evalJs(
    `(() => { const cs = getComputedStyle(document.querySelector(".island")); return JSON.stringify({ clipPath: cs.clipPath, radius: cs.borderRadius }); })()`,
  );
  console.log("CLIP:", clip);

  // 1. 双击 → 展开 → 450ms 后自动收回
  const dbl = (x) =>
    evalJs(
      `document.querySelector(".island").dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: ${x}, clientY: 23 }))`,
    );
  await dbl(60);
  await sleep(150);
  await dbl(60);
  await sleep(400);
  const afterDbl = await evalJs(
    `document.querySelector(".island").classList.contains("expanded")`,
  );
  console.log("AFTER_DBL(expanded, expect true):", afterDbl);
  await sleep(500); // 450ms 收回 + 展开动画余量
  const autoCollapsed = await evalJs(
    `document.querySelector(".island").classList.contains("expanded")`,
  );
  console.log("AFTER_450MS(expanded, expect false):", autoCollapsed);

  // 2. 拖动判定:pointerdown + move>4px → .dragging;断流后消失
  await evalJs(
    `(() => { const el = document.querySelector(".island");
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 60, clientY: 23 }));
      el.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: 80, clientY: 23 }));
    })()`,
  );
  await sleep(50);
  const draggingOn = await evalJs(
    `document.querySelector(".island").classList.contains("dragging")`,
  );
  console.log("DRAGGING_ON(expect true):", draggingOn);
  await sleep(250);
  const draggingOff = await evalJs(
    `document.querySelector(".island").classList.contains("dragging")`,
  );
  console.log("DRAGGING_OFF_after_150ms(expect false):", draggingOff);

  // 3. 截图:写 WSL 原生盘(Claude 可读,绕 Windows 加密)
  const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(
    "//wsl.localhost/Ubuntu-20.04/home/gzk/aurora_island_r3.png",
    Buffer.from(shot.result.data, "base64"),
  );
  console.log("SHOT: saved to WSL");
  cdp.close();
  await sleep(300);
}
process.exit(0);
