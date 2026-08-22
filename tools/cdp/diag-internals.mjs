// 查看 __TAURI_INTERNALS__ 结构,找 IPC 观测点
const t = await (await fetch("http://127.0.0.1:9222/json")).json();
const ws = new WebSocket(t.find((p) => p.type === "page").webSocketDebuggerUrl);
ws.onopen = () => {
  ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: `JSON.stringify({
    keys: Object.keys(window.__TAURI_INTERNALS__),
    pmType: typeof window.__TAURI_INTERNALS__.postMessage,
    cbType: typeof window.__TAURI_INTERNALS__.transformCallback,
    pmWritable: Object.getOwnPropertyDescriptor(window.__TAURI_INTERNALS__, 'postMessage')?.writable,
    cbWritable: Object.getOwnPropertyDescriptor(window.__TAURI_INTERNALS__, 'transformCallback')?.writable
  })`, returnByValue: true } }));
};
ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id === 1) { console.log(m.result.result.value); process.exit(0); } };
