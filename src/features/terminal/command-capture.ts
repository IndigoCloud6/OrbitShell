import type {Terminal} from '@xterm/xterm';
import {desktopCommand} from '../../shared/desktop';

// Read the command actually echoed by the terminal, including history recall and tab completion.
// Never infer commands from arbitrary output or collect raw password / interactive-program input.
function commandOnLine(terminal: Terminal, row: number): string | null {
  const buffer = terminal.buffer.active;
  if (buffer.type !== 'normal' || row < 0) return null;
  let first = row;
  while (first > 0 && buffer.getLine(first)?.isWrapped) first--;
  let text = '';
  for (let index = first; index <= row; index++) text += buffer.getLine(index)?.translateToString(index === row) ?? '';
  // Common sh/bash/zsh and PowerShell prompts. Unknown prompts are deliberately not logged.
  const prompt = /^(?:\([^\r\n)]*\)\s*)?(?:\[[^\]\r\n]+\]|[^\s@]+@[^\r\n]+?|[^\s:]+:[^\r\n]+?)[#$]\s?(.*)$/u.exec(text)
    ?? /^PS\s+[^\r\n>]+>\s?(.*)$/u.exec(text)
    ?? /^[#$%]\s+(.*)$/u.exec(text)
    ?? /^[\w.-]+(?:\([^\r\n)]+\))?#\s+(.*)$/u.exec(text)
    ?? /^(?:❯|➜)\s+(.*)$/u.exec(text);
  return prompt?.[1] ?? null;
}

export class CommandCapture {
  private pending: {id: string; until: number}[] = [];
  constructor(private readonly sessionId: string, private readonly onFailure: () => void) {}
  input(data: string | Uint8Array, terminal: Terminal | null) {
    if (!terminal || terminal.options.disableStdin || typeof data !== 'string' || terminal.buffer.active.type !== 'normal') return;
    const buffer = terminal.buffer.active;
    const current = commandOnLine(terminal, buffer.baseY + buffer.cursorY);
    if (data.startsWith('\x1b[200~') && data.endsWith('\x1b[201~')) {
      return;
    }
    const nativeEnter = /^(?:\r\n|\r|\n)$/.test(data);
    const singlePastedLine = !nativeEnter && /[^\r\n]+(?:\r\n|\r|\n)$/.test(data) && !/[\x00-\x1f\x7f]/.test(data.replace(/\r\n$|[\r\n]$/, ''));
    // Native Enter confirms a shell line. Embedded newlines in bracketed paste do not submit it.
    if ((!nativeEnter && !singlePastedLine) || current === null) return;
    this.pending.push({id: crypto.randomUUID(), until: Date.now() + 15000});
    this.pending = this.pending.slice(-64);
  }
  lineFeed(terminal: Terminal) {
    if (terminal.options.disableStdin || terminal.buffer.active.type !== 'normal') { this.pending = []; return; }
    this.pending = this.pending.filter(item => item.until > Date.now());
    const pending = this.pending.shift();
    if (!pending) return;
    const buffer = terminal.buffer.active;
    const echoed = commandOnLine(terminal, buffer.baseY + buffer.cursorY - 1);
    // A paste is only a submission intent. Persist the shell's visible echo, never
    // an unconfirmed input buffer (which may have gone to a hidden prompt).
    const command = echoed?.trim();
    if (!command) return;
    void desktopCommand('command_history_record', {sessionId: this.sessionId, id: pending.id, command}).catch(this.onFailure);
  }
  reset() { this.pending = []; }
}
