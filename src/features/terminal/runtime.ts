import {Channel} from '@tauri-apps/api/core';
import type {Terminal} from '@xterm/xterm';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import type {ConnectionProfile} from '../connections/types';
import {CommandCapture} from './command-capture';
import {directoryQuery, isDirectoryPromptReady, validateTerminalDirectory} from './working-directory';

export type SessionStatus = 'connecting' | 'hostKey' | 'connected' | 'closed' | 'error';
export interface HostKey {hostname: string; port: number; fingerprint: string; algorithm: string}
export type SessionEvent =
  | {type: 'status'; status: 'connecting' | 'connected'; message: string}
  | ({type: 'hostKey'} & HostKey)
  | {type: 'data'; data: number[]}
  | {type: 'exit'; code: number | null; message: string}
  | {type: 'error'; message: string}
  | {type: 'credentialWarning'; message: string};

export interface AuthenticationOptions {useSavedCredential?: boolean; rememberCredential?: boolean}

export interface ShellSession {
  id: string;
  profile: ConnectionProfile;
  status: SessionStatus;
  message: string;
  hostKey: HostKey | null;
  credentialWarning?: string;
  ephemeral?: boolean;
  columns: number;
  rows: number;
  directoryQueryPending?: boolean;
}

type Writer = (bytes: Uint8Array, done: () => void) => void;

/** Owns the byte stream; React only receives small session metadata changes. */
export class TerminalRuntime {
  readonly id: string;
  terminal: Terminal | null = null;
  private writer: Writer | null = null;
  private pending: Uint8Array[] = [];
  private disposed = false;
  private connected = false;
  private columns = 120;
  private rows = 36;
  private inputQueue: Promise<void> = Promise.resolve();
  private ackBytes = 0;
  private ackTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly events: Channel<SessionEvent> | null;
  private readonly capture: CommandCapture;
  private inputUnsettled = false;
  private draftSeen = false;
  private waitingForPrompt = false;
  private promptLineFeedSeen = false;
  private directoryStarting = false;
  private directoryRequest: {id: string; resolve: (path: string) => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout>} | null = null;

  constructor(id: string, private readonly update: (patch: Partial<ShellSession>) => void) {
    this.id = id;
    this.capture = new CommandCapture(id, () => this.update({message: '命令已发送，但本地执行历史记录失败'}));
    this.events = isDesktop ? new Channel<SessionEvent>() : null;
    if (this.events) this.events.onmessage = event => this.receive(event);
  }

  receive(event: SessionEvent) {
    if (this.disposed) return;
    if (event.type === 'data') {
      const bytes = Uint8Array.from(event.data);
      if (this.writer) this.write(bytes); else this.pending.push(bytes);
      return;
    }
    if (event.type === 'credentialWarning') {
      this.update({credentialWarning: event.message});
      return;
    }
    if (event.type === 'hostKey') {
      this.update({status: 'hostKey', message: '等待确认服务器指纹', hostKey: event});
    } else if (event.type === 'status') {
      this.connected = event.status === 'connected';
      this.update({status: event.status, message: event.message, hostKey: null});
      if (this.connected) void this.resize(this.columns, this.rows);
    } else {
      this.connected = false;
      this.finishDirectory(new Error('终端已断开，无法定位目录。'));
      this.update({status: event.type === 'error' ? 'error' : 'closed', message: event.message, hostKey: null});
      const safe = event.message.replace(/[\x00-\x1f\x7f]/g, ' ');
      const bytes = new TextEncoder().encode(`\r\n[OrbitShell] ${safe}\r\n`);
      if (this.writer) this.writer(bytes, () => {}); else this.pending.push(bytes);
    }
  }

  async connect(profile: ConnectionProfile, credential: string, ephemeral = false, options: AuthenticationOptions = {}) {
    try {
      if (!this.events) throw new Error('SSH 连接需要在 OrbitShell 桌面端打开。');
      await desktopCommand('ssh_connect', {
        request: {sessionId: this.id, profileId: profile.id, profile: ephemeral ? profile : null, credential,
          useSavedCredential: !ephemeral && Boolean(options.useSavedCredential),
          rememberCredential: !ephemeral && Boolean(options.rememberCredential),
          columns: this.columns, rows: this.rows},
        events: this.events,
      });
      if (this.disposed) await desktopCommand('ssh_disconnect', {sessionId: this.id});
    } catch (error) {
      this.receive({type: 'error', message: errorMessage(error)});
    }
  }

  attach(writer: Writer) {
    this.writer = writer;
    const lineFeed = this.terminal?.onLineFeed(() => { if (this.waitingForPrompt) this.promptLineFeedSeen = true; if (this.terminal) this.capture.lineFeed(this.terminal); });
    const directory = this.terminal?.parser.registerOscHandler(633, data => {
      if (!data.startsWith('OrbitShellCwd;')) return false;
      const request = this.directoryRequest;
      if (request && data.startsWith(`OrbitShellCwd;${request.id};`)) {
        try { this.finishDirectory(validateTerminalDirectory(data.slice(`OrbitShellCwd;${request.id};`.length))); }
        catch (failure) { this.finishDirectory(new Error(errorMessage(failure))); }
      }
      return true;
    });
    for (const bytes of this.pending.splice(0)) this.write(bytes);
    return () => { lineFeed?.dispose(); directory?.dispose(); this.finishDirectory(new Error('终端视图已切换，请重新定位目录。')); this.capture.reset(); this.writer = null; };
  }

