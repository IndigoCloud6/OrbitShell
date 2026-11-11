import {Channel} from '@tauri-apps/api/core';
import type {Terminal} from '@xterm/xterm';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import type {ConnectionProfile} from '../connections/types';

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
  columns: number;
  rows: number;
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

  constructor(id: string, private readonly update: (patch: Partial<ShellSession>) => void) {
    this.id = id;
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
    for (const bytes of this.pending.splice(0)) this.write(bytes);
    return () => { this.writer = null; };
  }

  private write(bytes: Uint8Array) {
    this.writer?.(bytes, () => {
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
    const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
    // A large paste is split into ordered chunks; individual key presses preserve order as well.
    for (let offset = 0; offset < bytes.length; offset += 32 * 1024) {
      const chunk = Array.from(bytes.subarray(offset, offset + 32 * 1024));
      this.inputQueue = this.inputQueue.then(async () => {
        if (!this.disposed && this.connected) await desktopCommand('ssh_write', {sessionId: this.id, data: chunk});
      }).catch(error => this.receive({type: 'error', message: errorMessage(error)}));
    }
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
    if (this.ackTimer) clearTimeout(this.ackTimer);
    this.writer = null;
    this.pending = [];
    void desktopCommand('ssh_disconnect', {sessionId: this.id}).catch(() => {});
  }
}
