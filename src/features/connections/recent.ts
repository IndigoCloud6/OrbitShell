import {useEffect, useState} from 'react';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {listen} from '@tauri-apps/api/event';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';

export interface RecentConnection {
  id: string; profileId: string | null; sessionId: string | null; name: string; group: string;
  hostname: string; port: number; username: string; lastUsedAt: number; temporary: boolean; requiresNewLink: boolean;
}
export interface RecentSnapshot {records: RecentConnection[]; warning: string | null}
export const recentConnectionsKey = ['recent-connections'] as const;

export function useRecentConnections() {
  const cache = useQueryClient();
  const [liveError, setLiveError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const query = useQuery({queryKey: recentConnectionsKey, queryFn: () => desktopCommand<RecentSnapshot>('list_recent_connections'),
    enabled: isDesktop, retry: false});
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen('orbitshell:recent-connections-changed', () => {
      if (!disposed) void cache.invalidateQueries({queryKey: recentConnectionsKey});
    }).then(unlisten => {
      if (disposed) unlisten();
      else { stop = unlisten; setLiveError(''); void cache.invalidateQueries({queryKey: recentConnectionsKey}); }
    }).catch(failure => { if (!disposed) setLiveError(errorMessage(failure)); });
    return () => { disposed = true; stop?.(); };
  }, [cache, attempt]);
  return {...query, liveError, retryHistory: () => { setAttempt(value => value + 1); void query.refetch(); }};
}

export function quickConnect(value: string) {
  return desktopCommand<{profileId: string | null}>('open_quick_connection', {value});
}

export function recentAddress(record: RecentConnection) {
  const host = record.hostname.includes(':') && !record.hostname.startsWith('[') ? `[${record.hostname}]` : record.hostname;
  return `${record.username ? `${record.username}@` : ''}${host}:${record.port}`;
}

export function recentQuickTarget(record: RecentConnection) {
  const host = record.hostname.includes(':') && !record.hostname.startsWith('[') ? `[${record.hostname}]` : record.hostname;
  return `${record.username ? `${encodeURIComponent(record.username)}@` : ''}${host}:${record.port}`;
}