  private write(bytes: Uint8Array) {
    this.writer?.(bytes, () => {
      const terminal = this.terminal;
      if (terminal && (this.waitingForPrompt || this.inputUnsettled || this.draftSeen)) {
        if (this.waitingForPrompt) {
          if (this.promptLineFeedSeen && isDirectoryPromptReady(terminal)) { this.waitingForPrompt = false; this.inputUnsettled = false; this.draftSeen = false; }
        } else if (isDirectoryPromptReady(terminal)) {
          if (this.draftSeen) { this.inputUnsettled = false; this.draftSeen = false; }
        } else { this.draftSeen = true; this.inputUnsettled = false; }
      }
      this.ackBytes += bytes.byteLength;
      if (this.ackTimer || this.disposed) return;
      this.ackTimer = setTimeout(() => {
        const count = this.ackBytes;
        this.ackBytes = 0;
        this.ackTimer = null;
        if (!this.disposed) void desktopCommand('ssh_acknowledge', {sessionId: this.id, bytes: count}).catch(() => {});
      }, 16);
    });
  }

  input(data: string | Uint8Array) {
    if (!this.connected || this.disposed) return;
    if (this.directoryRequest) return;
    this.capture.input(data, this.terminal);
    void this.queueInput(data).catch(error => {
      this.capture.reset();
      const message = errorMessage(error);
      if (message.includes('AI_SESSION_OWNED')) this.update({message: '当前终端由 AI 接管，请先交回人工'});
      else this.receive({type: 'error', message});
    });
  }

  async sendCommand(command: string, appendNewline: boolean) {
    if (!this.connected || this.disposed) throw new Error('当前终端尚未连接，指令未发送');
    const data = command.replace(/\r\n|\n/g, '\r');
    await this.queueInput(appendNewline ? `${data.replace(/\r+$/, '')}\r` : data, appendNewline ? command : undefined);
  }

  private queueInput(data: string | Uint8Array, submittedCommand?: string, trackPrompt = true): Promise<void> {
    if (trackPrompt && this.directoryRequest) return Promise.reject(new Error('正在查询终端目录，请稍后发送指令。'));
    const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
    if (trackPrompt && bytes.length) {
      this.inputUnsettled = true;
      if (bytes.includes(13) || bytes.includes(10) || bytes.includes(3)) { this.waitingForPrompt = true; this.promptLineFeedSeen = false; }
    }
    const historyId = submittedCommand ? crypto.randomUUID() : null;
    // Keep each paste/shortcut ordered and bound to this runtime, even after tab switches.
    const queued = this.inputQueue.then(async () => {
      for (let offset = 0; offset < bytes.length; offset += 32 * 1024) {
        if (this.disposed || !this.connected) throw new Error('终端已断开，剩余输入未发送');
        const last = offset + 32 * 1024 >= bytes.length;
        await desktopCommand('ssh_write', {sessionId: this.id, data: Array.from(bytes.subarray(offset, offset + 32 * 1024)),
          historyCommand: last ? submittedCommand ?? null : null, historyId: last ? historyId : null});
      }
    });
    this.inputQueue = queued.catch(() => {});
    return queued;
  }

  async requestWorkingDirectory(): Promise<string> {
    if (this.directoryStarting || this.directoryRequest) throw new Error('正在查询终端目录，请稍候。');
    this.directoryStarting = true;
    try {
      await this.inputQueue;
      const terminal = this.terminal;
      if (!this.connected || this.disposed || !terminal) throw new Error('终端尚未连接，无法定位目录。');
      if (terminal.options.disableStdin) throw new Error('当前终端由 AI 接管或暂不可输入，请先交回人工。');
      if (this.inputUnsettled || this.waitingForPrompt || !isDirectoryPromptReady(terminal)) throw new Error('请先等待命令结束，并回到没有未提交内容的 Shell 提示符后再定位。');
      this.capture.reset();
      return await new Promise<string>((resolve, reject) => {
        const id = crypto.randomUUID().replaceAll('-', '');
        const timer = setTimeout(() => this.finishDirectory(new Error('终端目录查询超时；当前 Shell 可能不支持此查询，文件面板保持原位置。')), 8000);
        this.directoryRequest = {id, resolve, reject, timer};
        this.update({directoryQueryPending: true});
        void this.queueInput(directoryQuery(id), undefined, false).catch(failure => this.finishDirectory(new Error(errorMessage(failure))));
      });
    } finally { this.directoryStarting = false; }
  }

  private finishDirectory(result: string | Error) {
    const request = this.directoryRequest;
    if (!request) return;
    this.directoryRequest = null; clearTimeout(request.timer);
    this.update({directoryQueryPending: false});
    if (result instanceof Error) request.reject(result); else request.resolve(result);
  }

  async resize(columns: number, rows: number) {
    this.columns = columns;
    this.rows = rows;
    this.update({columns, rows});
    if (this.connected && !this.disposed) {
      await desktopCommand('ssh_resize', {sessionId: this.id, columns, rows}).catch(() => {});
    }
  }

  dispose() {
    this.disposed = true;
    this.connected = false;
    this.finishDirectory(new Error('会话已关闭，无法定位目录。'));
    if (this.ackTimer) clearTimeout(this.ackTimer);
    this.writer = null;
    this.pending = [];
    void desktopCommand('ssh_disconnect', {sessionId: this.id}).catch(() => {});
  }
}
