<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useConfigStore } from "../../stores/config";
import WallpaperPanel from "./WallpaperPanel.vue";

const store = useConfigStore();
const emit = defineEmits<{ (e: "close"): void }>();

onMounted(async () => {
  void store.load();
  void loadMultiMonitorState(); // 显示器信息 + 素材列表 + 每屏当前素材(多屏关时也拉素材列表)
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

onUnmounted(() => {
  updateListeners.forEach((u) => u());
  updateListeners = [];
});

async function toggleIsland() {
  if (!store.cfg) return;
  store.cfg.enable_island = !store.cfg.enable_island;
  await store.save();
}

/** Phase2 模块开关(重启生效:窗口显隐/watcher/监听器都在启动时按配置初始化) */
async function toggleModule(
  key: "enable_dock" | "enable_file_drawer" | "enable_clipboard_history",
) {
  if (!store.cfg) return;
  store.cfg[key] = !store.cfg[key];
  await store.save();
}

/** Phase3 AI 总开关(重启生效:热键/托盘入口在启动时按配置初始化) */
async function toggleAiEnable() {
  if (!store.cfg) return;
  store.cfg.enable_ai = !store.cfg.enable_ai;
  await store.save();
}

/** Phase3 服务商切换(deepseek / ollama) */
async function setProvider(p: "deepseek" | "ollama") {
  if (!store.cfg) return;
  store.cfg.ai_provider = p;
  await store.save();
}

/** Phase3 文本输入保存(失焦/回车时提交,避免逐键保存) */
async function saveText() {
  await store.save();
}

/** Phase3 工具调用总开关(重启后随请求生效) */
async function toggleAiTools() {
  if (!store.cfg) return;
  store.cfg.ai_tools_enabled = !store.cfg.ai_tools_enabled;
  await store.save();
}

/** Phase3 搜索目录集合:文本域每行一个目录(默认空 = 仅桌面,禁止全盘扫描) */
async function onSearchRootsChange(e: Event) {
  if (!store.cfg) return;
  const text = (e.target as HTMLTextAreaElement).value;
  store.cfg.ai_search_roots = text
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  await store.save();
}

/** Phase3 密钥清除:传空串让后端清空(Rust 侧 resolve_key_save 对非掩码值直接生效) */
async function clearApiKey() {
  if (!store.cfg) return;
  store.cfg.ai_api_key = "";
  await store.save();
}

// ---- Phase4 模块开关(全部重启后生效:窗口/线程/命令门控都在启动时按配置初始化)----

/** Phase4 4.1 动态壁纸总开关 */
async function toggleDynamicWallpaper() {
  if (!store.cfg) return;
  store.cfg.enable_dynamic_wallpaper = !store.cfg.enable_dynamic_wallpaper;
  await store.save();
}

/** Phase4 4.1 电池降载子开关(受总开关控制,关闭时不可用) */
async function toggleBatteryDownshift() {
  if (!store.cfg) return;
  store.cfg.wallpaper_battery_downshift = !store.cfg.wallpaper_battery_downshift;
  await store.save();
}

/** Phase4 4.2/4.3 自动化总开关 */
async function toggleAutomation() {
  if (!store.cfg) return;
  store.cfg.enable_automation = !store.cfg.enable_automation;
  await store.save();
}

/** Phase4 4.3 UIA 控件操作子开关(受自动化总开关控制,关闭时不可用) */
async function toggleUiaEnable() {
  if (!store.cfg) return;
  store.cfg.automation_uia_enable = !store.cfg.automation_uia_enable;
  await store.save();
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
  await store.save();
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
  await store.save();
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

/** 应用选中的动态壁纸素材(video/html 走 WorkerW;图片走系统壁纸;拼接模式自动铺满全屏) */
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

/** Phase4 4.4 主题三态切换(system/dark/light;即时应用接线在 4.4 模块合入后接入 theme.ts) */
async function setThemeMode(mode: "system" | "dark" | "light") {
  if (!store.cfg) return;
  store.cfg.theme_mode = mode;
  await store.save();
}

/** Phase4 4.4 强调色选择(存 token 名,不存色值) */
async function setAccent(name: string) {
  if (!store.cfg) return;
  store.cfg.theme_accent = name;
  await store.save();
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
    class="h-full w-full flex flex-col bg-[var(--aurora-panel)] backdrop-blur-xl rounded-xl overflow-hidden text-[var(--aurora-text)]"
  >
    <div class="flex items-center justify-between px-4 py-3 border-b border-[var(--aurora-border)]">
      <span class="text-sm">设置</span>
      <button class="text-[var(--aurora-text-dim)] hover:text-[var(--aurora-text)] text-sm" title="关闭" @click="emit('close')">
        ✕
      </button>
    </div>
    <div class="flex-1 overflow-y-auto px-4 py-3 space-y-4">
      <div>
        <div class="text-xs text-[var(--aurora-text-dim)] mb-1">全局热键</div>
        <div class="text-sm font-mono bg-[var(--aurora-field)] rounded px-2 py-1 inline-block">
          {{ store.cfg?.hotkey_search ?? "Ctrl+Shift+Space" }}
        </div>
        <p class="text-[10px] text-[var(--aurora-text-dim)] mt-1">呼出/隐藏搜索框(固定快捷键,暂不可改)</p>
      </div>

      <div class="flex items-center justify-between">
        <div>
          <div class="text-sm">灵动岛</div>
          <div class="text-[10px] text-[var(--aurora-text-dim)]">顶部常驻:时间 + CPU/内存/网络,重启后生效</div>
        </div>
        <button
          class="w-10 h-5 rounded-full relative transition-colors"
          :class="store.cfg?.enable_island ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
          @click="toggleIsland"
        >
          <span
            class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
            :class="store.cfg?.enable_island ? 'left-[22px]' : 'left-0.5'"
          />
        </button>
      </div>

      <!-- Dock 栏:悬停呼出,无全局热键 -->
      <div class="flex items-center justify-between">
        <div>
          <div class="text-sm">Dock 栏</div>
          <div class="text-[10px] text-[var(--aurora-text-dim)]">Phase2 开放,悬停呼出,重启后生效</div>
        </div>
        <button
          class="w-10 h-5 rounded-full relative transition-colors"
          :class="store.cfg?.enable_dock ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
          @click="toggleModule('enable_dock')"
        >
          <span
            class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
            :class="store.cfg?.enable_dock ? 'left-[22px]' : 'left-0.5'"
          />
        </button>
      </div>

      <!-- 文件抽屉:热键受模块开关门控,关闭时仅展示不生效 -->
      <div class="flex flex-col gap-1.5">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">文件抽屉</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">Phase2 开放,重启后生效</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="store.cfg?.enable_file_drawer ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            @click="toggleModule('enable_file_drawer')"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="store.cfg?.enable_file_drawer ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
        <div v-if="store.cfg" class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">抽屉热键</span>
          <input
            v-model="store.cfg.drawer_hotkey"
            class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)] font-mono"
            @change="saveText"
          />
          <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">重启后生效</span>
        </div>
        <p v-if="store.cfg && !store.cfg.enable_file_drawer" class="text-[10px] text-[var(--aurora-text-dim)] ml-[70px]">
          模块关闭时不生效
        </p>
      </div>

      <!-- 剪贴板历史:热键受模块开关门控,关闭时仅展示不生效 -->
      <div class="flex flex-col gap-1.5">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">剪贴板历史</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">Phase2 开放,重启后生效</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="store.cfg?.enable_clipboard_history ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            @click="toggleModule('enable_clipboard_history')"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="store.cfg?.enable_clipboard_history ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
        <div v-if="store.cfg" class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">剪贴板热键</span>
          <input
            v-model="store.cfg.hotkey_clipboard"
            class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)] font-mono"
            @change="saveText"
          />
          <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">重启后生效</span>
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
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">AI 助手</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">
              总开关,关闭时不注册热键、不显示托盘入口,重启后生效
            </div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="store.cfg.enable_ai ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            @click="toggleAiEnable"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="store.cfg.enable_ai ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>

        <div v-if="store.cfg.enable_ai" class="space-y-2.5">
          <!-- 服务商切换 -->
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">服务商</span>
            <button
              class="text-xs px-2 py-0.5 rounded transition-colors"
              :class="store.cfg.ai_provider === 'deepseek' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
              @click="setProvider('deepseek')"
            >
              DeepSeek
            </button>
            <button
              class="text-xs px-2 py-0.5 rounded transition-colors"
              :class="store.cfg.ai_provider === 'ollama' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
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
                class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)] font-mono"
                placeholder="未配置"
                @change="saveText"
              />
              <button
                class="text-[10px] px-1.5 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
                title="清空密钥"
                @click="clearApiKey"
              >
                清除
              </button>
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">模型</span>
              <input
                v-model="store.cfg.ai_model"
                class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
                @change="saveText"
              />
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">接口地址</span>
              <input
                v-model="store.cfg.ai_base_url"
                class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
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
                class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
                @change="saveText"
              />
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">Ollama 模型</span>
              <input
                v-model="store.cfg.ai_ollama_model"
                class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
                @change="saveText"
              />
            </div>
            <p class="text-[10px] text-[var(--aurora-text-dim)] ml-[70px]">
              按本机已装模型修改,如 qwen2.5:7b
            </p>
          </template>

          <!-- 工具调用总开关 -->
          <div class="flex items-center justify-between">
            <div>
              <div class="text-sm">工具调用</div>
              <div class="text-[10px] text-[var(--aurora-text-dim)]">
                让 AI 打开应用/搜文件/设壁纸等,关闭后纯对话
              </div>
            </div>
            <button
              class="w-10 h-5 rounded-full relative transition-colors"
              :class="store.cfg.ai_tools_enabled ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
              @click="toggleAiTools"
            >
              <span
                class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
                :class="store.cfg.ai_tools_enabled ? 'left-[22px]' : 'left-0.5'"
              />
            </button>
          </div>

          <!-- 搜索目录集合(每行一个,默认空 = 仅桌面) -->
          <div>
            <div class="text-xs text-[var(--aurora-text-dim)] mb-1">文件搜索目录</div>
            <textarea
              class="w-full text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)] font-mono resize-y leading-relaxed"
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
              class="w-16 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
              @change="saveText"
            />
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">AI 热键</span>
            <input
              v-model="store.cfg.ai_hotkey"
              class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)] font-mono"
              @change="saveText"
            />
            <span class="text-[10px] text-[var(--aurora-text-dim)] shrink-0">重启后生效</span>
          </div>
        </div>
      </div>

      <!-- Phase4 4.1 动态壁纸区块(素材选择/预览待 4.1 模块合入后启用) -->
      <div v-if="store.cfg" class="border-t border-[var(--aurora-border)] pt-3 space-y-2.5">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">动态壁纸</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">
              WorkerW 壁纸层:本地视频/网页壁纸,重启后生效
            </div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="on(store.cfg.enable_dynamic_wallpaper) ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            @click="toggleDynamicWallpaper"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="on(store.cfg.enable_dynamic_wallpaper) ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>

        <!-- 动态壁纸素材选择(单屏/拼接模式共用;独立模式走下方逐屏选择) -->
        <div
          class="space-y-1.5"
          :class="{ 'opacity-40 pointer-events-none': !on(store.cfg.enable_dynamic_wallpaper) }"
        >
          <div class="flex items-center gap-1.5">
            <select
              v-model="materialSel"
              class="flex-1 min-w-0 text-[11px] bg-[var(--aurora-field)] rounded px-1.5 py-1 outline-none focus:bg-[var(--aurora-field)]"
            >
              <option value="">(选择动态壁纸素材…)</option>
              <option v-for="e in materials" :key="e.path" :value="e.path">
                {{ e.name }}
              </option>
            </select>
            <button
              class="text-[10px] px-2 py-1 rounded bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)] text-white shrink-0"
              :disabled="!materialSel"
              @click="applyMaterial"
            >
              应用
            </button>
            <button
              class="text-[10px] px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              @click="clearMaterial"
            >
              恢复系统壁纸
            </button>
          </div>
          <div v-if="materialError" class="text-xs text-red-300 bg-red-500/10 rounded-lg px-3 py-1.5">
            {{ materialError }}
          </div>
          <div
            v-else-if="materialNotice"
            class="text-xs text-emerald-300 bg-emerald-500/10 rounded-lg px-3 py-1.5"
          >
            {{ materialNotice }}
          </div>
          <div
            v-else-if="materials.length === 0"
            class="text-[10px] text-[var(--aurora-text-dim)]"
          >
            素材目录为空(配置动态壁纸目录或放入 mp4/html 素材后点"刷新")
          </div>
        </div>

        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">电池降载</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">电池模式下自动暂停动态渲染,减少耗电</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="
              on(store.cfg.enable_dynamic_wallpaper)
                ? on(store.cfg.wallpaper_battery_downshift)
                  ? 'bg-[var(--aurora-accent)]'
                  : 'bg-[var(--aurora-field)]'
                : 'bg-[var(--aurora-field)] opacity-40'
            "
            :disabled="!on(store.cfg.enable_dynamic_wallpaper)"
            @click="toggleBatteryDownshift"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="on(store.cfg.wallpaper_battery_downshift) ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>

        <!-- Phase5 5.2 多显示器小节(设计文档 §2.3:开关/模式即时生效,素材逐屏设置) -->
        <div
          class="border-t border-[var(--aurora-border)] pt-2.5 space-y-2.5"
          :class="{ 'opacity-40 pointer-events-none': !on(store.cfg.enable_dynamic_wallpaper) }"
        >
          <div class="flex items-center justify-between">
            <div>
              <div class="text-sm">多显示器壁纸</div>
              <div class="text-[10px] text-[var(--aurora-text-dim)]">
                每屏独立壁纸窗口;拼接 = 一张素材铺满全部屏幕
              </div>
            </div>
            <button
              class="w-10 h-5 rounded-full relative transition-colors"
              :class="on(store.cfg.wallpaper_multi_monitor) ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
              @click="toggleMultiMonitor"
            >
              <span
                class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
                :class="on(store.cfg.wallpaper_multi_monitor) ? 'left-[22px]' : 'left-0.5'"
              />
            </button>
          </div>

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
              >
                独立(每屏单独)
              </button>
            </div>

            <div v-if="multiError" class="text-xs text-red-300 bg-red-500/10 rounded-lg px-3 py-1.5">
              {{ multiError }}
            </div>

            <!-- 显示器信息(只读展示) -->
            <div class="text-[10px] text-[var(--aurora-text-dim)] space-y-0.5">
              <div v-for="m in monitors" :key="m.index">
                屏 {{ m.index + 1 }}{{ m.primary ? "(主)" : "" }}:
                {{ m.width }}×{{ m.height }}
                <span class="text-[var(--aurora-text-dim)]/70">@({{ m.x }},{{ m.y }})</span>
              </div>
              <div v-if="monitors.length === 0" class="text-[var(--aurora-text-dim)]/70">
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
                  class="flex-1 min-w-0 text-[11px] bg-[var(--aurora-field)] rounded px-1.5 py-1 outline-none focus:bg-[var(--aurora-field)]"
                >
                  <option value="">(未设置,显示系统壁纸)</option>
                  <option v-for="e in materials" :key="e.path" :value="e.path">
                    {{ e.name }}
                  </option>
                </select>
                <button
                  class="text-[10px] px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
                  :disabled="!perMonitorSel[m.index]"
                  @click="applyMonitorMaterial(m.index)"
                >
                  应用
                </button>
              </div>
              <div class="flex gap-2 items-center">
                <button
                  class="text-[10px] px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
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
                class="text-[10px] px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
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
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">自动化</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">键鼠模拟 + 控件操作,重启后生效</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="on(store.cfg.enable_automation) ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            @click="toggleAutomation"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="on(store.cfg.enable_automation) ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">控件操作(UIA)</div>
            <div class="text-[10px] text-[var(--aurora-text-dim)]">读取/点击窗口内控件,比键鼠模拟风险更高</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="
              on(store.cfg.enable_automation)
                ? on(store.cfg.automation_uia_enable)
                  ? 'bg-[var(--aurora-accent)]'
                  : 'bg-[var(--aurora-field)]'
                : 'bg-[var(--aurora-field)] opacity-40'
            "
            :disabled="!on(store.cfg.enable_automation)"
            @click="toggleUiaEnable"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="on(store.cfg.automation_uia_enable) ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
        <p class="text-[10px] text-[var(--aurora-text-dim)] leading-relaxed">
          自动化为高风险模块:普通用户权限下无法操作管理员窗口/UWP 应用;坐标点击依赖前台窗口位置,请确认目标可见
        </p>

        <!-- 4.2 键鼠模拟测试区(总开关开启后可用;错误红字展示) -->
        <div v-if="store.cfg.enable_automation" class="space-y-2">
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">坐标点击</span>
            <input
              v-model="simX"
              class="w-14 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)] font-mono"
              placeholder="x"
            />
            <input
              v-model="simY"
              class="w-14 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)] font-mono"
              placeholder="y"
            />
            <button
              class="text-[10px] px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              @click="simClick"
            >
              点击
            </button>
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">输入文本</span>
            <input
              v-model="simText"
              class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
              placeholder="写入前台焦点窗口(中文安全)"
              @keyup.enter="simType"
            />
            <button
              class="text-[10px] px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              @click="simType"
            >
              输入
            </button>
          </div>
          <p v-if="simError" class="text-[10px] text-red-400 break-all">{{ simError }}</p>
        </div>

        <!-- 4.3 UIA 控件操作测试区(UIA 子开关也开启后可用) -->
        <div v-if="store.cfg.automation_uia_enable" class="space-y-2 border-t border-[var(--aurora-border)] pt-2">
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">窗口搜索</span>
            <input
              v-model="uiaWinTitle"
              class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
              placeholder="按标题子串搜索(留空列出全部可见窗口)"
              @keyup.enter="uiaSearchWindows"
            />
            <button
              class="text-[10px] px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              @click="uiaSearchWindows"
            >
              搜索
            </button>
          </div>
          <div v-if="uiaWindows.length" class="space-y-0.5 max-h-24 overflow-y-auto">
            <button
              v-for="w in uiaWindows"
              :key="w.hwnd"
              class="w-full text-left text-[10px] px-2 py-0.5 rounded truncate"
              :class="uiaSelHwnd === w.hwnd ? 'bg-[var(--aurora-accent)]/20' : 'bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]'"
              :title="`hwnd=${w.hwnd} class=${w.class} pid=${w.pid}`"
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
                class="text-[10px] px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
                @click="uiaListControls(uiaSelHwnd)"
              >
                刷新
              </button>
            </div>
            <button
              v-for="c in uiaControls"
              :key="c.id"
              class="w-full text-left text-[10px] px-2 py-0.5 rounded truncate"
              :class="uiaSelId === c.id ? 'bg-[var(--aurora-accent)]/20' : 'bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]'"
              :title="`bounds=${c.bounds.join(',')}`"
              @click="uiaSelId = c.id"
            >
              [{{ c.id }}] {{ c.control_type }} {{ c.name || "(无名称)" }}
            </button>
          </div>
          <div v-if="uiaSelId" class="flex items-center gap-1.5 flex-wrap">
            <button
              class="text-[10px] px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              @click="uiaReadText"
            >
              读文本
            </button>
            <button
              class="text-[10px] px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              @click="uiaClick"
            >
              点击
            </button>
            <input
              v-model="uiaTypeText"
              class="flex-1 min-w-0 text-xs bg-[var(--aurora-field)] rounded px-2 py-1 outline-none focus:bg-[var(--aurora-field)]"
              placeholder="输入文本(中文安全)"
              @keyup.enter="uiaType"
            />
            <button
              class="text-[10px] px-2 py-0.5 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
              @click="uiaType"
            >
              输入
            </button>
            <span v-if="uiaText" class="text-[10px] text-[var(--aurora-text-dim)] break-all w-full">{{ uiaText }}</span>
          </div>
          <p v-if="uiaError" class="text-[10px] text-red-400 break-all">{{ uiaError }}</p>
        </div>
      </div>

      <!-- Phase4 4.4 主题区块(即时应用接线在 4.4 模块合入后接入 theme.ts) -->
      <div v-if="store.cfg" class="border-t border-[var(--aurora-border)] pt-3 space-y-2.5">
        <div class="text-sm mb-1">主题</div>
        <div class="flex items-center gap-1.5">
          <span class="text-xs text-[var(--aurora-text-dim)] w-16 shrink-0">外观</span>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'light' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            @click="setThemeMode('light')"
          >
            浅色
          </button>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'dark' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
            @click="setThemeMode('dark')"
          >
            深色
          </button>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'system' ? 'bg-[var(--aurora-accent)]' : 'bg-[var(--aurora-field)]'"
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
            class="text-[10px] px-2.5 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)] shrink-0"
            :disabled="updateStatus === 'checking' || updateStatus === 'downloading'"
            @click="checkUpdate"
          >
            {{ updateStatus === "checking" ? "检查中…" : "检查更新" }}
          </button>
        </div>

        <div v-if="updateError" class="text-xs text-red-300 bg-red-500/10 rounded-lg px-3 py-1.5">
          {{ updateError }}
        </div>
        <div
          v-else-if="updateStatus === 'latest'"
          class="text-xs text-emerald-300 bg-emerald-500/10 rounded-lg px-3 py-1.5"
        >
          已是最新版本(更新源 v{{ updateVersion || "?" }})
        </div>
        <div
          v-else-if="updateStatus === 'available' || updateStatus === 'downloading' || updateStatus === 'downloaded'"
          class="text-xs bg-[var(--aurora-field)]/60 rounded-lg px-3 py-1.5 space-y-1"
        >
          <div class="text-[var(--aurora-text)]">
            发现新版本 v{{ updateVersion || "?" }}
            <span v-if="updateStatus === 'downloading'" class="text-[var(--aurora-text-dim)]">
              (正在下载…)
            </span>
            <span v-else-if="updateStatus === 'downloaded'" class="text-emerald-300">
              (下载完成)
            </span>
          </div>
          <div v-if="updateNotes" class="text-[var(--aurora-text-dim)] break-all leading-relaxed">
            {{ updateNotes }}
          </div>
          <div class="flex items-center gap-2 pt-0.5">
            <button
              v-if="updateStatus === 'available' || updateStatus === 'downloaded'"
              class="text-[10px] px-2.5 py-1 rounded bg-[var(--aurora-accent)] hover:bg-[var(--aurora-accent)] text-white"
              @click="downloadAndInstall"
            >
              {{ updateStatus === "downloaded" ? "立即安装并重启" : "下载并安装" }}
            </button>
            <button
              class="text-[10px] px-2 py-1 rounded bg-[var(--aurora-field)] hover:bg-[var(--aurora-field)]"
              @click="openUpdatesFolder"
            >
              打开下载目录
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
