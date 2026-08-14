import { createApp } from "vue";
import { createPinia } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import App from "./App.vue";
import Island from "./components/core/Island.vue";
import MainPanel from "./components/core/MainPanel.vue";
import { apply_theme, apply_panel_style } from "./theme";
import "./style.css";
import "./styles/global.css";

// 多窗口共用同一份前端代码,按窗口 label 分流挂载不同根组件(Phase6 一岛一窗:
// island=灵动岛 / search=主面板五视图合一,其余 label 落到 App 兜底)
const label = getCurrentWindow().label;
const root =
  label === "island" ? Island : label === "search" ? MainPanel : App;

// Phase4 4.4 主题:启动时按持久化配置应用深浅色 + 强调色(mount 前应用,避免首帧错主题);
// config_load 失败(后端未就绪)时用默认 system/blue 兜底
async function bootstrap() {
  try {
    const cfg = await invoke<{
      theme_mode: string;
      theme_accent: string;
      skin: string;
      search_style?: string;
    }>("config_load");
    apply_theme(cfg);
    // 显示方式(2026-08-14 真机反馈恢复):默认不透明,玻璃为可选
    apply_panel_style(cfg.search_style ?? "solid");
  } catch (e) {
    console.error("config_load failed, fallback to default theme", e);
    apply_theme({ theme_mode: "system", theme_accent: "blue" });
    apply_panel_style("solid");
  }
  createApp(root).use(createPinia()).mount("#app");
}

void bootstrap();
