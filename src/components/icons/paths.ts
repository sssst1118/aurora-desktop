/**
 * Phase6 AuroraIcon 图标库(设计稿 d:\2605\aurora-v02-preview.html 内联 SVG 直接移植)。
 * 每个条目是 <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"
 * stroke-linecap="round" stroke-linejoin="round"> 内部的元素片段,上述 stroke 属性由
 * AuroraIcon.vue 在 svg 根统一给出;片段内可局部覆盖(如 grip 圆点改实心填充)。
 */
const FOLDER_PATH =
  '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path>';

export const ICON_PATHS: Record<string, string> = {
  /* 搜索(放大镜) */
  search: '<circle cx="11" cy="11" r="7"></circle><path d="M20 20l-3.5-3.5"></path>',
  /* 设置(齿轮) */
  settings:
    '<circle cx="12" cy="12" r="3.2"></circle>' +
    '<path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.1-1.56 1.7 1.7 0 0 0-1.88.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.65 8.8a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34H9a1.7 1.7 0 0 0 1.03-1.56V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87V9a1.7 1.7 0 0 0 1.56 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.56 1.03z"></path>',
  /* 剪贴板 */
  clipboard:
    '<rect x="8" y="3" width="8" height="4" rx="1.2"></rect>' +
    '<path d="M16 5h2a1.5 1.5 0 0 1 1.5 1.5v13A1.5 1.5 0 0 1 18 21H6a1.5 1.5 0 0 1-1.5-1.5v-13A1.5 1.5 0 0 1 6 5h2"></path>',
  /* AI 助手(星芒) */
  ai:
    '<path d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9z"></path>' +
    '<path d="M19 15l.9 2.4L22 18l-2.1.9L19 21l-.9-2.1L16 18l2.1-.6z"></path>',
  /* 小桌面/抽屉(文件夹) */
  drawer: FOLDER_PATH,
  /* 关闭 */
  close: '<path d="M6 6l12 12M18 6L6 18"></path>',
  /* 复制 */
  copy:
    '<rect x="9" y="9" width="12" height="12" rx="2.2"></rect>' +
    '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>',
  /* 文件 */
  file:
    '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path>' +
    '<path d="M14 2v6h6"></path>',
  /* 文件夹(分类) */
  folder: FOLDER_PATH,
  /* 拖拽把手(三点,实心) */
  grip:
    '<circle cx="5" cy="12" r="1.6" fill="currentColor" stroke="none"></circle>' +
    '<circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none"></circle>' +
    '<circle cx="19" cy="12" r="1.6" fill="currentColor" stroke="none"></circle>',
  /* 删除 */
  trash:
    '<path d="M3 6h18"></path>' +
    '<path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>',
  /* 加号 */
  plus: '<path d="M12 5v14M5 12h14"></path>',
  /* 对勾 */
  check: '<path d="M20 6L9 17l-5-5"></path>',
};
