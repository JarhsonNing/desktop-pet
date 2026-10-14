declare global {
  interface Window {
    __TAURI__?: {
      core: {
        invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
      };
      event: {
        listen<T>(
          name: string,
          handler: (event: { payload: T }) => void,
        ): Promise<() => void>;
      };
    };
  }
}
export async function invoke<T = unknown>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (!window.__TAURI__)
    throw new Error("请从桌宠打开面板，本页面需要原生服务。");
  return window.__TAURI__.core.invoke<T>(command, args);
}
export function call<T = unknown>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  return invoke<T>("host", { command, args });
}
export async function listen<T>(
  name: string,
  callback: (value: T) => void,
): Promise<() => void> {
  return (
    window.__TAURI__?.event.listen<T>(name, (event) =>
      callback(event.payload),
    ) ?? (() => {})
  );
}
