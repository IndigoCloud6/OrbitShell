import {useCallback, useEffect, useRef, useState} from 'react';
import {useQueryClient} from '@tanstack/react-query';
import {listen} from '@tauri-apps/api/event';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {useSessions} from '../terminal/session-provider';
import type {ShellSession} from '../terminal/runtime';
import type {RemoteEntry} from '../files/file-utils';
import {isDirty} from './types';
import {EditorDrafts} from './drafts';
import type {CloseRequest, Comparison, EditorDocument, EditorTab, OpenedFile, RemoteText, SaveResult} from './types';

export function useRemoteEditor() {
  const {sessions, activeId: sessionId, setActiveId: selectSession} = useSessions();
  const cache = useQueryClient();
  const documentsRef = useRef(new EditorDrafts()).current;
  const [documents, setDocuments] = useState<EditorDocument[]>(documentsRef.current);
  const updateDocuments = useCallback((change: (files: EditorDocument[]) => EditorDocument[]) => {
    setDocuments(documentsRef.update(change));
  }, [documentsRef]);
  const sessionsRef = useRef(sessions); sessionsRef.current = sessions;
  const [activeId, setActiveId] = useState<string | null>(null);
  const activeRef = useRef(activeId); activeRef.current = activeId;
  const [opening, setOpening] = useState(0);
  const openingKeys = useRef(new Set<string>());
  const documentIndex = useRef(new Map<string, string>());
  const intent = useRef(0);
  const pendingSaves = useRef(new Set<string>());
  const [error, setError] = useState('');
  const [comparison, setComparison] = useState<Comparison | null>(null);
  const [conflictId, setConflictId] = useState<string | null>(null);
  const [closing, setClosing] = useState<CloseRequest | null>(null);
  const closingRef = useRef(closing); closingRef.current = closing;
  const [closingBusy, setClosingBusy] = useState(false);
  const [restoreRequest, setRestoreRequest] = useState<{id: string; text: RemoteText} | null>(null);
  const allowWindowClose = useRef(false);
  const mounted = useRef(true);
  const patch = useCallback((id: string, update: Partial<EditorDocument>) => {
    updateDocuments(previous => previous.map(file => file.id === id ? {...file, ...update} : file));
  }, [updateDocuments]);
  const activate = useCallback((id: string) => {
    const file = documentsRef.current.find(item => item.id === id);
    if (!file) return;
    intent.current++;
    setActiveId(id); selectSession(sessionsRef.current.some(session => session.id === file.sessionId) ? file.sessionId : null);
  }, [selectSession]);
  const showTerminal = useCallback((id: string) => { intent.current++; setActiveId(null); selectSession(id); }, [selectSession]);
  useEffect(() => {
    const file = documentsRef.current.find(item => item.id === activeRef.current);
    if (file && sessionId !== null && file.sessionId !== sessionId) { intent.current++; setActiveId(null); }
  }, [sessionId]);
  const open = useCallback(async (session: ShellSession, entry: RemoteEntry) => {
    if (session.status !== 'connected' || entry.isDirectory) return;
    const existing = documentsRef.current.find(item => item.sessionId === session.id && item.path === entry.path);
    if (existing) { activate(existing.id); return; }
    const key = `${session.id}:${entry.path}`;
    if (openingKeys.current.has(key)) return;
    const focusIntent = ++intent.current;
    openingKeys.current.add(key); setOpening(count => count + 1); setError('');
    try {
      const loaded = await desktopCommand<OpenedFile>('editor_open', {sessionId: session.id, path: entry.path});
      if (!mounted.current) { void desktopCommand('editor_close', {id: loaded.id}).catch(() => {}); return; }
      const canonicalKey = `${session.id}:${loaded.path}`;
      const duplicateId = documentIndex.current.get(canonicalKey);
      if (duplicateId) {
        void desktopCommand('editor_close', {id: loaded.id}).catch(() => {});
        if (focusIntent === intent.current) { setActiveId(duplicateId); selectSession(sessionsRef.current.some(item => item.id === session.id) ? session.id : null); }
        return;
      }
      const file: EditorDocument = {id: loaded.id, sessionId: session.id, sessionName: session.profile.name,
        address: `${session.profile.hostname.includes(':') ? `[${session.profile.hostname}]` : session.profile.hostname}:${session.profile.port}`,
        name: loaded.path.split('/').pop() || entry.name, path: loaded.path, document: loaded.document, content: loaded.document.content,
        saving: false, restoring: false, error: loaded.warning ?? '', feedback: '', conflict: null, searchRequest: 0, historyRequest: 0};
      documentIndex.current.set(canonicalKey, file.id);
      updateDocuments(previous => [...previous, file]);
      if (focusIntent === intent.current) { setActiveId(file.id); selectSession(sessionsRef.current.some(session => session.id === file.sessionId) ? file.sessionId : null); }
    } catch (failure) { if (mounted.current) setError(errorMessage(failure)); }
    finally { openingKeys.current.delete(key); if (mounted.current) setOpening(count => count - 1); }
  }, [activate, selectSession, updateDocuments]);
  const save = useCallback(async (id: string, revision?: string): Promise<boolean> => {
    const file = documentsRef.current.find(item => item.id === id);
    if (!file || pendingSaves.current.has(id)) return false;
    if (sessionsRef.current.find(item => item.id === file.sessionId)?.status !== 'connected') { patch(id, {error: '所属会话已断开，编辑内容仍保留。请保留此标签或复制内容后重新打开文件。'}); return false; }
    if (!isDirty(file) && !revision) return true;
    pendingSaves.current.add(id); patch(id, {saving: true, error: '', feedback: ''});
    try {
      const result = await desktopCommand<SaveResult>('editor_save', {id, content: file.content, revision: revision ?? file.document.revision, restoring: file.restoring});
      if (result.status === 'conflict') { patch(id, {conflict: result.current}); setComparison(null); setConflictId(id); return false; }
      patch(id, {document: result.document, restoring: false, conflict: null, feedback: '已保存到远程 · 远程原文件已记录为本地快照'});
      void cache.invalidateQueries({queryKey: ['remote-directory', file.sessionId]});
      return true;
    } catch (failure) { patch(id, {error: errorMessage(failure)}); return false; }
    finally { pendingSaves.current.delete(id); patch(id, {saving: false}); }
  }, [cache, patch]);
  const closeNow = useCallback((id: string) => {
    if (pendingSaves.current.has(id)) return;
    intent.current++;
    const remaining = documentsRef.current.filter(item => item.id !== id);
    const closingFile = documentsRef.current.find(item => item.id === id);
    if (closingFile) documentIndex.current.delete(`${closingFile.sessionId}:${closingFile.path}`);
    updateDocuments(previous => previous.filter(item => item.id !== id));
    if (activeRef.current === id) {
      const next = remaining[remaining.length - 1];
      setActiveId(next?.id ?? null);
      if (next) selectSession(sessionsRef.current.some(session => session.id === next.sessionId) ? next.sessionId : null);
      else {
        const owner = documentsRef.current.find(item => item.id === id)?.sessionId;
        const session = sessionsRef.current.find(item => item.id === owner) ?? sessionsRef.current[0];
        selectSession(session?.id ?? null);
      }
    }
    setComparison(current => current?.documentId === id ? null : current);
    void desktopCommand('editor_close', {id}).catch(failure => setError(errorMessage(failure)));
  }, [documentsRef, selectSession, updateDocuments]);
  const requestClose = useCallback((id: string) => {
    const file = documentsRef.current.find(item => item.id === id);
    if (!file || file.saving || pendingSaves.current.has(id)) return;
    if (isDirty(file)) setClosing({kind: 'file', id}); else closeNow(id);
  }, [closeNow]);
  const closeWindow = useCallback(async (): Promise<boolean> => {
    allowWindowClose.current = true;
    try { await desktopCommand('desktop_confirm_exit'); return true; }
    catch (failure) { allowWindowClose.current = false; setError(errorMessage(failure)); return false; }
  }, []);
  const cancelClose = useCallback(() => {
    if (closingRef.current?.kind === 'window' && isDesktop) void desktopCommand('desktop_cancel_exit').catch(failure => setError(errorMessage(failure)));
    setClosing(null);
  }, []);
  const finishClose = async (saveChanges: boolean) => {
    if (!closing || closingBusy || documentsRef.current.some(file => file.saving)) return;
    setClosingBusy(true); setError('');
    try {
      if (saveChanges) {
        const targets = documentsRef.current.filter(file => isDirty(file) && (closing.kind === 'window' || file.id === closing.id));
        for (const file of targets) {
          if (!await save(file.id)) { activate(file.id); cancelClose(); return; }
        }
      }
      if (closing.kind === 'window') { if (!await closeWindow()) return; } else closeNow(closing.id);
      setClosing(null);
    } finally { setClosingBusy(false); }
  };
  useEffect(() => {
    mounted.current = true;
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (!allowWindowClose.current && documentsRef.current.some(file => isDirty(file) || file.saving)) { event.preventDefault(); event.returnValue = ''; }
    };
    window.addEventListener('beforeunload', beforeUnload);
    let disposed = false; let stop: (() => void) | undefined; let registered = false;
    const guardId = crypto.randomUUID();
    const origin = Math.floor(performance.timeOrigin * 1000);
    const register = async () => {
      const unlisten = await listen('orbitshell:request-exit', () => {
        if (documentsRef.current.some(file => isDirty(file) || file.saving) || pendingSaves.current.size > 0) setClosing({kind: 'window'});
        else void closeWindow();
      });
      if (disposed) { unlisten(); return; }
      stop = unlisten;
      await desktopCommand('desktop_register_exit_guard', {id: guardId, origin});
      registered = true;
      if (disposed) { unlisten(); await desktopCommand('desktop_unregister_exit_guard', {id: guardId}); }
    };
    if (isDesktop) void register().catch(failure => { if (!disposed) setError(errorMessage(failure)); });
    return () => {
      mounted.current = false; disposed = true; stop?.(); window.removeEventListener('beforeunload', beforeUnload);
      if (registered) void desktopCommand('desktop_unregister_exit_guard', {id: guardId}).catch(() => {});
      queueMicrotask(() => {
        if (!mounted.current) for (const file of documentsRef.current) void desktopCommand('editor_close', {id: file.id}).catch(() => {});
      });
    };
  }, [closeWindow]);
  const active = documents.find(file => file.id === activeId) ?? null;
  const find = useCallback(() => { const file = documentsRef.current.find(item => item.id === activeRef.current); if (file) patch(file.id, {searchRequest: file.searchRequest + 1}); }, [patch]);
  const tabs: EditorTab[] = documents.map(file => ({id: file.id, name: file.name, path: file.path, sessionName: file.sessionName,
    dirty: isDirty(file), saving: file.saving, canSave: sessions.some(session => session.id === file.sessionId && session.status === 'connected')}));
  return {documents, activeId, active, tabs, opening, error, clearError: () => setError(''), open, activate, showTerminal, save, requestClose, patch,
    comparison, setComparison, conflictId, setConflictId, closing, cancelClose, closingBusy, finishClose,
    restoreRequest, setRestoreRequest,
    history: (id: string) => { const file = documentsRef.current.find(item => item.id === id); if (file) { activate(id); patch(id, {historyRequest: file.historyRequest + 1}); } },
    find,
    restore: (id: string, text: RemoteText) => { patch(id, {content: text.content, restoring: true, feedback: '已恢复到编辑器，保存到远程后才会生效。'}); setComparison(null); },
  };
}
export type EditorController = ReturnType<typeof useRemoteEditor>;
