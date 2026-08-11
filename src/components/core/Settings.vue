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

const phase2Modules = [
  { key: "enable_dock", label: "Dock 栏" },
  { key: "enable_file_drawer", label: "文件抽屉" },
  { key: "enable_clipboard_history", label: "剪贴板历史" },
] as const;

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
        <p class="text-[10px] text-white/30 mt-1">呼出/隐藏搜索框(Phase1 固定,Phase2 支持自定义)</p>
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

      <div v-for="item in phase2Modules" :key="item.key" class="flex items-center justify-between">
        <div>
          <div class="text-sm">{{ item.label }}</div>
          <div class="text-[10px] text-white/30">Phase2 开放,重启后生效</div>
        </div>
        <button
          class="w-10 h-5 rounded-full relative transition-colors"
          :class="store.cfg?.[item.key] ? 'bg-blue-500/80' : 'bg-white/10'"
          @click="toggleModule(item.key)"
        >
          <span
            class="absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all"
            :class="store.cfg?.[item.key] ? 'left-[22px]' : 'left-0.5'"
          />
        </button>
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
    </div>
  </div>
</template>
