import {invoke, isTauri} from '@tauri-apps/api/core';

export const isDesktop = isTauri();

export function desktopCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktop) return Promise.reject(new Error('请在 OrbitShell 桌面端使用此功能。浏览器预览仅支持界面与本地连接配置。'));
  return invoke<T>(command, args);
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : '操作未完成，请重试';
}
