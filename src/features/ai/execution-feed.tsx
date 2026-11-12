import {createContext, useContext, useEffect, useRef, useSyncExternalStore} from 'react';
import type {ReactNode} from 'react';
import {listen} from '@tauri-apps/api/event';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';

export type ExecutionStatus = 'running' | 'completed' | 'failed' | 'timedOut' | 'truncated' | 'cancelled' | 'sent';
export interface ExecutionRecord {
  id: string; sessionId: string; sessionName: string; group: string; hostname: string; port: number; username: string;
  tool: 'execute_command' | 'send_input'; command: string | null; status: ExecutionStatus;
  startedAt: number; elapsedMs: number; exitCode: number | null; message: string;
}
interface Snapshot {revision: number; records: ExecutionRecord[]}
interface FeedSnapshot extends Snapshot {loading: boolean; error: string}
export interface ExecutionOutput {chunks: {stream: 'stdout' | 'stderr'; data: number[]}[]; cursor: number; hasMore: boolean}

class ExecutionFeed {
  private snapshot: FeedSnapshot = {revision: -1, records: [], loading: isDesktop, error: ''};
  private listeners = new Set<() => void>();
  private outputListeners = new Map<string, Set<() => void>>();
  private pendingOutputs = new Set<string>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private needsRecords = false;
  private refreshing = false;
  private refreshAgain = false;
  private disposed = false;

  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  subscribeOutput = (id: string, listener: () => void) => {
    let listeners = this.outputListeners.get(id);
    if (!listeners) { listeners = new Set(); this.outputListeners.set(id, listeners); }
    listeners.add(listener);
    return () => { listeners.delete(listener); if (!listeners.size) this.outputListeners.delete(id); };
  };
  private notify() { this.listeners.forEach(listener => listener()); }
  private schedule(records: boolean, outputId?: string) {
    this.needsRecords ||= records;
    if (outputId) this.pendingOutputs.add(outputId);
    if (this.timer || this.disposed) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      const ids = [...this.pendingOutputs]; this.pendingOutputs.clear();
      ids.forEach(id => this.outputListeners.get(id)?.forEach(listener => listener()));
      if (this.needsRecords) { this.needsRecords = false; void this.refresh(); }
    }, 100);
  }
  refresh = async () => {
    if (!isDesktop || this.disposed) return;
    if (this.refreshing) { this.refreshAgain = true; return; }
    this.refreshing = true;
    try {
      do {
        this.refreshAgain = false;
        const snapshot = await desktopCommand<Snapshot>('execution_list');
        if (this.disposed) return;
        if (snapshot.revision >= this.snapshot.revision) {
          this.snapshot = {...snapshot, loading: false, error: ''};
          this.notify();
          this.outputListeners.forEach(listeners => listeners.forEach(listener => listener()));
        }
      } while (this.refreshAgain);
    } catch (error) {
      if (!this.disposed) { this.snapshot = {...this.snapshot, loading: false, error: errorMessage(error)}; this.notify(); }
    } finally { this.refreshing = false; }
  };
  async start() {
    this.disposed = false;
    if (!isDesktop) return () => {};
    const stop = await listen<{revision: number; outputId: string | null}>('orbitshell:execution-changed', event => {
      if (event.payload.outputId) this.schedule(false, event.payload.outputId);
      else this.schedule(true);
    });
    if (this.disposed) { stop(); return () => {}; }
    void this.refresh();
    return stop;
  }
  fail(error: unknown) { this.snapshot = {...this.snapshot, loading: false, error: errorMessage(error)}; this.notify(); }
  clear = async () => { await desktopCommand('execution_clear'); await this.refresh(); };
  dispose() { this.disposed = true; if (this.timer) clearTimeout(this.timer); this.timer = null; }
}

const Context = createContext<ExecutionFeed | null>(null);
export function ExecutionProvider({children}: {children: ReactNode}) {
  const ref = useRef<ExecutionFeed | null>(null);
  if (!ref.current) ref.current = new ExecutionFeed();
  const feed = ref.current;
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void feed.start().then(unlisten => { if (disposed) unlisten(); else stop = unlisten; }).catch(error => { if (!disposed) feed.fail(error); });
    return () => { disposed = true; stop?.(); feed.dispose(); };
  }, [feed]);
  return <Context value={feed}>{children}</Context>;
}
export function useExecutionFeed() {
  const feed = useContext(Context);
  if (!feed) throw new Error('ExecutionProvider is missing');
  return feed;
}
export function useExecutions() {
  const feed = useExecutionFeed();
  return useSyncExternalStore(feed.subscribe, feed.getSnapshot);
}

export const executionLabels: Record<ExecutionStatus, string> = {
  running: '运行中', completed: '已完成', failed: '失败', timedOut: '已超时', truncated: '输出已截断', cancelled: '已中断', sent: '已发送',
};
export function formatElapsed(ms: number) { return ms < 1000 ? `${Math.max(0, Math.floor(ms))} ms` : `${(ms / 1000).toFixed(1)} s`; }
