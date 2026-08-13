<script setup lang="ts">
import { onActivated, onDeactivated, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useConfigStore } from "../../stores/config";
import { apply_theme } from "../../theme";
import ToggleSwitch from "../ToggleSwitch.vue";
import WallpaperPanel from "./WallpaperPanel.vue";

const store = useConfigStore();
const emit = defineEmits<{ (e: "close"): void }>();

/** 配置保存统一入口:失败时回滚本地值为后端实际配置并展示红字提示;成功清空提示 */
const saveError = ref("");

// ---- 配置加载状态(store.cfg 为 null 时模板展示加载中/失败占位,避免整页空白) ----
/** 初始视为加载中(首次激活必触发 loadCfg),避免错误占位闪现 */
const cfgLoading = ref(true);
const cfgError = ref("");

/** 配置拉取入口(激活刷新与失败重试共用;失败保留错误信息供占位展示) */
async function loadCfg() {
  cfgLoading.value = true;
  cfgError.value = "";
  try {
    await store.load();
  } catch (e) {
    cfgError.value = `配置加载失败:${e}`;
  } finally {
    cfgLoading.value = false;
  }
}

async function saveSafe(): Promise<boolean> {
  if (!store.cfg) return false;
  try {
    await store.save();
    saveError.value = "";
    return true;
  } catch (e) {
    saveError.value = `保存失败:${e}`;
    try {
      await store.load(); // 回滚:以后端实际落盘配置为准,丢弃失败的本地修改
    } catch {
      /* 后端读取也失败时保留本地值,用户可重试 */
    }
    return false;
  }
}

// 设置页被 SearchBar 用 KeepAlive 缓存(2026-08-13 状态保持改造):
// onMounted 只跑一次(事件订阅/版本号),配置与显示器数据拉取移到 onActivated,
// 每次打开设置页都刷新,保证"重新打开即最新",与旧版每次重建重新拉数据行为一致
onMounted(async () => {
  try {
    appVersion.value = await getVersion();
  } catch {
    appVersion.value = "";
  }
  // 订阅后台检查/托盘检查结果与下载事件(后端驱动,前端只展示)
  const unlisteners = [
    await listen("update-available", (ev) => {
      const p = ev.payload as { version?: string | null; notes?: string | null };
      updateStatus.value = "available";
      updateVersion.value = p.version ?? "";
      updateNotes.value = p.notes ?? "";
    }),
    await listen("update-check-result", (ev) => {
      const p = ev.payload as {
        status: string;
        version?: string | null;
        notes?: string | null;
        error?: string | null;
      };
      if (p.status === "available") {
        updateStatus.value = "available";
        updateVersion.value = p.version ?? "";
        updateNotes.value = p.notes ?? "";
      } else if (p.status === "latest") {
        updateStatus.value = "latest";
        updateVersion.value = p.version ?? "";
      } else {
        updateStatus.value = "error";
        updateError.value = p.error ?? "检查更新失败";
      }
    }),
    await listen("update-downloaded", () => {
      updateStatus.value = "downloaded";
    }),
    await listen("update-error", (ev) => {
      updateStatus.value = "error";
      updateError.value = String((ev.payload as { message?: string }).message ?? "下载失败");
    }),
  ];
  updateListeners = unlisteners;
});

onActivated(() => {
  void loadCfg();
  void loadMultiMonitorState(); // 显示器信息 + 素材列表 + 每屏当前素材(多屏关时也拉素材列表)
});

// 离开设置页(KeepAlive 停用/销毁):结束热键录制,防止残留窗口级监听
onDeactivated(() => {
  stopRecord();
});

onUnmounted(() => {
  stopRecord();
  updateListeners.forEach((u) => u());
  updateListeners = [];
});

async function toggleIsland() {
  if (!store.cfg) return;
  store.cfg.enable_island = !store.cfg.enable_island;
  await saveSafe();
}

/** Phase2 模块开关(热生效:config_save 后 runtime::apply 同步 watcher/监听器/窗口显隐) */
async function toggleModule(
  key: "enable_dock" | "enable_file_drawer" | "enable_clipboard_history",
) {
  if (!store.cfg) return;
  store.cfg[key] = !store.cfg[key];
  await saveSafe();
}

/** 搜索框显示方式(热生效:保存后 SearchBar 经 aurora:config-saved 即时刷新,无需重启) */
async function setSearchStyle(style: "glass" | "solid") {
  if (!store.cfg) return;
  store.cfg.search_style = style;
  await saveSafe();
}

/** Phase3 AI 总开关(热生效:热键注册走 apply_hotkeys diff,关闭即注销) */
async function toggleAiEnable() {
  if (!store.cfg) return;
  store.cfg.enable_ai = !store.cfg.enable_ai;
  await saveSafe();
}

/** Phase3 服务商切换(deepseek / ollama) */
async function setProvider(p: "deepseek" | "ollama") {
  if (!store.cfg) return;
  store.cfg.ai_provider = p;
  await saveSafe();
}

/** Phase3 文本输入保存(失焦/回车时提交,避免逐键保存) */
async function saveText() {
  await saveSafe();
}

// ---- 热键录制输入(2026-08-13):裸文本框改点击录制,生成与后端解析一致的小写格式) ----
// 后端 hotkey.rs 用 Shortcut::from_str(tauri-plugin-global-shortcut)解析,规范格式为
// "ctrl+alt+d" 全小写加号分隔(与 config.rs 默认值一致);三个热键输入共用一套录制逻辑,
// 一次只录一个(recordingKey 记录当前录制项)
const recordingKey = ref<string | null>(null);

/** 录制项 → 配置字段映射 */
const HOTKEY_FIELDS: Record<string, "drawer_hotkey" | "hotkey_clipboard" | "ai_hotkey"> = {
  drawer: "drawer_hotkey",
  clipboard: "hotkey_clipboard",
  ai: "ai_hotkey",
};

/** 输入框显示文本:录制中显示提示,否则显示当前配置值 */
function hotkeyText(key: string): string {
  const cfg = store.cfg;
  if (!cfg) return "";
  return recordingKey.value === key ? "按下组合键…" : cfg[HOTKEY_FIELDS[key]];
}

/** 修饰键判定(纯修饰键按下不生成组合,继续等主键) */
function isModifierKey(key: string): boolean {
  return key === "Control" || key === "Alt" || key === "Shift" || key === "Meta";
}

