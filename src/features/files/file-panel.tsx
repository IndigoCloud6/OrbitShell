import {useEffect, useState} from 'react';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {usePreferences} from '../../app/preferences';
import {desktopCommand, errorMessage} from '../../shared/desktop';
import type {ShellSession} from '../terminal/runtime';
import {FileBrowserView} from './file-browser-view';
import {useTransfers} from './transfer-provider';
import type {Directory, RemoteEntry} from './file-utils';

export function FilePanel({session, onCollapse}: {session: ShellSession | null; onCollapse: () => void}) {
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
  useEffect(() => { setDraft(query.data?.path ?? path); setSelected(null); }, [session?.id, path, query.data?.path]);
  const navigate = (next: string) => { if (session && connected) setPaths(previous => ({...previous, [session.id]: next || '.'})); };
  const transfer = async (direction: 'upload' | 'download') => {
    if (!session || !connected || (direction === 'download' && (!selected || selected.isDirectory))) return;
    const key = ['remote-directory', session.id, path];
    const result = await run({sessionId: session.id, direction, path: direction === 'upload' ? query.data?.path ?? path : selected!.path, ...(selected && direction === 'download' ? {name: selected.name} : {})});
    if (result && direction === 'upload') void queryClient.invalidateQueries({queryKey: key, exact: true});
  };
  return <FileBrowserView connected={connected} entries={query.data?.entries ?? []} path={draft} selectedPath={selected?.path ?? null}
    compact={compactFiles} showHidden={showHiddenFiles} loading={query.isPending} fetching={query.isFetching}
    {...(query.error ? {error: errorMessage(query.error)} : {})}
    emptyTitle={session?.status === 'connecting' ? '正在连接服务器' : session?.status === 'hostKey' ? '等待确认服务器身份' : session ? '连接已断开' : '尚未连接服务器'}
    onPath={setDraft} onNavigate={navigate} onRefresh={() => { void query.refetch(); }} onHidden={() => setShowHiddenFiles(!showHiddenFiles)}
    onDensity={() => setCompactFiles(!compactFiles)} onUpload={() => { void transfer('upload'); }} onDownload={() => { void transfer('download'); }}
    onSelect={setSelected} onCollapse={onCollapse} />;
}
