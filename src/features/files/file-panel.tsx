import {useEffect, useState} from 'react';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {usePreferences} from '../../app/preferences';
import {desktopCommand, errorMessage} from '../../shared/desktop';
import type {ShellSession} from '../terminal/runtime';
import {FileBrowserView} from './file-browser-view';
import {useTransfers} from './transfer-provider';
import type {Directory, RemoteEntry} from './file-utils';

export function FilePanel({session, onCollapse, onEdit}: {session: ShellSession | null; onCollapse: () => void; onEdit?: (session: ShellSession, entry: RemoteEntry) => void}) {
  const {compactFiles, setCompactFiles, showHiddenFiles, setShowHiddenFiles} = usePreferences();
  const {run} = useTransfers();
  const queryClient = useQueryClient();
  const [paths, setPaths] = useState<Record<string, string>>({});
  const [draft, setDraft] = useState('.');
  const [selected, setSelected] = useState<RemoteEntry | null>(null);
  const connected = session?.status === 'connected';
  const path = session ? paths[session.id] ?? '.' : '.';
  const query = useQuery({
    queryKey: ['remote-directory', session?.id, path], enabled: connected, retry: false,
    queryFn: async ({signal}) => {
      signal.throwIfAborted();
      const result = await desktopCommand<Directory>('sftp_list', {sessionId: session!.id, path});
      signal.throwIfAborted(); return result;
    },
  });
  const preparing = connected && query.isFetching && !query.data;
  useEffect(() => { setDraft(query.data?.path ?? path); setSelected(null); }, [session?.id, path, query.data?.path]);
  const navigate = (next: string) => { if (session && connected) setPaths(previous => ({...previous, [session.id]: next || '.'})); };
  const transfer = async (direction: 'upload' | 'download', entry = selected, directory = query.data?.path ?? path) => {
    if (!session || !connected || preparing || (direction === 'download' && (!entry || entry.isDirectory))) return;
    const sessionId = session.id;
    const key = ['remote-directory', sessionId, path];
    const targetKey = ['remote-directory', sessionId, directory];
    const result = await run({sessionId, direction, path: direction === 'upload' ? directory : entry!.path, ...(entry && direction === 'download' ? {name: entry.name} : {})});
    if (result && direction === 'upload') {
      void queryClient.invalidateQueries({queryKey: key, exact: true});
      if (directory !== path) void queryClient.invalidateQueries({queryKey: targetKey, exact: true});
    }
  };
  return <FileBrowserView connected={connected} entries={query.data?.entries ?? []} path={draft} selectedPath={selected?.path ?? null}
    contextKey={`${session?.id ?? 'none'}:${path}:${session?.status ?? 'none'}`} directoryPath={query.data?.path ?? path}
    compact={compactFiles} showHidden={showHiddenFiles} preparing={preparing} loading={query.isPending || preparing} fetching={query.isFetching}
    {...(query.error ? {error: errorMessage(query.error)} : {})}
    emptyTitle={session?.status === 'connecting' ? '正在连接服务器' : session?.status === 'hostKey' ? '等待确认服务器身份' : session ? '连接已断开' : '尚未连接服务器'}
    {...(session?.status === 'connecting' || session?.status === 'hostKey' ? {emptyDescription: '认证完成并建立会话后，将自动加载远程文件。'} : {})}
    onPath={setDraft} onNavigate={navigate} onRefresh={() => { if (connected) void queryClient.refetchQueries({queryKey: ['remote-directory', session?.id, path], exact: true, type: 'active'}); }} onHidden={() => setShowHiddenFiles(!showHiddenFiles)}
    onDensity={() => setCompactFiles(!compactFiles)} onUpload={directory => { void transfer('upload', null, directory); }} onDownload={entry => { void transfer('download', entry); }}
    onSelect={setSelected} onCollapse={onCollapse} {...(onEdit ? {onEdit: (entry: RemoteEntry) => { if (session && connected) onEdit(session, entry); }} : {})} />;
}
