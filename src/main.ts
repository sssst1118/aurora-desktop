import { createApp } from "vue";
import { createPinia } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import App from "./App.vue";
import Island from "./components/core/Island.vue";
import SearchBar from "./components/core/SearchBar.vue";
import { apply_theme } from "./theme";
import "./style.css";
import "./styles/global.css";

// 双窗口共用同一份前端代码,按窗口 label 分流挂载不同根组件
const label = getCurrentWindow().label;
const root =
  label === "island" ? Island : label === "search" ? SearchBar : App;

// Phase4 4.4 主题:启动时按持久化配置应用深浅色 + 强调色(mount 前应用,避免首帧错主题);
// config_load 失败(后端未就绪)时用默认 system/blue 兜底
async function bootstrap() {
  try {
    const cfg = await invoke<{ theme_mode: string; theme_accent: string }>("config_load");
    apply_theme(cfg);
  } catch (e) {
    console.error("config_load failed, fallback to default theme", e);
    apply_theme({ theme_mode: "system", theme_accent: "blue" });
  }
  createApp(root).use(createPinia()).mount("#app");
}

void bootstrap();
