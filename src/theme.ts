/**
 * Phase4 4.4 主题系统:深浅色切换 + 强调色(设计文档 §4.2)。
 * - 深浅:html.dark class 驱动 :root.dark 令牌块;system 模式跟随 prefers-color-scheme 并注册 change 监听;
 * - 强调色:4 色 token 名 → 色值,覆盖到 :root 的 --aurora-accent(两主题共用,不落在 .dark 选择器下);
 * - 导出 apply_theme(cfg) 供 main.ts(启动 config_load 后)与 Settings(切换时 + config_save)调用。
 */

/** 强调色 token 名 → 色值(与 Settings.vue 四色圆点 bg-blue-500 等一致) */
const ACCENT_COLORS: Record<string, string> = {
  blue: "#3b82f6",
  green: "#22c55e",
  purple: "#a855f7",
  orange: "#f97316",
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

/**
 * 应用主题配置。
 * @param cfg.theme_mode   "system" | "dark" | "light"(AppConfig.theme_mode)
 * @param cfg.theme_accent "blue" | "green" | "purple" | "orange"(AppConfig.theme_accent,存 token 名)
 */
export function apply_theme(cfg: { theme_mode: string; theme_accent: string }): void {
  // 强调色:覆盖在 :root 上(不在 .dark 选择器下),两主题共用同一色值
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
