import { ref } from "vue";

/**
 * 最近打开(应用 + 文件,SearchView 空 query 态展示;设计文档 §6:文件经 open_item
 * 打开后同样记入,与预览稿「最近打开」应用/文件双组对应)。
 * localStorage 持久化,最多 10 条,按最近打开倒序;模块级单例,重挂载不丢内存态。
 * kind 为 Phase6 新增字段(老数据无此字段,回退按应用展示)。
 */
export interface RecentApp {
  name: string;
  path: string;
  kind?: "app" | "file";
}

const STORAGE_KEY = "aurora-recent-apps";
const MAX_RECENT = 10;

const recents = ref<RecentApp[]>([]);

function isValid(v: unknown): v is RecentApp {
  return (
    typeof v === "object" &&
    v !== null &&
    typeof (v as RecentApp).name === "string" &&
    typeof (v as RecentApp).path === "string"
  );
}

/** 从 localStorage 读回(损坏/非法条目静默丢弃) */
function loadRecents() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    const parsed: unknown = JSON.parse(raw);
    recents.value = Array.isArray(parsed)
      ? parsed.filter(isValid).slice(0, MAX_RECENT)
      : [];
  } catch (e) {
    console.error("load recent apps failed", e);
    recents.value = [];
  }
}

/** 记录一次打开:同路径去重后置顶,截断到上限 */
function saveRecent(app: RecentApp) {
  const next = [app, ...recents.value.filter((r) => r.path !== app.path)].slice(
    0,
    MAX_RECENT,
  );
  recents.value = next;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch (e) {
    console.error("save recent apps failed", e);
  }
}

export function useRecentApps() {
  return { recents, loadRecents, saveRecent };
}
