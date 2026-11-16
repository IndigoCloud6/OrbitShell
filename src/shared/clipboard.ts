import {desktopCommand, isDesktop} from './desktop';

/** Desktop clipboard access must not depend on WebView focus or browser permissions. */
export async function readClipboardText(): Promise<string> {
  if (isDesktop) {
    const text = await desktopCommand<string | null>('clipboard_read_text');
    // Other desktop platforms retain the browser path until a native adapter is available.
    if (text !== null) return text;
  }
  try {
    return await navigator.clipboard.readText();
  } catch {
    throw new Error('无法读取系统剪贴板，请将窗口切回前台，重新复制文本后再粘贴。');
  }
}

export async function writeClipboardText(text: string): Promise<void> {
  if (isDesktop && await desktopCommand<boolean>('clipboard_write_text', {text})) return;
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    throw new Error('无法写入系统剪贴板，请将窗口切回前台后重试复制。');
  }
}