/** 主键名规范化成后端可解析 token(global-hotkey Code 格式);不支持返回 null(忽略继续) */
function normalizeHotkeyKey(key: string): string | null {
  if (key.length === 1) {
    const c = key.toLowerCase();
    return /^[a-z0-9]$/.test(c) ? c : null;
  }
  const special: Record<string, string> = {
    Space: "space",
    Enter: "enter",
    Tab: "tab",
    Backspace: "backspace",
    Delete: "delete",
    Insert: "insert",
    Home: "home",
    End: "end",
    PageUp: "pageup",
    PageDown: "pagedown",
    ArrowUp: "up",
    ArrowDown: "down",
    ArrowLeft: "left",
    ArrowRight: "right",
  };
  if (special[key]) return special[key];
  const m = /^F([1-9]|1[0-9]|2[0-4])$/.exec(key);
  return m ? "f" + m[1].toLowerCase() : null;
}

/** 进入录制:窗口捕获阶段接管后续按键(capture 先于 SearchBar 冒泡阶段 handler) */
function startRecord(key: string) {
  if (!store.cfg) return;
  recordingKey.value = key;
  window.removeEventListener("keydown", onRecordKeydown, true); // 防重复注册
  window.addEventListener("keydown", onRecordKeydown, true);
}

/** 结束录制(未写入任何值,Esc/失焦取消时显示自动恢复原配置值) */
function stopRecord() {
  if (recordingKey.value === null) return;
  recordingKey.value = null;
  window.removeEventListener("keydown", onRecordKeydown, true);
}

/**
 * 录制中按键:Esc 取消;纯修饰键/无修饰键/不支持主键一律忽略继续;
 * 修饰键 + 主键 → 生成 "ctrl+alt+d" 格式写入配置并自动保存(复用 saveText 入口)。
 * stopPropagation 防 SearchBar 窗口级 handler 收到(否则 Esc 会直接关掉设置页)
 */
function onRecordKeydown(e: KeyboardEvent) {
  const key = recordingKey.value;
  const cfg = store.cfg;
  if (!key || !cfg) return;
  e.stopPropagation();
  e.preventDefault();
  if (e.key === "Escape") {
    stopRecord();
    return;
  }
  if (isModifierKey(e.key)) return;
  if (!(e.ctrlKey || e.altKey || e.shiftKey || e.metaKey)) return; // 必须含至少一个修饰键
  const main = normalizeHotkeyKey(e.key);
  if (!main) return;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  if (e.metaKey) parts.push("super");
  parts.push(main);
  cfg[HOTKEY_FIELDS[key]] = parts.join("+");
  stopRecord();
  void saveText(); // 录制完成自动保存(失败红字提示 + 回滚,走统一保存入口)
}

/** Phase3 工具调用总开关(热生效:命令内每次请求实时读配置) */
async function toggleAiTools() {
  if (!store.cfg) return;
  store.cfg.ai_tools_enabled = !store.cfg.ai_tools_enabled;
  await saveSafe();
}

/** Phase3 搜索目录集合:文本域每行一个目录(默认空 = 仅桌面,禁止全盘扫描) */
async function onSearchRootsChange(e: Event) {
  if (!store.cfg) return;
  const text = (e.target as HTMLTextAreaElement).value;
  store.cfg.ai_search_roots = text
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  await saveSafe();
}

/** Phase3 密钥清除:传空串让后端清空(Rust 侧 resolve_key_save 对非掩码值直接生效) */
async function clearApiKey() {
  if (!store.cfg) return;
  store.cfg.ai_api_key = "";
  await saveSafe();
}

// ---- Phase4 模块开关(全部热生效:壁纸撤下/电池线程/命令门控均实时响应配置)----

/** Phase4 4.1 动态壁纸总开关 */
async function toggleDynamicWallpaper() {
  if (!store.cfg) return;
  store.cfg.enable_dynamic_wallpaper = !store.cfg.enable_dynamic_wallpaper;
  await saveSafe();
}

/** Phase4 4.1 电池降载子开关(受总开关控制,关闭时不可用) */
async function toggleBatteryDownshift() {
  if (!store.cfg) return;
  store.cfg.wallpaper_battery_downshift = !store.cfg.wallpaper_battery_downshift;
  await saveSafe();
}

/** Phase4 4.2/4.3 自动化总开关 */
async function toggleAutomation() {
  if (!store.cfg) return;
  store.cfg.enable_automation = !store.cfg.enable_automation;
  await saveSafe();
}

/** Phase4 4.3 UIA 控件操作子开关(受自动化总开关控制,关闭时不可用) */
async function toggleUiaEnable() {
  if (!store.cfg) return;
  store.cfg.automation_uia_enable = !store.cfg.automation_uia_enable;
  await saveSafe();
}

// ---- Phase5 5.2 多屏壁纸(设计文档 §2.3;开关/模式即时生效 = 保存后调 multi_apply)----

/** 与 Rust 侧 MonitorInfo 对应(wallpaper_multi_monitors 返回) */
interface MonitorInfo {
  index: number;
  x: number;
  y: number;
  width: number;
  height: number;
  primary: boolean;
}

/** 动态壁纸素材列表元素(与 Rust 侧 WallpaperEntry 对应) */
interface WallpaperEntry {
  name: string;
  path: string;
  size: number;
}

const monitors = ref<MonitorInfo[]>([]);
const materials = ref<WallpaperEntry[]>([]);
/** 独立模式:每屏下拉选中项(屏 index → 素材路径;空串 = 无素材) */
const perMonitorSel = ref<Record<number, string>>({});
const multiError = ref("");

/** 枚举显示器 + 拉素材列表 + 按当前每屏素材初始化下拉(重复调用刷新) */
async function loadMultiMonitorState() {
  multiError.value = "";
  try {
    monitors.value = await invoke<MonitorInfo[]>("wallpaper_multi_monitors");
    materials.value = await invoke<WallpaperEntry[]>("wallpaper_dynamic_list");
    const st = await invoke<{
      monitors: { index: number; path: string | null }[];
    }>("wallpaper_dynamic_get_state");
    const sel: Record<number, string> = {};
    for (const m of st.monitors) sel[m.index] = m.path ?? "";
    perMonitorSel.value = sel;
  } catch (e) {
    multiError.value = `读取显示器信息失败:${e}`;
  }
}

