import {createContext, useContext, useEffect, useRef, useSyncExternalStore} from 'react';
import type {ReactNode} from 'react';
import type {Terminal} from '@xterm/xterm';
import {listen} from '@tauri-apps/api/event';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';

export interface AiAgent {id: string; name: string; version: string; connected: boolean; connectedAt: number}
export interface AiTerminalInfo {
  mode: 'dedicated' | 'takeover';
  id: string; agentId: string; agentName: string; sessionId: string; sessionName: string; hostname: string; port: number; username: string;
  status: 'connecting' | 'idle' | 'running' | 'interactive' | 'closed' | 'error'; message: string; command: string | null;
  startedAt: number | null; elapsedMs: number; exitCode: number | null; updatedAt: number;
}
interface Snapshot {revision: number; agents: AiAgent[]; terminals: AiTerminalInfo[]}
interface FeedSnapshot extends Snapshot {loading: boolean; error: string; unread: ReadonlySet<string>}
interface Output {data: number[]; cursor: number; truncated: boolean}
type Writer = (bytes: Uint8Array, done: () => void) => void;

/** Byte buffers stay in Rust and xterm, never in React state or query caches. */
class AiTerminalRuntime {
  terminal: Terminal | null = null;
  follow = true;
  private generation = 0;
  private writer: Writer | null = null;
  private wake: (() => void) | null = null;
  private finishWrite: (() => void) | null = null;
  constructor(readonly id: string, private readonly feed: AiWorkspaceFeed) {}
  input() { /* This view intentionally never sends human input to an Agent's shell. */ }
  async resize(columns: number, rows: number) {
    if (isDesktop) await desktopCommand('ai_terminal_resize', {id: this.id, columns, rows}).catch(() => {});
  }
  attach(writer: Writer) {
    const generation = ++this.generation;
    this.writer = writer;
    let cursor: number | undefined;
    let reading = false;
    let again = false;
    const write = (data: Uint8Array) => new Promise<void>(resolve => {
      this.finishWrite = resolve;
      writer(data, () => {
        if (this.finishWrite === resolve) this.finishWrite = null;
        if (this.follow && generation === this.generation) this.terminal?.scrollToBottom();
        resolve();
      });
    });
    const drain = async () => {
      if (reading) { again = true; return; }
      if (!isDesktop || !this.writer || generation !== this.generation) return;
      reading = true;
      try {
        do {
          again = false;
          const output = await desktopCommand<Output>('ai_terminal_output', {id: this.id, cursor: cursor ?? null});
          if (generation !== this.generation) return;
          if (output.truncated) await write(new TextEncoder().encode('\r\n[OrbitShell] 部分较早输出已超出内存保留范围。\r\n'));
          if (generation !== this.generation) return;
          if (output.data.length) await write(Uint8Array.from(output.data));
          cursor = output.cursor;
          again ||= output.data.length === 65536;
        } while (again && generation === this.generation);
      } catch (error) {
        if (generation === this.generation) this.feed.fail(error);
      } finally { reading = false; }
    };
    this.wake = () => { void drain(); };
    void drain();
    return () => {
      if (generation !== this.generation) return;
      this.generation++; this.writer = null; this.wake = null;
      this.finishWrite?.(); this.finishWrite = null;
    };
  }
  outputChanged() { this.wake?.(); }
}

