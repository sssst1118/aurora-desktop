<script setup lang="ts">
import { onMounted } from "vue";
import { useConfigStore } from "../../stores/config";
import WallpaperPanel from "./WallpaperPanel.vue";

const store = useConfigStore();
const emit = defineEmits<{ (e: "close"): void }>();

onMounted(() => {
  void store.load();
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

/** 开关组件标记(纯展示辅助,避免模板重复) */
const on = (v: boolean | undefined) => v === true;
</script>

<template>
  <div
    class="h-full w-full flex flex-col bg-black/70 backdrop-blur-xl rounded-xl overflow-hidden text-white"
  >
    <div class="flex items-center justify-between px-4 py-3 border-b border-white/10">
      <span class="text-sm">设置</span>
      <button class="text-white/40 hover:text-white/80 text-sm" title="关闭" @click="emit('close')">
        ✕
      </button>
    </div>
    <div class="flex-1 overflow-y-auto px-4 py-3 space-y-4">
      <div>
        <div class="text-xs text-white/50 mb-1">全局热键</div>
        <div class="text-sm font-mono bg-white/5 rounded px-2 py-1 inline-block">
          {{ store.cfg?.hotkey_search ?? "Ctrl+Shift+Space" }}
        </div>
        <p class="text-[10px] text-white/30 mt-1">呼出/隐藏搜索框(固定快捷键,暂不可改)</p>
      </div>

      <div class="flex items-center justify-between">
        <div>
          <div class="text-sm">灵动岛</div>
          <div class="text-[10px] text-white/30">顶部常驻:时间 + CPU/内存/网络,重启后生效</div>
        </div>
        <button
          class="w-10 h-5 rounded-full relative transition-colors"
          :class="store.cfg?.enable_island ? 'bg-blue-500/80' : 'bg-white/10'"
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
          <div class="text-[10px] text-white/30">Phase2 开放,悬停呼出,重启后生效</div>
        </div>
        <button
          class="w-10 h-5 rounded-full relative transition-colors"
          :class="store.cfg?.enable_dock ? 'bg-blue-500/80' : 'bg-white/10'"
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
            <div class="text-[10px] text-white/30">Phase2 开放,重启后生效</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="store.cfg?.enable_file_drawer ? 'bg-blue-500/80' : 'bg-white/10'"
            @click="toggleModule('enable_file_drawer')"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="store.cfg?.enable_file_drawer ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
        <div v-if="store.cfg" class="flex items-center gap-1.5">
          <span class="text-xs text-white/50 w-16 shrink-0">抽屉热键</span>
          <input
            v-model="store.cfg.drawer_hotkey"
            class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10 font-mono"
            @change="saveText"
          />
          <span class="text-[10px] text-white/30 shrink-0">重启后生效</span>
        </div>
        <p v-if="store.cfg && !store.cfg.enable_file_drawer" class="text-[10px] text-white/30 ml-[70px]">
          模块关闭时不生效
        </p>
      </div>

      <!-- 剪贴板历史:热键受模块开关门控,关闭时仅展示不生效 -->
      <div class="flex flex-col gap-1.5">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">剪贴板历史</div>
            <div class="text-[10px] text-white/30">Phase2 开放,重启后生效</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="store.cfg?.enable_clipboard_history ? 'bg-blue-500/80' : 'bg-white/10'"
            @click="toggleModule('enable_clipboard_history')"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="store.cfg?.enable_clipboard_history ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
        <div v-if="store.cfg" class="flex items-center gap-1.5">
          <span class="text-xs text-white/50 w-16 shrink-0">剪贴板热键</span>
          <input
            v-model="store.cfg.hotkey_clipboard"
            class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10 font-mono"
            @change="saveText"
          />
          <span class="text-[10px] text-white/30 shrink-0">重启后生效</span>
        </div>
        <p v-if="store.cfg && !store.cfg.enable_clipboard_history" class="text-[10px] text-white/30 ml-[70px]">
          模块关闭时不生效
        </p>
      </div>

      <!-- 2.4 静态壁纸区块 -->
      <div>
        <div class="text-sm mb-1.5">壁纸</div>
        <WallpaperPanel />
      </div>

      <!-- Phase3 AI 设置区块(密钥脱敏契约:config_load 返回掩码,前端永不见明文) -->
      <div v-if="store.cfg" class="border-t border-white/10 pt-3 space-y-3">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">AI 助手</div>
            <div class="text-[10px] text-white/30">
              总开关,关闭时不注册热键、不显示托盘入口,重启后生效
            </div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="store.cfg.enable_ai ? 'bg-blue-500/80' : 'bg-white/10'"
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
            <span class="text-xs text-white/50 w-16 shrink-0">服务商</span>
            <button
              class="text-xs px-2 py-0.5 rounded transition-colors"
              :class="store.cfg.ai_provider === 'deepseek' ? 'bg-blue-500/70' : 'bg-white/10'"
              @click="setProvider('deepseek')"
            >
              DeepSeek
            </button>
            <button
              class="text-xs px-2 py-0.5 rounded transition-colors"
              :class="store.cfg.ai_provider === 'ollama' ? 'bg-blue-500/70' : 'bg-white/10'"
              @click="setProvider('ollama')"
            >
              Ollama
            </button>
          </div>

          <!-- DeepSeek:密钥(password 型,已配置显示掩码)+ 模型 + 接口地址 -->
          <template v-if="store.cfg.ai_provider === 'deepseek'">
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-white/50 w-16 shrink-0">API Key</span>
              <input
                v-model="store.cfg.ai_api_key"
                type="password"
                class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10 font-mono"
                placeholder="未配置"
                @change="saveText"
              />
              <button
                class="text-[10px] px-1.5 py-0.5 rounded bg-white/10 hover:bg-white/20 shrink-0"
                title="清空密钥"
                @click="clearApiKey"
              >
                清除
              </button>
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-white/50 w-16 shrink-0">模型</span>
              <input
                v-model="store.cfg.ai_model"
                class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10"
                @change="saveText"
              />
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-white/50 w-16 shrink-0">接口地址</span>
              <input
                v-model="store.cfg.ai_base_url"
                class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10"
                @change="saveText"
              />
            </div>
          </template>

          <!-- Ollama:本地地址 + 模型 -->
          <template v-else>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-white/50 w-16 shrink-0">Ollama 地址</span>
              <input
                v-model="store.cfg.ai_ollama_url"
                class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10"
                @change="saveText"
              />
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-xs text-white/50 w-16 shrink-0">Ollama 模型</span>
              <input
                v-model="store.cfg.ai_ollama_model"
                class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10"
                @change="saveText"
              />
            </div>
            <p class="text-[10px] text-white/30 ml-[70px]">
              按本机已装模型修改,如 qwen2.5:7b
            </p>
          </template>

          <!-- 工具调用总开关 -->
          <div class="flex items-center justify-between">
            <div>
              <div class="text-sm">工具调用</div>
              <div class="text-[10px] text-white/30">
                让 AI 打开应用/搜文件/设壁纸等,关闭后纯对话
              </div>
            </div>
            <button
              class="w-10 h-5 rounded-full relative transition-colors"
              :class="store.cfg.ai_tools_enabled ? 'bg-blue-500/80' : 'bg-white/10'"
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
            <div class="text-xs text-white/50 mb-1">文件搜索目录</div>
            <textarea
              class="w-full text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10 font-mono resize-y leading-relaxed"
              rows="2"
              placeholder="每行一个目录(留空 = 仅桌面,禁止全盘扫描)"
              :value="store.cfg.ai_search_roots.join('\n')"
              @change="onSearchRootsChange"
            />
          </div>

          <!-- 工具循环上限 + AI 热键 -->
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-white/50 w-16 shrink-0">工具轮数上限</span>
            <input
              v-model.number="store.cfg.ai_max_tool_rounds"
              type="number"
              min="1"
              max="10"
              class="w-16 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10"
              @change="saveText"
            />
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-xs text-white/50 w-16 shrink-0">AI 热键</span>
            <input
              v-model="store.cfg.ai_hotkey"
              class="flex-1 min-w-0 text-xs bg-white/5 rounded px-2 py-1 outline-none focus:bg-white/10 font-mono"
              @change="saveText"
            />
            <span class="text-[10px] text-white/30 shrink-0">重启后生效</span>
          </div>
        </div>
      </div>

      <!-- Phase4 4.1 动态壁纸区块(素材选择/预览待 4.1 模块合入后启用) -->
      <div v-if="store.cfg" class="border-t border-white/10 pt-3 space-y-2.5">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">动态壁纸</div>
            <div class="text-[10px] text-white/30">
              WorkerW 壁纸层:本地视频/网页壁纸,重启后生效
            </div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="on(store.cfg.enable_dynamic_wallpaper) ? 'bg-blue-500/80' : 'bg-white/10'"
            @click="toggleDynamicWallpaper"
          >
            <span
              class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
              :class="on(store.cfg.enable_dynamic_wallpaper) ? 'left-[22px]' : 'left-0.5'"
            />
          </button>
        </div>
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">电池降载</div>
            <div class="text-[10px] text-white/30">电池模式下自动暂停动态渲染,减少耗电</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="
              on(store.cfg.enable_dynamic_wallpaper)
                ? on(store.cfg.wallpaper_battery_downshift)
                  ? 'bg-blue-500/80'
                  : 'bg-white/10'
                : 'bg-white/10 opacity-40'
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
      </div>

      <!-- Phase4 4.2/4.3 自动化区块(测试区待 4.2/4.3 模块合入后追加) -->
      <div v-if="store.cfg" class="border-t border-white/10 pt-3 space-y-2.5">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm">自动化</div>
            <div class="text-[10px] text-white/30">键鼠模拟 + 控件操作,重启后生效</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="on(store.cfg.enable_automation) ? 'bg-blue-500/80' : 'bg-white/10'"
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
            <div class="text-[10px] text-white/30">读取/点击窗口内控件,比键鼠模拟风险更高</div>
          </div>
          <button
            class="w-10 h-5 rounded-full relative transition-colors"
            :class="
              on(store.cfg.enable_automation)
                ? on(store.cfg.automation_uia_enable)
                  ? 'bg-blue-500/80'
                  : 'bg-white/10'
                : 'bg-white/10 opacity-40'
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
        <p class="text-[10px] text-white/30 leading-relaxed">
          自动化为高风险模块:普通用户权限下无法操作管理员窗口/UWP 应用;坐标点击依赖前台窗口位置,请确认目标可见
        </p>
      </div>

      <!-- Phase4 4.4 主题区块(即时应用接线在 4.4 模块合入后接入 theme.ts) -->
      <div v-if="store.cfg" class="border-t border-white/10 pt-3 space-y-2.5">
        <div class="text-sm mb-1">主题</div>
        <div class="flex items-center gap-1.5">
          <span class="text-xs text-white/50 w-16 shrink-0">外观</span>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'light' ? 'bg-blue-500/70' : 'bg-white/10'"
            @click="setThemeMode('light')"
          >
            浅色
          </button>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'dark' ? 'bg-blue-500/70' : 'bg-white/10'"
            @click="setThemeMode('dark')"
          >
            深色
          </button>
          <button
            class="text-xs px-2 py-0.5 rounded transition-colors"
            :class="store.cfg.theme_mode === 'system' ? 'bg-blue-500/70' : 'bg-white/10'"
            @click="setThemeMode('system')"
          >
            跟随系统
          </button>
        </div>
        <div class="flex items-center gap-1.5">
          <span class="text-xs text-white/50 w-16 shrink-0">强调色</span>
          <button
            v-for="c in ['blue', 'green', 'purple', 'orange']"
            :key="c"
            class="w-4 h-4 rounded-full transition-transform"
            :class="[
              c === 'blue' && 'bg-blue-500',
              c === 'green' && 'bg-green-500',
              c === 'purple' && 'bg-purple-500',
              c === 'orange' && 'bg-orange-500',
              store.cfg.theme_accent === c ? 'scale-110 ring-1 ring-white/60' : '',
            ]"
            :title="c"
            @click="setAccent(c)"
          />
        </div>
      </div>
    </div>
  </div>
</template>
