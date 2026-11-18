import {createContext, useCallback, useContext, useEffect, useRef, useState} from 'react';
import type {ReactNode} from 'react';
import {Channel} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {UploadConflictDialog} from './upload-conflict-dialog';
import type {UploadConflict} from './upload-conflict-dialog';

export interface Transfer extends Record<string, unknown> {
  id: string; name: string; direction: 'upload' | 'download';
  status: 'choosing' | 'running' | 'done' | 'error' | 'cancelled';
  bytes: number; total: number | null; speed: number; message: string;
  agentId?: string | null; agentName?: string | null; sessionId?: string; sessionName?: string;
  hostname?: string; port?: number; temporary?: boolean; restored?: boolean;
  localPath?: string; remotePath?: string; createdAt?: number; finishedAt?: number | null; elapsedMs?: number;
}
interface Snapshot {records: Transfer[]; warning: string | null}
const transferKey = ['file-transfers'] as const;
function active(row: Transfer) { return row.status === 'choosing' || row.status === 'running'; }
interface Progress {name: string; bytes: number; total: number | null}
type UploadEvent = {type: 'progress'; progress: Progress} | {type: 'conflict'; conflict: Omit<UploadConflict, 'transferId'>};
interface TransferResult {name: string; bytes: number}
interface Request {sessionId: string; direction: 'upload' | 'download'; path: string; name?: string; localPath?: string; drag?: boolean}
interface ContextValue {transfers: Transfer[]; records: Transfer[]; warning: string; run: (request: Request) => Promise<boolean>; clearCompleted: () => void}
const Context = createContext<ContextValue | null>(null);

export function TransferProvider({children}: {children: ReactNode}) {
  const [transfers, setTransfers] = useState<Transfer[]>([]);
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set());
  const [feedError, setFeedError] = useState('');
  const [conflicts, setConflicts] = useState<UploadConflict[]>([]);
  const conflictRef = useRef(conflicts); conflictRef.current = conflicts;
  useEffect(() => () => {
    for (const conflict of conflictRef.current) void desktopCommand('sftp_confirm_upload', {token: conflict.token, overwrite: false}).catch(() => {});
  }, []);
  const completed = useRef(new Set<string>());
  const queryClient = useQueryClient();
  const history = useQuery({queryKey: transferKey, enabled: isDesktop,
    queryFn: () => desktopCommand<Snapshot>('file_transfer_list'), retry: false, refetchOnWindowFocus: true,
    refetchInterval: query => query.state.data?.records.some(active) ? 2000 : false, refetchIntervalInBackground: true});
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    let off: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => { void queryClient.invalidateQueries({queryKey: transferKey}); };
    void listen('orbitshell:file-transfers-changed', () => {
      if (disposed || timer) return;
      timer = setTimeout(() => { timer = undefined; if (!disposed) refresh(); }, 100);
    }).then(unlisten => {
      if (disposed) unlisten(); else { off = unlisten; setFeedError(''); refresh(); }
    }).catch(error => { if (!disposed) setFeedError(errorMessage(error)); });
    return () => { disposed = true; off?.(); if (timer) clearTimeout(timer); };
  }, [queryClient]);
  useEffect(() => {
    const rows = history.data?.records;
    if (!rows) return;
    const ids = new Set(rows.map(row => row.id));
    setTransfers(previous => previous.some(row => ids.has(row.id)) ? previous.filter(row => !ids.has(row.id)) : previous);
    for (const row of rows) {
      if (row.status === 'done' && row.direction === 'upload' && !row.restored && row.sessionId && !completed.current.has(row.id)) {
        // Invalidate only the originating session, never whichever host is currently selected.
        void queryClient.invalidateQueries({queryKey: ['remote-directory', row.sessionId]});
      }
    }
    completed.current = new Set(rows.filter(row => row.status === 'done').map(row => row.id));
  }, [history.data, queryClient]);
  const run = useCallback(async (request: Request) => {
    const id = crypto.randomUUID();
    let sampleAt = performance.now();
    let sampleBytes = 0;
    let finished = false;
    setTransfers(previous => [...previous.filter(row => row.status === 'choosing' || row.status === 'running'),
      ...previous.filter(row => row.status !== 'choosing' && row.status !== 'running').slice(-19),
      {id, name: request.name ?? '选择上传文件…', direction: request.direction, status: 'choosing', bytes: 0, total: null, speed: 0,
        message: request.drag ? '等待选择拖放目标' : request.localPath ? '正在准备上传' : '等待选择文件', sessionId: request.sessionId, createdAt: Date.now()}]);
    const progress = new Channel<Progress | UploadEvent>();
    progress.onmessage = event => {
      if (finished) return;
      if ('type' in event && event.type === 'conflict') {
        const conflict = {...event.conflict, transferId: id};
        setConflicts(previous => previous.some(item => item.token === conflict.token) ? previous : [...previous, conflict]);
        return;
      }
      const update = 'type' in event ? event.progress : event;
      const now = performance.now();
      const speed = (update.bytes - sampleBytes) / Math.max((now - sampleAt) / 1000, 0.05);
      sampleAt = now; sampleBytes = update.bytes;
      setTransfers(previous => previous.map(row => row.id === id ? {...row, ...update, speed, status: 'running', message: '传输中'} : row));
    };
    try {
      const result = await desktopCommand<TransferResult | null>(request.direction === 'upload' ? 'sftp_upload' : request.drag ? 'sftp_drag_download' : 'sftp_download',
        request.direction === 'upload' ? {sessionId: request.sessionId, transferId: id, directory: request.path, localPath: request.localPath ?? null, progress} : {sessionId: request.sessionId, transferId: id, path: request.path, progress});
      if (!result) { setTransfers(previous => previous.filter(row => row.id !== id)); return false; }
      setTransfers(previous => previous.map(row => row.id === id ? {...row, name: result.name, bytes: result.bytes, total: result.bytes, status: 'done', message: '已完成'} : row));
      return true;
    } catch (error) {
      setTransfers(previous => previous.map(row => row.id === id ? {...row, status: 'error', message: errorMessage(error)} : row));
      return false;
    } finally { finished = true; setConflicts(previous => previous.filter(conflict => conflict.transferId !== id)); }
  }, []);
  const native = history.data?.records ?? [];
  const nativeIds = new Set(native.map(row => row.id));
  const records = [...native, ...transfers.filter(row => !nativeIds.has(row.id))].sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  const visible = records.filter(row => active(row) || (!row.restored && !hidden.has(row.id)));
  const queue = [...visible.filter(active), ...visible.filter(row => !active(row)).slice(-20)];
  const clearCompleted = () => {
    // Clearing the queue does not erase persisted transfer history or touch any files.
    setHidden(new Set(records.filter(row => !active(row)).map(row => row.id)));
    setTransfers(previous => previous.filter(active));
  };
  const warning = [history.error ? errorMessage(history.error) : '', history.data?.warning, feedError].filter(Boolean).join('；');
  return <Context value={{transfers: queue, records, warning, run, clearCompleted}}>{children}
    {conflicts[0] && <UploadConflictDialog key={conflicts[0].token} conflict={conflicts[0]} onAnswered={token => setConflicts(previous => previous.filter(item => item.token !== token))} />}
  </Context>;
}

export function useTransfers() {
  const value = useContext(Context);
  if (!value) throw new Error('TransferProvider is missing');
  return value;
}
