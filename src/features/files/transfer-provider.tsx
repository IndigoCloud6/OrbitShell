import {createContext, useCallback, useContext, useState} from 'react';
import type {ReactNode} from 'react';
import {Channel} from '@tauri-apps/api/core';
import {desktopCommand, errorMessage} from '../../shared/desktop';

export interface Transfer extends Record<string, unknown> {
  id: string; name: string; direction: 'upload' | 'download';
  status: 'choosing' | 'running' | 'done' | 'error';
  bytes: number; total: number | null; speed: number; message: string;
}
interface Progress {name: string; bytes: number; total: number | null}
interface TransferResult {name: string; bytes: number}
interface Request {sessionId: string; direction: 'upload' | 'download'; path: string; name?: string}
interface ContextValue {transfers: Transfer[]; run: (request: Request) => Promise<boolean>; clearCompleted: () => void}
const Context = createContext<ContextValue | null>(null);

export function TransferProvider({children}: {children: ReactNode}) {
  const [transfers, setTransfers] = useState<Transfer[]>([]);
  const run = useCallback(async (request: Request) => {
    const id = crypto.randomUUID();
    let sampleAt = performance.now();
    let sampleBytes = 0;
    setTransfers(previous => [...previous.filter(row => row.status === 'choosing' || row.status === 'running'),
      ...previous.filter(row => row.status !== 'choosing' && row.status !== 'running').slice(-19),
      {id, name: request.name ?? '选择上传文件…', direction: request.direction, status: 'choosing', bytes: 0, total: null, speed: 0, message: '等待选择文件'}]);
    const progress = new Channel<Progress>();
    progress.onmessage = event => {
      const now = performance.now();
      const speed = (event.bytes - sampleBytes) / Math.max((now - sampleAt) / 1000, 0.05);
      sampleAt = now; sampleBytes = event.bytes;
      setTransfers(previous => previous.map(row => row.id === id ? {...row, ...event, speed, status: 'running', message: '传输中'} : row));
    };
    try {
      const result = await desktopCommand<TransferResult | null>(request.direction === 'upload' ? 'sftp_upload' : 'sftp_download',
        request.direction === 'upload' ? {sessionId: request.sessionId, directory: request.path, progress} : {sessionId: request.sessionId, path: request.path, progress});
      if (!result) { setTransfers(previous => previous.filter(row => row.id !== id)); return false; }
      setTransfers(previous => previous.map(row => row.id === id ? {...row, name: result.name, bytes: result.bytes, total: result.bytes, status: 'done', message: '已完成'} : row));
      return true;
    } catch (error) {
      setTransfers(previous => previous.map(row => row.id === id ? {...row, status: 'error', message: errorMessage(error)} : row));
      return false;
    }
  }, []);
  const clearCompleted = useCallback(() => setTransfers(previous => previous.filter(row => row.status === 'running' || row.status === 'choosing')), []);
  return <Context value={{transfers, run, clearCompleted}}>{children}</Context>;
}

export function useTransfers() {
  const value = useContext(Context);
  if (!value) throw new Error('TransferProvider is missing');
  return value;
}
