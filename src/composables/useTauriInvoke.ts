import { invoke } from "@tauri-apps/api/core";

/** 统一 Tauri invoke 封装:参数对象可选,失败时打印错误后继续抛出 */
export function useTauriInvoke<T>(cmd: string) {
  const call = async (args?: Record<string, unknown>): Promise<T> => {
    try {
      return (await invoke<T>(cmd, args)) as T;
    } catch (e) {
      console.error(`invoke ${cmd} failed:`, e);
      throw e;
    }
  };
  return { call };
}
