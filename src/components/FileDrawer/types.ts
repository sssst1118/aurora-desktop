/** 与 Rust 端 commands::drawer::DrawerFile 对应的前端类型 */
export interface DrawerFile {
  name: string;
  path: string;
  ext: string;
  is_dir: boolean;
}

/** 与 Rust 端 commands::drawer::DrawerGroup 对应的前端类型 */
export interface DrawerGroup {
  category: string;
  files: DrawerFile[];
}

/** 后端单次展示的条目上限(与 Rust 端 MAX_FILES 对齐,提示条用) */
export const MAX_FILES = 1500;

/** 分类 → emoji 图标(前端内置,不走后端) */
export const CATEGORY_ICONS: Record<string, string> = {
  文件夹: "📁",
  文档: "📄",
  图片: "🖼️",
  视频: "🎬",
  音频: "🎵",
  压缩: "🗜️",
  程序: "⚙️",
  代码: "💻",
  其他: "🧩",
};

/** 取条目图标:目录一律文件夹,文件按分类映射,未知分类兜底文档图标 */
export function iconOf(category: string, isDir: boolean): string {
  if (isDir) return "📁";
  return CATEGORY_ICONS[category] ?? "📄";
}