/** 多屏开关(保存后重建各屏 attach,立即生效) */
async function toggleMultiMonitor() {
  if (!store.cfg) return;
  store.cfg.wallpaper_multi_monitor = !store.cfg.wallpaper_multi_monitor;
  if (!(await saveSafe())) return; // 保存失败已回滚,不再应用多屏
  try {
    await invoke("wallpaper_multi_apply");
    await loadMultiMonitorState();
  } catch (e) {
    multiError.value = `应用多屏壁纸失败:${e}`;
  }
}

/** 拼接/独立模式单选(保存后重建,立即生效) */
async function setSpanMode(span: boolean) {
  if (!store.cfg) return;
  store.cfg.wallpaper_span_mode = span;
  if (!(await saveSafe())) return; // 保存失败已回滚,不再重建
  try {
    await invoke("wallpaper_multi_apply");
    await loadMultiMonitorState();
  } catch (e) {
    multiError.value = `切换模式失败:${e}`;
  }
}

/** 独立模式:给指定屏设素材(未选素材不提交;清除某屏素材 = 用"恢复系统壁纸"全屏清) */
async function applyMonitorMaterial(index: number) {
  multiError.value = "";
  const path = perMonitorSel.value[index] ?? "";
  if (path === "") return;
  try {
    await invoke("wallpaper_dynamic_set_monitor", { path, index });
    await loadMultiMonitorState();
  } catch (e) {
    multiError.value = `设置屏 ${index + 1} 素材失败:${e}`;
  }
}

// ---- 动态壁纸素材选择(单屏/拼接模式共用;Phase4 遗留未接线的素材入口,Phase5 补齐)----

const materialSel = ref("");
const materialError = ref("");
const materialNotice = ref("");

/** 应用选中的动态壁纸素材(video 走 WorkerW;图片走系统壁纸;拼接模式自动铺满全屏) */
async function applyMaterial() {
  materialError.value = "";
  materialNotice.value = "";
  const path = materialSel.value;
  if (!path) return;
  try {
    await invoke("wallpaper_dynamic_set", { path });
    materialNotice.value = "壁纸已应用";
    await loadMultiMonitorState();
  } catch (e) {
    materialError.value = String(e);
  }
}

/** 恢复系统壁纸(撤下全部动态壁纸注入;多屏开时一并清全部屏) */
async function clearMaterial() {
  materialError.value = "";
  materialNotice.value = "";
  try {
    await invoke("wallpaper_dynamic_clear");
    materialNotice.value = "已恢复系统壁纸";
    materialSel.value = "";
    await loadMultiMonitorState();
  } catch (e) {
    materialError.value = String(e);
  }
}

/** Phase4 4.4 主题三态切换(system/dark/light;保存成功后经 theme.ts 立即应用) */
async function setThemeMode(mode: "system" | "dark" | "light") {
  if (!store.cfg) return;
  store.cfg.theme_mode = mode;
  await saveSafe();
  // 成功即应用新值;失败时 saveSafe 已回滚 cfg,此处按回滚值重放,保证界面与落盘一致
  apply_theme({ theme_mode: store.cfg.theme_mode, theme_accent: store.cfg.theme_accent });
}

/** Phase4 4.4 强调色选择(存 token 名,不存色值;保存成功后立即应用) */
async function setAccent(name: string) {
  if (!store.cfg) return;
  store.cfg.theme_accent = name;
  await saveSafe();
  apply_theme({ theme_mode: store.cfg.theme_mode, theme_accent: store.cfg.theme_accent });
}

// ---- Phase5 5.1 自动更新(自研 updater;命令契约见 docs/Phase5-设计.md §1)----

const appVersion = ref("");
const updateStatus = ref("idle"); // idle | checking | latest | available | downloading | downloaded | error
const updateVersion = ref("");
const updateNotes = ref("");
const updateError = ref("");
let updateListeners: UnlistenFn[] = [];

/** 手动检查更新(update_check 三态:latest/available/error) */
async function checkUpdate() {
  updateStatus.value = "checking";
  updateError.value = "";
  try {
    const r = await invoke<{
      status: string;
      version: string | null;
      notes: string | null;
      error: string | null;
    }>("update_check");
    if (r.status === "available") {
      updateStatus.value = "available";
      updateVersion.value = r.version ?? "";
      updateNotes.value = r.notes ?? "";
    } else if (r.status === "latest") {
      updateStatus.value = "latest";
      updateVersion.value = r.version ?? "";
    } else {
      updateStatus.value = "error";
      updateError.value = r.error ?? "检查更新失败";
    }
  } catch (e) {
    updateStatus.value = "error";
    updateError.value = String(e);
  }
}

/** 下载并安装:update_download(进度经事件)成功后 update_install(静默安装 + 退出重启) */
async function downloadAndInstall() {
  updateStatus.value = "downloading";
  updateError.value = "";
  try {
    await invoke("update_download");
    // 已下载完成,交给安装器(命令内部 spawn 独立进程并退出 app)
    await invoke("update_install");
  } catch (e) {
    updateStatus.value = "error";
    updateError.value = String(e);
  }
}

/** 打开下载目录(手动安装兜底) */
async function openUpdatesFolder() {
  try {
    await invoke("update_open_folder");
  } catch (e) {
    updateError.value = String(e);
  }
}

/** 开关组件标记(纯展示辅助,避免模板重复) */
const on = (v: boolean | undefined) => v === true;

// ---- Phase4 4.2 自动化测试区(坐标点击/文本输入;错误红字展示,后端不崩)----
const simX = ref("");
const simY = ref("");
const simText = ref("");
const simError = ref("");

async function simClick() {
  simError.value = "";
  try {
    await invoke("automation_sim_click", { x: Number(simX.value), y: Number(simY.value) });
  } catch (e) {
    simError.value = String(e);
  }
}

async function simType() {
  simError.value = "";
  try {
    await invoke("automation_sim_type", { text: simText.value });
  } catch (e) {
    simError.value = String(e);
  }
}