class AiWorkspaceFeed {
  private snapshot: FeedSnapshot = {revision: -1, agents: [], terminals: [], loading: isDesktop, error: '', unread: new Set()};
  private listeners = new Set<() => void>();
  private runtimes = new Map<string, AiTerminalRuntime>();
  private pending = new Set<string>();
  private metadata = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private refreshing = false;
  private refreshAgain = false;
  private generation = 0;
  private viewed: string | null = null;
  private running = false;
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private notify() { this.listeners.forEach(listener => listener()); }
  getRuntime(id: string) {
    let runtime = this.runtimes.get(id);
    if (!runtime) { runtime = new AiTerminalRuntime(id, this); this.runtimes.set(id, runtime); }
    return runtime;
  }
  view(id: string | null) {
    this.viewed = id;
    if (id && this.snapshot.unread.has(id)) {
      const unread = new Set(this.snapshot.unread); unread.delete(id);
      this.snapshot = {...this.snapshot, unread}; this.notify();
    }
  }
  private schedule(outputId: string | null) {
    if (outputId) this.pending.add(outputId); else this.metadata = true;
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      const unread = new Set(this.snapshot.unread);
      for (const id of this.pending) {
        this.runtimes.get(id)?.outputChanged();
        if (id !== this.viewed) unread.add(id);
      }
      this.pending.clear();
      if (unread.size !== this.snapshot.unread.size) { this.snapshot = {...this.snapshot, unread}; this.notify(); }
      if (this.metadata) { this.metadata = false; void this.refresh(); }
    }, 100);
  }
  refresh = async () => {
    if (!isDesktop || !this.running) return;
    if (this.refreshing) { this.refreshAgain = true; return; }
    const generation = this.generation;
    this.refreshing = true;
    try {
      do {
        this.refreshAgain = false;
        const snapshot = await desktopCommand<Snapshot>('ai_workspace');
        if (generation !== this.generation) return;
        if (snapshot.revision >= this.snapshot.revision) {
          const ids = new Set(snapshot.terminals.map(t => t.id));
          this.snapshot = {...snapshot, loading: false, error: '', unread: new Set([...this.snapshot.unread].filter(id => ids.has(id)))}; this.notify();
          for (const [id, runtime] of this.runtimes) { if (!ids.has(id)) this.runtimes.delete(id); else runtime.outputChanged(); }
        }
      } while (this.refreshAgain);
    } catch (error) { if (generation === this.generation) this.fail(error); }
    finally {
      this.refreshing = false;
      if (this.refreshAgain && this.running && generation !== this.generation) void this.refresh();
    }
  };
  fail(error: unknown) { this.snapshot = {...this.snapshot, loading: false, error: errorMessage(error)}; this.notify(); }
  start() {
    this.running = true;
    const generation = ++this.generation;
    let stop: (() => void) | undefined;
    if (isDesktop) {
      void listen<{outputId: string | null}>('orbitshell:ai-changed', event => this.schedule(event.payload.outputId)).then(unlisten => {
        if (generation !== this.generation) unlisten(); else { stop = unlisten; void this.refresh(); }
      }).catch(error => { if (generation === this.generation) this.fail(error); });
      void this.refresh();
    }
    // A small metadata refresh also recovers from an interrupted desktop event subscription.
    const interval = isDesktop ? setInterval(() => { void this.refresh(); }, 5000) : null;
    return () => { this.running = false; this.generation++; stop?.(); if (interval) clearInterval(interval); if (this.timer) clearTimeout(this.timer); this.timer = null; this.pending.clear(); this.metadata = false; };
  }
}

const Context = createContext<AiWorkspaceFeed | null>(null);
export function AiWorkspaceProvider({children}: {children: ReactNode}) {
  const ref = useRef<AiWorkspaceFeed | null>(null);
  if (!ref.current) ref.current = new AiWorkspaceFeed();
  const feed = ref.current;
  useEffect(() => feed.start(), [feed]);
  return <Context value={feed}>{children}</Context>;
}
export function useAiWorkspaceFeed() {
  const feed = useContext(Context);
  if (!feed) throw new Error('AiWorkspaceProvider is missing');
  return feed;
}
export function useAiWorkspace() {
  const feed = useAiWorkspaceFeed();
  return useSyncExternalStore(feed.subscribe, feed.getSnapshot);
}
export const aiStatusLabels: Record<AiTerminalInfo['status'], string> = {
  connecting: '正在连接', idle: '空闲', running: '运行中', interactive: '交互中', closed: '已断开', error: '连接失败',
};
