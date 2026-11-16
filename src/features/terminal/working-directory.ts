import type {Terminal} from '@xterm/xterm';
import {commandOnLine} from './command-capture';

export function isDirectoryPromptReady(terminal: Terminal) {
  const buffer = terminal.buffer.active;
  if (buffer.type !== 'normal') return false;
  const row = buffer.baseY + buffer.cursorY;
  let first = row;
  while (first > 0 && buffer.getLine(first)?.isWrapped) {
    if (row - first >= 16) return false;
    first--;
  }
  let prompt = '';
  for (let index = first; index <= row; index++) prompt += buffer.getLine(index)?.translateToString(index === row) ?? '';
  if (prompt.length > 4096) return false;
  // The query uses POSIX builtins; never insert it into a PowerShell prompt.
  if (/^PS\s/.test(prompt)) return false;
  return commandOnLine(terminal, row) === '' || /^[#$%❯➜]$/.test(prompt);
}

export function validateTerminalDirectory(path: string) {
  if (!path.startsWith('/') || path.length > 4096 || /[\x00-\x1f\x7f-\x9f]/.test(path)) {
    throw new Error('终端未返回有效的绝对目录，文件面板保持原位置。');
  }
  return path;
}

export function directoryQuery(id: string) {
  // No user-provided path or command is interpolated. The command substitution
  // inherits this exact shell's cwd; a fresh SSH exec channel would not.
  return ` command printf '\\033]633;OrbitShellCwd;${id};%s\\007' "$(command pwd -P)"\r`;
}