// ---- Phase4 4.3 UIA 测试区(窗口搜索/控件列表/读文本/点击/输入;错误红字展示)----
interface UiaWindowItem {
  hwnd: number;
  title: string;
  class: string;
  pid: number;
  visible: boolean;
}
interface UiaControlItem {
  id: string;
  name: string;
  control_type: string;
  bounds: [number, number, number, number];
}
const uiaWinTitle = ref("");
const uiaWindows = ref<UiaWindowItem[]>([]);
const uiaSelHwnd = ref<number | null>(null);
const uiaControls = ref<UiaControlItem[]>([]);
const uiaSelId = ref("");
const uiaText = ref("");
const uiaTypeText = ref("");
const uiaError = ref("");

async function uiaSearchWindows() {
  uiaError.value = "";
  try {
    uiaWindows.value = await invoke<UiaWindowItem[]>("uia_find_window", {
      title: uiaWinTitle.value,
    });
    uiaControls.value = [];
    uiaSelHwnd.value = null;
  } catch (e) {
    uiaError.value = String(e);
  }
}

async function uiaListControls(hwnd: number) {
  uiaError.value = "";
  try {
    uiaSelHwnd.value = hwnd;
    uiaControls.value = await invoke<UiaControlItem[]>("uia_find_controls", {
      hwnd,
      controlType: null,
      name: null,
    });
    uiaSelId.value = "";
  } catch (e) {
    uiaError.value = String(e);
  }
}

async function uiaReadText() {
  uiaError.value = "";
  if (uiaSelHwnd.value === null || !uiaSelId.value) return;
  try {
    uiaText.value = await invoke<string>("uia_get_control_text", {
      hwnd: uiaSelHwnd.value,
      controlId: uiaSelId.value,
    });
  } catch (e) {
    uiaError.value = String(e);
  }
}

async function uiaClick() {
  uiaError.value = "";
  if (uiaSelHwnd.value === null || !uiaSelId.value) return;
  try {
    await invoke("uia_click_control", {
      hwnd: uiaSelHwnd.value,
      controlId: uiaSelId.value,
    });
  } catch (e) {
    uiaError.value = String(e);
  }
}

async function uiaType() {
  uiaError.value = "";
  if (uiaSelHwnd.value === null || !uiaSelId.value) return;
  try {
    await invoke("uia_type_into", {
      hwnd: uiaSelHwnd.value,
      controlId: uiaSelId.value,
      text: uiaTypeText.value,
    });
  } catch (e) {
    uiaError.value = String(e);
  }
}
</script>

