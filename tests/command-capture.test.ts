import {beforeEach, describe, expect, it, vi} from 'vitest';
import type {Terminal} from '@xterm/xterm';
import {CommandCapture} from '../src/features/terminal/command-capture';
import {desktopCommand} from '../src/shared/desktop';

vi.mock('../src/shared/desktop', () => ({desktopCommand: vi.fn().mockResolvedValue(undefined)}));

// Only the terminal buffer contract is simulated; no SSH or saved configuration is used.
function screen(lines: string[], cursorY: number, type = 'normal', disabled = false): Terminal {
  return {options: {disableStdin: disabled}, buffer: {active: {type, baseY: 0, cursorY,
    getLine: (index: number) => lines[index] === undefined ? undefined : {isWrapped: false, translateToString: () => lines[index]},
  }}} as unknown as Terminal;
}

beforeEach(() => vi.clearAllMocks());
describe('human command capture', () => {
  it('records the echoed command after recall or completion, rather than raw keystrokes', () => {
    const capture = new CommandCapture('session', vi.fn());
    capture.input('\r', screen(['demo@host:~$ git status'], 0));
    capture.lineFeed(screen(['demo@host:~$ git status', ''], 1));
    expect(desktopCommand).toHaveBeenCalledWith('command_history_record', expect.objectContaining({command: 'git status', sessionId: 'session'}));
  });
  it('never persists an unconfirmed paste to a hidden prompt', () => {
    const capture = new CommandCapture('session', vi.fn());
    capture.input('private-answer\r', screen(['demo@host:~$ '], 0));
    capture.lineFeed(screen(['Password:', ''], 1));
    expect(desktopCommand).not.toHaveBeenCalled();
  });
  it('does not record password, full-screen or AI-owned terminal input', () => {
    for (const terminal of [screen(['Password:'], 0), screen(['# echo test'], 0, 'alternate'), screen(['demo@host:~$ echo test'], 0, 'normal', true)]) {
      const capture = new CommandCapture('session', vi.fn());
      capture.input('\r', terminal);
      capture.lineFeed(screen(['demo@host:~$ echo test', ''], 1));
    }
    expect(desktopCommand).not.toHaveBeenCalled();
  });
  it('bracketed paste is not submitted until Enter and only its echo is saved', () => {
    const capture = new CommandCapture('session', vi.fn());
    capture.input('\x1b[200~echo hello\x1b[201~', screen(['demo@host:~$ '], 0));
    capture.lineFeed(screen(['demo@host:~$ echo hello', ''], 1));
    expect(desktopCommand).not.toHaveBeenCalled();
    capture.input('\r', screen(['demo@host:~$ echo hello'], 0));
    capture.lineFeed(screen(['demo@host:~$ echo hello', ''], 1));
    expect(desktopCommand).toHaveBeenCalledOnce();
  });
  it('drops rejected or expired submission intents', () => {
    const capture = new CommandCapture('session', vi.fn());
    capture.input('\r', screen(['demo@host:~$ echo test'], 0));
    capture.reset();
    capture.lineFeed(screen(['demo@host:~$ echo test', ''], 1));
    const time = vi.spyOn(Date, 'now').mockReturnValue(0);
    capture.input('\r', screen(['demo@host:~$ echo test'], 0));
    time.mockReturnValue(15001);
    capture.lineFeed(screen(['demo@host:~$ echo test', ''], 1));
    time.mockRestore();
    expect(desktopCommand).not.toHaveBeenCalled();
  });
});
