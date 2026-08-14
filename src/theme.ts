/**
 * Phase4 4.4 主题系统 → Phase6 视觉系统扩展(设计文档 §5):皮肤包 + 强调色。
 * - 皮肤:document.documentElement.dataset.skin 切换 global.css 里 4 套令牌(:root 默认=deep);
 *   theme_mode 保留兼容:老用户升级(无 skin 字段或默认 "deep")且 theme_mode==="light" 时按拂晓(dawn)呈现;
 * - 深浅:html.dark class 机制保留(系统级 color-scheme 与未来 dark: 变体),皮肤令牌自身决定明暗;
 * - 强调色:4 色 token 名 → 色值,覆盖到 :root 的 --aurora-accent(所有皮肤共用同一选择);
 * - 导出 apply_theme(cfg) 供 main.ts(启动 config_load 后)与 Settings(切换时 + config_save)调用。
 */

/** 皮肤白名单(与后端 AppConfig.skin 契约一致) */
const VALID_SKINS = ["deep", "midnight", "dawn", "verdant"];

/** 强调色 token 名 → 色值(与 Settings.vue 四色圆点一致,Phase6 新色值) */
const ACCENT_COLORS: Record<string, string> = {
  blue: "#38bdf8",
  purple: "#a78bfa",
  green: "#34d399",
  orange: "#fbbf24",
};

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** system 模式的 matchMedia 监听(重复调用先注销,不叠加) */
let media: MediaQueryList | null = null;
let mediaHandler: ((e: MediaQueryListEvent) => void) | null = null;

/** 把解析出的深浅状态落到 <html> 上(dark class + color-scheme 同步) */
function applyDarkState(isDark: boolean) {
  const html = document.documentElement;
  html.classList.toggle("dark", isDark);
  html.style.colorScheme = isDark ? "dark" : "light";
}

/** 应用皮肤包:设置 <html data-skin>,global.css 按选择器切换整套令牌;非法值回退 deep */
export function apply_skin(skin: string): void {
  document.documentElement.dataset.skin = VALID_SKINS.includes(skin) ? skin : "deep";
}

/** 解析 skin 字段:显式非默认皮肤直接用;缺失或默认 "deep"(老用户)+ 浅色主题 → 拂晓(兼容映射) */
function resolveSkin(cfg: { theme_mode: string; skin?: string }): string {
  const skin = cfg.skin ?? "deep";
  if (VALID_SKINS.includes(skin) && skin !== "deep") return skin;
  if (cfg.theme_mode === "light") return "dawn";
  return "deep";
}

/**
 * 应用主题配置。
 * @param cfg.theme_mode   "system" | "dark" | "light"(AppConfig.theme_mode,兼容保留)
 * @param cfg.theme_accent "blue" | "green" | "purple" | "orange"(AppConfig.theme_accent,存 token 名)
 * @param cfg.skin         "deep" | "midnight" | "dawn" | "verdant"(AppConfig.skin,可选,老配置无此字段)
 */
export function apply_theme(cfg: { theme_mode: string; theme_accent: string; skin?: string }): void {
  // 皮肤包(先于强调色,dark class 不影响皮肤令牌)
  apply_skin(resolveSkin(cfg));

  // 强调色:覆盖在 :root 上(内联样式优先级高于皮肤块),所有皮肤共用同一色值
  const accent = ACCENT_COLORS[cfg.theme_accent] ?? ACCENT_COLORS.blue;
  document.documentElement.style.setProperty("--aurora-accent", accent);

  // 先注销旧 system 监听,避免重复注册
  if (media && mediaHandler) {
    media.removeEventListener("change", mediaHandler);
    media = null;
    mediaHandler = null;
  }

  const mode = cfg.theme_mode ?? "system";
  if (mode === "dark") {
    applyDarkState(true);
  } else if (mode === "light") {
    applyDarkState(false);
  } else {
    // system:当前系统偏好决定,并监听系统切换时重算
    media = window.matchMedia(DARK_QUERY);
    applyDarkState(media.matches);
    mediaHandler = (e) => applyDarkState(e.matches);
    media.addEventListener("change", mediaHandler);
  }
}
