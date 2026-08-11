import { createApp } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App.vue";
import Island from "./components/core/Island.vue";
import SearchBar from "./components/core/SearchBar.vue";
import "./style.css";

// 双窗口共用同一份前端代码,按窗口 label 分流挂载不同根组件
const label = getCurrentWindow().label;
const root =
  label === "island" ? Island : label === "search" ? SearchBar : App;

createApp(root).mount("#app");
