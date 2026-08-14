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

/* 2026-08-14 emoji 清剿:原 CATEGORY_ICONS 分类 emoji 与 iconOf 已删——
   全 src 无消费方(grep 确认),分类图标由 FileItem.vue 各自渲染(真实图标/首字母占位)。 */