<template>
  <div
    class="h-full w-full flex flex-col rounded-xl overflow-hidden text-[var(--aurora-text)]"
    :class="store.cfg?.search_style === 'solid' ? 'bg-[var(--aurora-panel-solid)]' : 'bg-[var(--aurora-panel)] backdrop-blur-xl'"
  >
    <div class="flex items-center justify-between px-4 py-3 border-b border-[var(--aurora-border)]">
      <span class="text-sm">设置</span>
      <button class="text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] text-sm" title="关闭" aria-label="关闭设置" @click="emit('close')">
        ✕
      </button>
    </div>
    <!-- 内容区禁拖(设置项滚动/开关点击放行;标题栏留可拖,设置页也能拖窗口) -->
    <div class="flex-1 overflow-y-auto px-4 py-3 space-y-4" data-tauri-drag-region="false">
      <!-- 配置未加载占位:加载中提示 / 失败红字 + 重试(cfg 为空时其余区块一律不渲染,避免整页空白) -->
      <div v-if="!store.cfg" class="py-10 flex flex-col items-center gap-2">
        <div v-if="cfgLoading" class="text-xs text-[var(--aurora-text-dim)]">配置加载中…</div>
        <template v-else>
          <div
            class="text-xs text-[var(--aurora-danger)] bg-[var(--aurora-danger-bg)] rounded-lg px-3 py-1.5"
          >
            {{ cfgError || "配置加载失败" }}
          </div>
          <button
            class="text-xs px-3 py-1.5 rounded-lg bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)] text-white transition-colors"
            aria-label="重试加载配置"
            @click="loadCfg"
          >
            重试
          </button>
        </template>
      </div>
      <template v-else>
      <!-- 保存失败提示(saveSafe 捕获所有开关/设置的保存失败,回滚后红字展示) -->
      <div
        v-if="saveError"
        class="text-xs text-[var(--aurora-danger)] bg-[var(--aurora-danger-bg)] rounded-lg px-3 py-1.5"
      >
        {{ saveError }}
      </div>
      <!-- 快捷键速查:搜索/抽屉/剪贴板三个快捷键一屏可见(抽屉/剪贴板键位可在各自区块修改) -->
      <div class="bg-[var(--aurora-field)] rounded p-2.5 space-y-1.5">
        <div class="text-xs text-[var(--aurora-text-dim)]">快捷键速查</div>
        <div class="flex items-center justify-between text-xs">
          <!-- 灵动岛即顶部常驻栏:点击或按快捷键均呼出搜索框;键位为固定值(与 hotkey.rs SEARCH_HOTKEY 一致) -->
          <span>灵动岛 · 呼出搜索框</span>
          <span
            class="font-mono bg-[var(--aurora-panel)] rounded px-2 py-0.5 border border-[var(--aurora-border)]"
            >Ctrl+Shift+Space</span
          >
        </div>
        <div class="flex items-center justify-between text-xs">
          <span>文件抽屉</span>
          <span
            class="font-mono bg-[var(--aurora-panel)] rounded px-2 py-0.5 border border-[var(--aurora-border)]"
            >{{ store.cfg?.drawer_hotkey ?? "Ctrl+Alt+D" }}</span
          >
        </div>
        <div class="flex items-center justify-between text-xs">
          <span>剪贴板历史</span>
          <span
            class="font-mono bg-[var(--aurora-panel)] rounded px-2 py-0.5 border border-[var(--aurora-border)]"
            >{{ store.cfg?.hotkey_clipboard ?? "Ctrl+Alt+V" }}</span
          >
        </div>
        <div class="flex items-center justify-between text-xs">
          <!-- 全部显示/隐藏:与托盘菜单同语义,共用快照(固定值,见 hotkey.rs ALL_HOTKEY) -->
          <span>全部显示/隐藏</span>
          <span
            class="font-mono bg-[var(--aurora-panel)] rounded px-2 py-0.5 border border-[var(--aurora-border)]"
            >Ctrl+Shift+H</span
          >
        </div>
        <p class="text-[10px] text-[var(--aurora-text-dim)]">灵动岛点击或快捷键均呼出搜索框;全部显示/隐藏为固定值;抽屉/剪贴板可在各自区块修改</p>
      </div>

      <ToggleSwitch
        :model-value="on(store.cfg?.enable_island)"
        label="灵动岛"
        description="顶部常驻:时间 + CPU/内存/网络,立即生效"
        @update:model-value="toggleIsland"
      />

      <!-- Dock 栏:并入搜索窗口底部,拖拽添加 -->
      <ToggleSwitch
        :model-value="on(store.cfg?.enable_dock)"
        label="Dock 栏"
        description="搜索窗口底部快捷栏,拖拽 exe/lnk 添加,立即生效"
        @update:model-value="toggleModule('enable_dock')"
      />

      <!-- 搜索框:显示方式可选(毛玻璃/不透明);几何(位置/大小)自动记忆,无需设置项 -->
      <div v-if="store.cfg" class="flex flex-col gap-1.5">
        <div>
          <div class="text-sm">搜索框</div>
          <div class="text-[10px] text-[var(--aurora-text-dim)]">
            拖左上角手柄移动、拖右下角调整大小,位置与大小自动记忆;显示方式保存后即时生效
          </div>
        </div>
        <div class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">显示方式</span>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.search_style !== 'solid' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            aria-label="搜索框显示方式:毛玻璃"
            @click="setSearchStyle('glass')"
          >
            毛玻璃
          </button>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.search_style === 'solid' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            aria-label="搜索框显示方式:经典不透明"
            @click="setSearchStyle('solid')"
          >
            经典不透明
          </button>
        </div>
      </div>

      <!-- 文件抽屉:热键受模块开关门控,关闭时仅展示不生效 -->
      <div class="flex flex-col gap-1.5">
        <ToggleSwitch
          :model-value="on(store.cfg?.enable_file_drawer)"
          label="文件抽屉"
          description="立即生效"
          @update:model-value="toggleModule('enable_file_drawer')"
        />
        <div v-if="store.cfg" class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">抽屉热键</span>
          <input
            :value="hotkeyText('drawer')"
            readonly
            placeholder="点击后按下组合键"
            aria-label="抽屉热键"
            class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 font-mono cursor-pointer"
            :class="recordingKey === 'drawer' ? 'ring-1 ring-[var(--aurora-accent)] text-[var(--aurora-text-dim)]' : ''"
            :title="recordingKey === 'drawer' ? '按下 Esc 取消录制' : '点击进入录制模式'"
            @click="startRecord('drawer')"
            @blur="stopRecord"
          />
          <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">立即生效</span>
        </div>
        <p v-if="store.cfg && !store.cfg.enable_file_drawer" class="text-[10px] text-[var(--aurora-text-dim)] ml-[70px]">
          模块关闭时不生效
        </p>
      </div>

      <!-- 剪贴板历史:热键受模块开关门控,关闭时仅展示不生效 -->
      <div class="flex flex-col gap-1.5">
        <ToggleSwitch
          :model-value="on(store.cfg?.enable_clipboard_history)"
          label="剪贴板历史"
          description="立即生效"
          @update:model-value="toggleModule('enable_clipboard_history')"
        />
        <div v-if="store.cfg" class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">剪贴板热键</span>
          <input
            :value="hotkeyText('clipboard')"
            readonly
            placeholder="点击后按下组合键"
            aria-label="剪贴板热键"
            class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 font-mono cursor-pointer"
            :class="recordingKey === 'clipboard' ? 'ring-1 ring-[var(--aurora-accent)] text-[var(--aurora-text-dim)]' : ''"
            :title="recordingKey === 'clipboard' ? '按下 Esc 取消录制' : '点击进入录制模式'"
            @click="startRecord('clipboard')"
            @blur="stopRecord"
          />
          <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">立即生效</span>
        </div>
        <p v-if="store.cfg && !store.cfg.enable_clipboard_history" class="text-[10px] text-[var(--aurora-text-dim)] ml-[70px]">
          模块关闭时不生效
        </p>
      </div>

      <!-- 2.4 静态壁纸区块 -->
      <div>
        <div class="text-sm mb-1.5">壁纸</div>
        <WallpaperPanel />
      </div>

      <!-- Phase3 AI 设置区块(密钥脱敏契约:config_load 返回掩码,前端永不见明文) -->
      <div v-if="store.cfg" class="border-t border-[var(--aurora-border)] pt-3 space-y-3">
        <ToggleSwitch
          :model-value="on(store.cfg.enable_ai)"
          label="AI 助手"
          description="总开关,关闭时热键立即注销"
          @update:model-value="toggleAiEnable"
        />

        <div v-if="store.cfg.enable_ai" class="space-y-2.5">
          <!-- 服务商切换 -->
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">服务商</span>
            <button
              class="text-xs px-2 py-0.5 rounded transition-colors"
              :class="store.cfg.ai_provider === 'deepseek' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
              aria-label="选择服务商 DeepSeek"
              @click="setProvider('deepseek')"
            >
              DeepSeek
            </button>
            <button
              class="text-xs px-2 py-0.5 rounded transition-colors"
              :class="store.cfg.ai_provider === 'ollama' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
              aria-label="选择服务商 Ollama"
              @click="setProvider('ollama')"
            >
              Ollama
            </button>
          </div>

          <!-- DeepSeek:密钥(password 型,已配置显示掩码)+ 模型 + 接口地址 -->
          <template v-if="store.cfg.ai_provider === 'deepseek'">
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">API Key</span>
              <input
                v-model="store.cfg.ai_api_key"
                type="password"
                class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)] font-mono"
                placeholder="未配置"
                @change="saveText"
              />
              <button
                class="text-xs px-1.5 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
                title="清空密钥"
                aria-label="清空 API Key"
                @click="clearApiKey"
              >
                清除
              </button>
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">模型</span>
              <input
                v-model="store.cfg.ai_model"
                class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
                @change="saveText"
              />
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">接口地址</span>
              <input
                v-model="store.cfg.ai_base_url"
                class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
                @change="saveText"
              />
            </div>
          </template>

          <!-- Ollama:本地地址 + 模型 -->
          <template v-else>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">Ollama 地址</span>
              <input
                v-model="store.cfg.ai_ollama_url"
                class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
                @change="saveText"
              />
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">Ollama 模型</span>
              <input
                v-model="store.cfg.ai_ollama_model"
                class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
                @change="saveText"
              />
            </div>
            <p class="text-[10px] text-[var(--aurora-text-dim)] ml-[70px]">
              按本机已装模型修改,如 qwen2.5:7b
            </p>
          </template>

          <!-- 工具调用总开关 -->
          <ToggleSwitch
            :model-value="on(store.cfg.ai_tools_enabled)"
            label="工具调用"
            description="让 AI 打开应用/搜文件/设壁纸等,关闭后纯对话"
            @update:model-value="toggleAiTools"
          />

          <!-- 搜索目录集合(每行一个,默认空 = 仅桌面) -->
          <div>
            <div class="text-xs text-[var(--aurora-text-dim)] mb-1">文件搜索目录</div>
            <textarea
              class="w-full text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)] font-mono resize-y leading-relaxed"
              rows="2"
              placeholder="每行一个目录(留空 = 仅桌面,禁止全盘扫描)"
              :value="store.cfg.ai_search_roots.join('\n')"
              @change="onSearchRootsChange"
            />
          </div>

          <!-- 工具循环上限 + AI 热键 -->
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">工具轮数上限</span>
            <input
              v-model.number="store.cfg.ai_max_tool_rounds"
              type="number"
              min="1"
              max="10"
              class="w-16 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
              @change="saveText"
            />
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">AI 热键</span>
            <input
              :value="hotkeyText('ai')"
              readonly
              placeholder="点击后按下组合键"
              aria-label="AI 热键"
              class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 font-mono cursor-pointer"
              :class="recordingKey === 'ai' ? 'ring-1 ring-[var(--aurora-accent)] text-[var(--aurora-text-dim)]' : ''"
              :title="recordingKey === 'ai' ? '按下 Esc 取消录制' : '点击进入录制模式'"
              @click="startRecord('ai')"
              @blur="stopRecord"
            />
            <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">立即生效</span>
          </div>
        </div>
      </div>

      <!-- Phase4 4.1 动态壁纸区块(素材选择/预览待 4.1 模块合入后启用) -->
      <div v-if="store.cfg" class="border-t border-[var(--aurora-border)] pt-3 space-y-2.5">
        <ToggleSwitch
          :model-value="on(store.cfg.enable_dynamic_wallpaper)"
          label="动态壁纸"
          description="WorkerW 壁纸层:本地视频壁纸,关闭开关即撤下,立即生效"
          @update:model-value="toggleDynamicWallpaper"
        />

        <!-- 动态壁纸素材选择(单屏/拼接模式共用;独立模式走下方逐屏选择) -->
        <div
          class="space-y-1.5"
          :class="{ 'opacity-40 pointer-events-none': !on(store.cfg.enable_dynamic_wallpaper) }"
        >
          <div class="flex items-center gap-1.5">
            <select
              v-model="materialSel"
              class="flex-1 min-w-0 text-[11px] bg-[var(--aurora-field)] rounded px-1.5 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
              aria-label="选择动态壁纸素材"
            >
              <option value="">(选择动态壁纸素材…)</option>
              <option v-for="e in materials" :key="e.path" :value="e.path">
                {{ e.name }}
              </option>
            </select>
            <button
              class="text-xs px-2 py-1 rounded bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)] text-white shrink-0"
              :disabled="!materialSel"
              aria-label="应用选中的动态壁纸素材"
              @click="applyMaterial"
            >
              应用
            </button>
            <button
              class="text-xs px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              aria-label="恢复系统壁纸"
              @click="clearMaterial"
            >
              恢复系统壁纸
            </button>
          </div>
          <div v-if="materialError" class="text-xs text-[var(--aurora-danger)] bg-[var(--aurora-danger-bg)] rounded-lg px-3 py-1.5">
            {{ materialError }}
          </div>
          <div
            v-else-if="materialNotice"
            class="text-xs text-[var(--aurora-success)] bg-[var(--aurora-success-bg)] rounded-lg px-3 py-1.5"
          >
            {{ materialNotice }}
          </div>
          <div
            v-else-if="materials.length === 0"
            class="text-[10px] text-[var(--aurora-text-dim)]"
          >
            素材目录为空(配置动态壁纸目录或放入 mp4/webm 等视频素材后点"刷新")
          </div>
        </div>

        <ToggleSwitch
          :model-value="on(store.cfg.wallpaper_battery_downshift)"
          :disabled="!on(store.cfg.enable_dynamic_wallpaper)"
          label="电池降载"
          description="电池模式下自动暂停动态渲染,减少耗电"
          @update:model-value="toggleBatteryDownshift"
        />

        <!-- Phase5 5.2 多显示器小节(设计文档 §2.3:开关/模式即时生效,素材逐屏设置) -->
        <div
          class="border-t border-[var(--aurora-border)] pt-2.5 space-y-2.5"
          :class="{ 'opacity-40 pointer-events-none': !on(store.cfg.enable_dynamic_wallpaper) }"
        >
          <ToggleSwitch
            :model-value="on(store.cfg.wallpaper_multi_monitor)"
            label="多显示器壁纸"
            description="每屏独立壁纸窗口;拼接 = 一张素材铺满全部屏幕"
            @update:model-value="toggleMultiMonitor"
          />

          <!-- 多屏开启后:模式单选 + 显示器只读信息 + 独立模式逐屏素材 -->
          <div v-if="on(store.cfg.wallpaper_multi_monitor)" class="space-y-2.5">
            <!-- 模式单选(拼接/独立) -->
            <div class="flex items-center gap-2">
              <button
                class="flex-1 text-[11px] px-2 py-1 rounded border"
                :class="
                  on(store.cfg.wallpaper_span_mode)
                    ? 'border-[var(--aurora-accent)] text-[var(--aurora-accent)]'
                    : 'border-[var(--aurora-border)] text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)]'
                "
                @click="setSpanMode(true)"
                aria-label="切换拼接模式"
              >
                拼接(一张铺满)
              </button>
              <button
                class="flex-1 text-[11px] px-2 py-1 rounded border"
                :class="
                  !on(store.cfg.wallpaper_span_mode)
                    ? 'border-[var(--aurora-accent)] text-[var(--aurora-accent)]'
                    : 'border-[var(--aurora-border)] text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)]'
                "
                @click="setSpanMode(false)"
                aria-label="切换独立模式"
              >
                独立(每屏单独)
              </button>
            </div>

            <div v-if="multiError" class="text-xs text-[var(--aurora-danger)] bg-[var(--aurora-danger-bg)] rounded-lg px-3 py-1.5">
              {{ multiError }}
            </div>

            <!-- 显示器信息(只读展示) -->
            <div class="text-[10px] text-[var(--aurora-text-dim)] space-y-0.5">
              <div v-for="m in monitors" :key="m.index">
                屏 {{ m.index + 1 }}{{ m.primary ? "(主)" : "" }}:
                {{ m.width }}×{{ m.height }}
                <span class="text-[var(--aurora-text-dim)] opacity-70">@({{ m.x }},{{ m.y }})</span>
              </div>
              <div v-if="monitors.length === 0" class="text-[var(--aurora-text-dim)] opacity-70">
                未获取到显示器信息(点击下方"刷新"重试)
              </div>
            </div>

            <!-- 独立模式:每屏素材选择(拼接模式素材统一由"应用壁纸"设置) -->
            <div v-if="!on(store.cfg.wallpaper_span_mode)" class="space-y-1.5">
              <div v-for="m in monitors" :key="m.index" class="flex items-center gap-1.5">
                <span class="text-[11px] w-12 shrink-0 text-[var(--aurora-text-dim)]">
                  屏{{ m.index + 1 }}
                </span>
                <select
                  v-model="perMonitorSel[m.index]"
                  class="flex-1 min-w-0 text-[11px] bg-[var(--aurora-field)] rounded px-1.5 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
                  :aria-label="`选择屏 ${m.index + 1} 的素材`"
                >
                  <option value="">(未设置,显示系统壁纸)</option>
                  <option v-for="e in materials" :key="e.path" :value="e.path">
                    {{ e.name }}
                  </option>
                </select>
                <button
                  class="text-xs px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
                  :disabled="!perMonitorSel[m.index]"
                  :aria-label="`应用屏 ${m.index + 1} 的素材`"
                  @click="applyMonitorMaterial(m.index)"
                >
                  应用
                </button>
              </div>
              <div class="flex gap-2 items-center">
                <button
                  class="text-xs px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
                  aria-label="刷新显示器信息"
                  @click="loadMultiMonitorState"
                >
                  刷新
                </button>
                <span class="text-[10px] text-[var(--aurora-text-dim)]">
                  清除某屏素材:用壁纸区"恢复系统壁纸"按钮整体恢复
                </span>
              </div>
            </div>
            <div v-else class="flex gap-2 items-center">
              <button
                class="text-xs px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
                aria-label="刷新显示器信息"
                @click="loadMultiMonitorState"
              >
                刷新
              </button>
              <span class="text-[10px] text-[var(--aurora-text-dim)]">
                拼接模式:素材用上方"动态壁纸素材"统一设置,自动铺满全部屏幕
              </span>
            </div>
          </div>
        </div>
      </div>

      <!-- Phase4 4.2/4.3 自动化区块(测试区待 4.2/4.3 模块合入后追加) -->
      <div v-if="store.cfg" class="border-t border-[var(--aurora-border)] pt-3 space-y-2.5">
        <ToggleSwitch
          :model-value="on(store.cfg.enable_automation)"
          label="自动化"
          description="键鼠模拟 + 控件操作,命令内实时校验,立即生效"
          @update:model-value="toggleAutomation"
        />
        <ToggleSwitch
          :model-value="on(store.cfg.automation_uia_enable)"
          :disabled="!on(store.cfg.enable_automation)"
          label="控件操作(UIA)"
          description="读取/点击窗口内控件,比键鼠模拟风险更高"
          @update:model-value="toggleUiaEnable"
        />
        <p class="text-[10px] text-[var(--aurora-text-dim)] leading-relaxed">
          自动化为高风险模块:普通用户权限下无法操作管理员窗口/UWP 应用;坐标点击依赖前台窗口位置,请确认目标可见
        </p>

        <!-- 4.2 键鼠模拟测试区(总开关开启后可用;错误红字展示) -->
        <div v-if="store.cfg.enable_automation" class="space-y-2">
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">坐标点击</span>
            <input
              v-model="simX"
              class="w-14 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)] font-mono"
              placeholder="x"
            />
            <input
              v-model="simY"
              class="w-14 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)] font-mono"
              placeholder="y"
            />
            <button
              class="text-xs px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              aria-label="执行坐标点击"
              @click="simClick"
            >
              点击
            </button>
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">输入文本</span>
            <input
              v-model="simText"
              class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
              placeholder="写入前台焦点窗口(中文安全)"
              @keyup.enter="simType"
            />
            <button
              class="text-xs px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              aria-label="执行文本输入"
              @click="simType"
            >
              输入
            </button>
          </div>
          <p v-if="simError" class="text-[10px] text-[var(--aurora-danger)] break-all">{{ simError }}</p>
        </div>

        <!-- 4.3 UIA 控件操作测试区(UIA 子开关也开启后可用) -->
        <div v-if="store.cfg.automation_uia_enable" class="space-y-2 border-t border-[var(--aurora-border)] pt-2">
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">窗口搜索</span>
            <input
              v-model="uiaWinTitle"
              class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
              placeholder="按标题子串搜索(留空列出全部可见窗口)"
              @keyup.enter="uiaSearchWindows"
            />
            <button
              class="text-xs px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              aria-label="搜索窗口"
              @click="uiaSearchWindows"
            >
              搜索
            </button>
          </div>
          <div v-if="uiaWindows.length" class="space-y-0.5 max-h-24 overflow-y-auto">
            <button
              v-for="w in uiaWindows"
              :key="w.hwnd"
              class="w-full text-left text-xs px-2 py-0.5 rounded truncate"
              :class="uiaSelHwnd === w.hwnd ? 'cfg-sel-tint' : 'bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]'"
              :title="`hwnd=${w.hwnd} class=${w.class} pid=${w.pid}`"
              :aria-label="`选择窗口:${w.title || '(无标题)'}`"
              @click="uiaListControls(w.hwnd)"
            >
              {{ w.title || "(无标题)" }} <span class="text-[var(--aurora-text-dim)]">#{{ w.hwnd }}</span>
            </button>
          </div>
          <div v-if="uiaSelHwnd !== null" class="space-y-0.5 max-h-28 overflow-y-auto">
            <div class="flex items-center justify-between">
              <span class="text-[10px] text-[var(--aurora-text-dim)]">控件({{ uiaControls.length }} 个,点选后操作)</span>
              <button
                v-if="uiaControls.length"
                class="text-xs px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
                aria-label="刷新控件列表"
                @click="uiaListControls(uiaSelHwnd)"
              >
                刷新
              </button>
            </div>
            <button
              v-for="c in uiaControls"
              :key="c.id"
              class="w-full text-left text-xs px-2 py-0.5 rounded truncate"
              :class="uiaSelId === c.id ? 'cfg-sel-tint' : 'bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]'"
              :title="`bounds=${c.bounds.join(',')}`"
              :aria-label="`选择控件:${c.control_type}`"
              @click="uiaSelId = c.id"
            >
              [{{ c.id }}] {{ c.control_type }} {{ c.name || "(无名称)" }}
            </button>
          </div>
          <div v-if="uiaSelId" class="flex items-center gap-1.5 flex-wrap">
            <button
              class="text-xs px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              aria-label="读取选中控件文本"
              @click="uiaReadText"
            >
              读文本
            </button>
            <button
              class="text-xs px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              aria-label="点击选中控件"
              @click="uiaClick"
            >
              点击
            </button>
            <input
              v-model="uiaTypeText"
              class="flex-1 min-w-0 text-sm bg-[var(--aurora-field)] rounded-lg px-2 py-1 focus:outline-none focus:ring-1 focus:ring-[var(--aurora-accent)]"
              placeholder="输入文本(中文安全)"
              @keyup.enter="uiaType"
            />
            <button
              class="text-xs px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              aria-label="输入文本到选中控件"
              @click="uiaType"
            >
              输入
            </button>
            <span v-if="uiaText" class="text-[10px] text-[var(--aurora-text-dim)] break-all w-full">{{ uiaText }}</span>
          </div>
          <p v-if="uiaError" class="text-[10px] text-[var(--aurora-danger)] break-all">{{ uiaError }}</p>
        </div>
      </div>

      <!-- Phase4 4.4 主题区块(切换后经 theme.ts 立即应用) -->
      <div v-if="store.cfg" class="border-t border-[var(--aurora-border)] pt-3 space-y-2.5">
        <div class="text-sm mb-1">主题</div>
        <div class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">外观</span>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'light' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            aria-label="切换浅色主题"
            @click="setThemeMode('light')"
          >
            浅色
          </button>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'dark' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            aria-label="切换深色主题"
            @click="setThemeMode('dark')"
          >
            深色
          </button>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'system' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            aria-label="切换跟随系统主题"
            @click="setThemeMode('system')"
          >
            跟随系统
          </button>
        </div>
        <div class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">强调色</span>
          <button
            v-for="c in ['blue', 'green', 'purple', 'orange']"
            :key="c"
            class="w-4 h-4 rounded-full transition-transform"
            :class="[
              c === 'blue' && 'bg-blue-500',
              c === 'green' && 'bg-green-500',
              c === 'purple' && 'bg-purple-500',
              c === 'orange' && 'bg-orange-500',
              store.cfg.theme_accent === c ? 'scale-110 ring-1 ring-[var(--aurora-border)]' : '',
            ]"
            :title="c"
            :aria-label="`强调色:${c}`"
            @click="setAccent(c)"
          />
        </div>
      </div>

      <!-- Phase5 5.1 自动更新区块(自研 updater;latest.json 源见设计文档 §1) -->
      <div v-if="store.cfg" class="border-t border-[var(--aurora-border)] pt-3 space-y-2.5">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">自动更新</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">
              当前版本 v{{ appVersion || "?" }},启动 15s 后 + 每 6 小时静默检查
            </div>
          </div>
          <button
            class="text-xs px-2.5 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
            :disabled="updateStatus === 'checking' || updateStatus === 'downloading'"
            aria-label="检查更新"
            @click="checkUpdate"
          >
            {{ updateStatus === "checking" ? "检查中…" : "检查更新" }}
          </button>
        </div>

        <div v-if="updateError" class="text-xs text-[var(--aurora-danger)] bg-[var(--aurora-danger-bg)] rounded-lg px-3 py-1.5">
          {{ updateError }}
        </div>
        <div
          v-else-if="updateStatus === 'latest'"
          class="text-xs text-[var(--aurora-success)] bg-[var(--aurora-success-bg)] rounded-lg px-3 py-1.5"
        >
          已是最新版本(更新源 v{{ updateVersion || "?" }})
        </div>
        <div
          v-else-if="updateStatus === 'available' || updateStatus === 'downloading' || updateStatus === 'downloaded'"
          class="text-xs bg-[var(--aurora-field)] rounded-lg px-3 py-1.5 space-y-1"
        >
          <div class="text-[var(--aurora-text)]">
            发现新版本 v{{ updateVersion || "?" }}
            <span v-if="updateStatus === 'downloading'" class="text-[var(--aurora-text-dim)]">
              (正在下载…)
            </span>
            <span v-else-if="updateStatus === 'downloaded'" class="text-[var(--aurora-success)]">
              (下载完成)
            </span>
          </div>
          <div v-if="updateNotes" class="text-[var(--aurora-text-dim)] break-all leading-relaxed">
            {{ updateNotes }}
          </div>
          <div class="flex items-center gap-2 pt-0.5">
            <button
              v-if="updateStatus === 'available' || updateStatus === 'downloaded'"
              class="text-xs px-2.5 py-1 rounded bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)] text-white"
              aria-label="下载并安装更新"
              @click="downloadAndInstall"
            >
              {{ updateStatus === "downloaded" ? "立即安装并重启" : "下载并安装" }}
            </button>
            <button
              class="text-xs px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
              aria-label="打开下载目录"
              @click="openUpdatesFolder"
            >
              打开下载目录
            </button>
          </div>
        </div>
      </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* 语义色低透明度选中底(UIA 列表选中态;同 WallpaperPanel 做法:
 * var() 任意值 + /opacity 修饰在 Tailwind v3 下不生成样式,用 color-mix 兜底) */
.cfg-sel-tint {
  background: color-mix(in srgb, var(--aurora-accent) 20%, transparent);
}
</style>
