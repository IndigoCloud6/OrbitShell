import {useEffect, useRef, useState} from 'react';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {usePreferences} from '../../app/preferences';
import {desktopCommand, errorMessage} from '../../shared/desktop';
import type {ShellSession} from '../terminal/runtime';
import {FileBrowserView} from './file-browser-view';
import {useTransfers} from './transfer-provider';
import type {Directory, RemoteEntry} from './file-utils';
import {validateTerminalDirectory} from '../terminal/working-directory';
import {useFileDrag} from './use-file-drag';
export function FilePanel({session, onCollapse, onEdit, onTerminalDirectory, terminalDirectoryReason}: {
  session: ShellSession | null; onCollapse: () => void; onEdit?: (session: ShellSession, entry: RemoteEntry) => void;
  onTerminalDirectory?: () => Promise<string>; terminalDirectoryReason?: string;
}) {
  const {compactFiles, setCompactFiles, showHiddenFiles, setShowHiddenFiles} = usePreferences();
  const {run} = useTransfers();
  const queryClient = useQueryClient();
  const [paths, setPaths] = useState<Record<string, string>>({});
  const [draft, setDraft] = useState('.');
  const [selected, setSelected] = useState<RemoteEntry | null>(null);
  const [locating, setLocating] = useState(false);
  const [locateError, setLocateError] = useState('');
  const dropTarget = useRef<HTMLElement | null>(null);
  const intent = useRef(0);
  const currentSession = useRef(session?.id);
  if (currentSession.current !== session?.id) { currentSession.current = session?.id; intent.current++; }
  const request = useRef(0);
  useEffect(() => { request.current++; setLocating(false); setLocateError(''); }, [session?.id]);
  useEffect(() => () => { intent.current++; request.current++; }, []);
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
  const contextKey = `${session?.id ?? 'none'}:${path}:${session?.status ?? 'none'}`;
  useEffect(() => { setDraft(query.data?.path ?? path); setSelected(null); }, [session?.id, path, query.data?.path]);
  const navigate = (next: string) => { intent.current++; setLocateError(''); if (session && connected) setPaths(previous => ({...previous, [session.id]: next || '.'})); };
  const locateTerminal = async () => {
    if (!session || !connected || !onTerminalDirectory || locating || currentSession.current !== session.id) return;
    const sessionId = session.id;
    const navigation = ++intent.current;
    const id = ++request.current;
    setLocating(true); setLocateError('');
    try {
      const next = validateTerminalDirectory(await onTerminalDirectory());
      if (navigation !== intent.current || currentSession.current !== sessionId) return;
      const directory = await queryClient.fetchQuery({queryKey: ['remote-directory', sessionId, next], retry: false,
        queryFn: () => desktopCommand<Directory>('sftp_list', {sessionId, path: next})});
      if (navigation === intent.current && currentSession.current === sessionId) {
        setPaths(previous => ({...previous, [sessionId]: next})); setDraft(directory.path); setSelected(null);
      }
    } catch (failure) { if (navigation === intent.current && currentSession.current === sessionId) setLocateError(errorMessage(failure)); }
    finally { if (id === request.current) setLocating(false); }
  };
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
  const drag = useFileDrag({root: dropTarget, contextKey, enabled: Boolean(connected && query.data && !query.error && !preparing),
    directory: query.data?.path ?? path, entries: query.data?.entries ?? [],
    upload: async (localPaths, directory) => {
      if (!session || !connected) return;
      const sessionId = session.id;
      const key = ['remote-directory', sessionId, path];
      const targetKey = ['remote-directory', sessionId, directory];
      for (const localPath of localPaths) {
        const result = await run({sessionId, direction: 'upload', path: directory, localPath, name: localPath.split(/[\\/]/).pop() ?? localPath});
        if (!result) break;
        void queryClient.invalidateQueries({queryKey: key, exact: true});
        if (directory !== path) void queryClient.invalidateQueries({queryKey: targetKey, exact: true});
      }
    },
    download: async entry => {
      if (!session || !connected || entry.isDirectory) return;
      await run({sessionId: session.id, direction: 'download', path: entry.path, name: entry.name, drag: true});
    },
  });
  return <FileBrowserView connected={connected} entries={query.data?.entries ?? []} path={draft} selectedPath={selected?.path ?? null}
    contextKey={contextKey} directoryPath={query.data?.path ?? path} dropTargetRef={dropTarget} dropDirectory={drag.targetDirectory} onDragDownload={drag.beginDownload}
    compact={compactFiles} showHidden={showHiddenFiles} preparing={preparing} loading={query.isPending || preparing} fetching={query.isFetching}
    {...(query.error ? {error: errorMessage(query.error)} : {})}
    {...(drag.error ? {actionError: drag.error, actionErrorTitle: '文件拖放未完成', onDismissActionError: drag.dismissError} : locateError ? {actionError: locateError, onDismissActionError: () => setLocateError('')} : {})}
    locating={locating} {...(onTerminalDirectory ? {onLocateDirectory: () => { void locateTerminal(); }} : {})}
    {...(terminalDirectoryReason ? {locateDirectoryReason: terminalDirectoryReason} : {})}
    emptyTitle={session?.status === 'connecting' ? '正在连接服务器' : session?.status === 'hostKey' ? '等待确认服务器身份' : session ? '连接已断开' : '尚未连接服务器'}
    {...(session?.status === 'connecting' || session?.status === 'hostKey' ? {emptyDescription: '认证完成并建立会话后，将自动加载远程文件。'} : {})}
    onPath={value => { intent.current++; setDraft(value); setLocateError(''); }} onNavigate={navigate} onRefresh={() => { setLocateError(''); if (connected) void queryClient.refetchQueries({queryKey: ['remote-directory', session?.id, path], exact: true, type: 'active'}); }} onHidden={() => setShowHiddenFiles(!showHiddenFiles)}
    onDensity={() => setCompactFiles(!compactFiles)} onUpload={directory => { void transfer('upload', null, directory); }} onDownload={entry => { void transfer('download', entry); }}
    onSelect={setSelected} onCollapse={onCollapse} {...(onEdit ? {onEdit: (entry: RemoteEntry) => { if (session && connected) onEdit(session, entry); }} : {})}
    />;
}
