import {useEffect} from 'react';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {listen} from '@tauri-apps/api/event';
import {desktopCommand, isDesktop} from '../../shared/desktop';

export interface McpStatus {enabled: boolean; url: string | null; token: string; tools: string[]; protocolVersion: string; autoStart: boolean; startupError: string | null}
export function useMcpStatus() {
  const cache = useQueryClient();
  const query = useQuery({queryKey: ['mcp-status'], queryFn: () => desktopCommand<McpStatus>('mcp_status'), enabled: isDesktop, retry: false});
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen('orbitshell:mcp-status-changed', () => { void cache.invalidateQueries({queryKey: ['mcp-status']}); }).then(unlisten => {
      if (disposed) unlisten(); else { stop = unlisten; void cache.invalidateQueries({queryKey: ['mcp-status']}); }
    }).catch(() => {});
    return () => { disposed = true; stop?.(); };
  }, [cache]);
  return query;
}
