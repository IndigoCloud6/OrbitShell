import {lazy, Suspense, useCallback, useEffect, useRef, useState} from 'react';
import {Outlet, useLocation, useNavigate, useSearchParams} from 'react-router';
import {useQuery} from '@tanstack/react-query';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {useResizable} from '@astryxdesign/core/Resizable';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {Banner} from '@astryxdesign/core/Banner';
import {Bot, History, ShieldCheck} from 'lucide-react';
import {CredentialsDialog, useConnections} from '../connections/dialogs';
import {ConnectionExplorer} from '../connections/connection-explorer';
import type {ConnectionProfile} from '../connections/types';
import {useRecentConnections} from '../connections/recent';
import {useSessions} from '../terminal/session-provider';
import {TerminalPane} from '../terminal/terminal-pane';
import {SessionTabs} from '../terminal/session-tabs';
import type {ShellSession} from '../terminal/runtime';
import {FilePanel} from '../files/file-panel';
import {useTransfers} from '../files/transfer-provider';
import {usePreferences} from '../../app/preferences';
import {useThemePreference} from '../../app/theme';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {desktopStatusKey} from '../../shared/desktop-lifecycle';
import type {DesktopStatus} from '../../shared/desktop-lifecycle';
import {IdeLayout} from './ide-layout';
import {ActivityRail, IdeStatusbar, IdeTitlebar, OrbitMark, SessionContextBar} from './ide-chrome';
import type {IdeMenu, ToolTab} from './ide-chrome';
import {WorkspaceTools} from './workspace-tools';
import {WorkspaceStart} from './start-page';
import {AgentPanel} from '../ai/agent-panel';
import {AiTakeoverDialog} from '../ai/takeover-dialog';
import {AiTerminalPane} from '../ai/ai-terminal-pane';
import {useAiWorkspace, useAiWorkspaceFeed} from '../ai/ai-workspace';
import {useMcpStatus} from '../ai/mcp-status';
import {CommandHistoryPane, HISTORY_TAB_ID} from '../history/command-history';
import {QuickCommandPanel} from '../quick-commands/quick-commands';
import {useQuickCommandDraft} from '../quick-commands/command-draft';
import {useRemoteEditor} from '../editor/use-remote-editor';
import {isDirty} from '../editor/types';
import type {ConnectionLog} from './workspace-tools';

const RemoteEditorPane = lazy(() => import('../editor/remote-editor-pane').then(module => ({default: module.RemoteEditorPane})));
const EditorDialogs = lazy(() => import('../editor/editor-dialogs').then(module => ({default: module.EditorDialogs})));

function HostKeyPrompt({session}: {session: ShellSession}) {
  const {trustHost} = useSessions();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const answer = async (accept: boolean) => {
    setBusy(true); setError('');
    try { await trustHost(session.id, accept); } catch (failure) { setError(errorMessage(failure)); } finally { setBusy(false); }
  };
  return <Dialog isOpen className="orbit-dialog host-key-dialog" purpose="required" width="calc(var(--orbit-unit) * 48)" padding={0}
    maxHeight="calc(100dvh - var(--orbit-unit) * 6)" onOpenChange={() => {}}>
    <VStack className="orbit-dialog-layout" gap={0}>
      <DialogHeader className="orbit-dialog-header" title="确认服务器身份" hasDivider startContent={<ShieldCheck size="1.2em" />} />
      <VStack className="orbit-dialog-body" gap={4}>
        <Text className="host-key-server mono-text">{session.hostKey?.hostname}:{session.hostKey?.port}</Text>
        <Text>首次连接，请核对服务器指纹，确认后保存到本机。</Text>
        <VStack className="host-key-details" gap={3}>
          <VStack gap={1}><Text className="field-label" color="secondary">密钥算法</Text><Text className="mono-text">{session.hostKey?.algorithm ?? '—'}</Text></VStack>
          <VStack gap={1}><Text className="field-label" color="secondary">SHA-256 指纹</Text><Text className="host-fingerprint mono-text">{session.hostKey?.fingerprint ?? '—'}</Text></VStack>
        </VStack>
        {error && <Banner className="compact-feedback" status="error" title="确认未完成" description={error} />}
      </VStack>
      <HStack className="orbit-dialog-footer" gap={2} hAlign="end"><Button label="取消连接" isDisabled={busy} onClick={() => { void answer(false); }} /><Button label="信任并连接" variant="primary" isLoading={busy} onClick={() => { void answer(true); }} /></HStack>
    </VStack>
  </Dialog>;
}

export default function Workbench() {
  const navigate = useNavigate();
  const location = useLocation();
  const [params, setParams] = useSearchParams();
  const query = useConnections();
  const recent = useRecentConnections();
  const {sessions, activeId, getRuntime, close, pendingProfile, dismissPending, launchError, clearLaunchError} = useSessions();
  const {transfers, clearCompleted} = useTransfers();
  const {fontSize, setFontSize, showServers, setShowServers, showFiles, setShowFiles} = usePreferences();
  const theme = useThemePreference();
  const [notice, setNotice] = useState('');
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchRequest, setSearchRequest] = useState(0);
  const [toolsOpen, setToolsOpen] = useState(true);
  const [toolTab, setToolTab] = useState<ToolTab>('transfers');
  const [about, setAbout] = useState(false);
  const [takeoverSessionId, setTakeoverSessionId] = useState<string | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyActive, setHistoryActive] = useState(false);
  const [historyFind, setHistoryFind] = useState(0);
  const beforeHistory = useRef<{kind: 'terminal' | 'editor' | 'ai'; id: string} | null>(null);
  const previousSessions = useRef(new Set(sessions.map(session => session.id)));
  const [startFilesOpen, setStartFilesOpen] = useState(false);
  const [logs, setLogs] = useState<ConnectionLog[]>([]);
  const lastStates = useRef(new Map<string, string>());
  const knownTransfers = useRef(new Set<string>());
  const aiWorkspace = useAiWorkspace();
  const aiFeed = useAiWorkspaceFeed();
  const [activeAiId, setActiveAiId] = useState<string | null>(null);
  const [hiddenAiIds, setHiddenAiIds] = useState<ReadonlySet<string>>(new Set());
  const [restoredAiIds, setRestoredAiIds] = useState<ReadonlySet<string>>(new Set());
  const aiTabs = aiWorkspace.terminals.filter(terminal => (terminal.mode === 'dedicated' || restoredAiIds.has(terminal.id)) && !hiddenAiIds.has(terminal.id));
  const takeovers = aiWorkspace.terminals.filter(terminal => terminal.mode === 'takeover' && terminal.status !== 'closed' && terminal.status !== 'error');
  const activeAi = aiTabs.find(terminal => terminal.id === activeAiId) ?? null;
  const editor = useRemoteEditor();
  const activeTakeover = !editor.activeId && !activeAiId ? takeovers.find(terminal => terminal.sessionId === activeId) ?? null : null;
  const showTerminal = useCallback((id: string) => { setHistoryActive(false); setActiveAiId(null); editor.showTerminal(id); }, [editor.showTerminal]);
  const showAiTerminal = useCallback((id: string) => {
    const terminal = aiWorkspace.terminals.find(item => item.id === id);
    if (!terminal) return;
    setHistoryActive(false);
    if (terminal.mode === 'takeover' && sessions.some(session => session.id === terminal.sessionId)) { setActiveAiId(null); editor.showTerminal(terminal.sessionId); setSearchOpen(false); return; }
    if (terminal.mode === 'takeover') setRestoredAiIds(previous => new Set([...previous, id]));
    setHiddenAiIds(previous => { const next = new Set(previous); next.delete(id); return next; });
    editor.showTerminal(terminal.sessionId); setActiveAiId(id); setSearchOpen(false);
  }, [aiWorkspace.terminals, editor.showTerminal, sessions]);
  const closeAiTerminal = useCallback((id: string) => {
    setHiddenAiIds(previous => new Set([...previous, id]));
    setActiveAiId(current => current === id ? null : current);
    if (id === activeAiId && !sessions.length && editor.documents[0]) editor.activate(editor.documents[0].id);
  }, [activeAiId, sessions.length, editor.documents, editor.activate]);
  useEffect(() => { aiFeed.view(historyActive ? null : activeAiId ?? activeTakeover?.id ?? null); return () => aiFeed.view(null); }, [aiFeed, activeAiId, activeTakeover?.id, historyActive]);
  useEffect(() => { if (editor.activeId) { setActiveAiId(null); setHistoryActive(false); } }, [editor.activeId]);
  useEffect(() => {
    // Removing a background session must not dismiss the selected utility tab.
    if (activeId && !previousSessions.current.has(activeId)) { setHistoryActive(false); setActiveAiId(null); }
    previousSessions.current = new Set(sessions.map(session => session.id));
  }, [activeId, sessions]);
  useEffect(() => {
    if (activeAiId && !aiWorkspace.terminals.some(t => t.id === activeAiId)) setActiveAiId(null);
  }, [activeId, activeAiId, aiWorkspace.terminals]);
  useEffect(() => {
    if (!historyActive && !activeId && !editor.activeId && !activeAiId && aiTabs.length) setActiveAiId(aiTabs[0]?.id ?? null);
  }, [activeId, editor.activeId, activeAiId, aiTabs, historyActive]);
  const viewedAi = activeAi ?? activeTakeover;
  const focusedTerminal = () => historyActive ? null : activeAi ? aiFeed.getRuntime(activeAi.id).terminal : activeId ? getRuntime(activeId)?.terminal : null;
  const starting = !sessions.length && !editor.documents.length && !aiTabs.length && !historyOpen;
  const filesVisible = historyActive ? false : starting ? startFilesOpen : showFiles;
  const toggleFiles = () => { if (historyActive) { restoreWorkspace(); setShowFiles(true); } else if (starting) setStartFilesOpen(value => !value); else setShowFiles(!showFiles); };
  const {stage: stageCommand} = useQuickCommandDraft();
  const active = sessions.find(session => session.id === activeId) ?? null;
  const pendingHost = sessions.find(session => session.hostKey);
  const takeoverSession = sessions.find(session => session.id === takeoverSessionId);
  useEffect(() => {
    if (isDesktop) void desktopCommand('ai_set_active_session', {sessionId: !historyActive && !activeAiId && !editor.activeId ? activeId : null}).catch(() => {});
  }, [activeId, activeAiId, editor.activeId, historyActive]);
  const startPanel = useResizable({defaultSize: '15%', minSize: 200, maxSize: 380, autoSaveId: 'orbitshell.ide.connections.v2'});
  const endPanel = useResizable({defaultSize: '20%', minSize: 260, maxSize: 480, autoSaveId: 'orbitshell.ide.files.v2'});
  const mcp = useMcpStatus();
  const desktop = useQuery<DesktopStatus>({queryKey: desktopStatusKey, queryFn: () => desktopCommand('desktop_status'), enabled: isDesktop, retry: false});
  const openSearch = useCallback(() => { setSearchOpen(true); setSearchRequest(value => value + 1); }, []);
  const closeSearch = useCallback(() => setSearchOpen(false), []);
  const showHistory = () => {
    if (!historyActive) beforeHistory.current = editor.activeId ? {kind: 'editor', id: editor.activeId} : activeAiId ? {kind: 'ai', id: activeAiId} : activeId ? {kind: 'terminal', id: activeId} : null;
    editor.showUtility(); setHistoryOpen(true); setHistoryActive(true); setActiveAiId(null); setSearchOpen(false);
  };
  const restoreWorkspace = useCallback(() => {
    setHistoryActive(false);
    const previous = beforeHistory.current;
    if (previous?.kind === 'editor' && editor.documents.some(file => file.id === previous.id)) editor.activate(previous.id);
    else if (previous?.kind === 'ai' && aiTabs.some(terminal => terminal.id === previous.id)) showAiTerminal(previous.id);
    else if (previous?.kind === 'terminal' && sessions.some(session => session.id === previous.id)) showTerminal(previous.id);
    else if (sessions[0]) showTerminal(sessions[0].id);
    else if (editor.documents[0]) editor.activate(editor.documents[0].id);
    else if (aiTabs[0]) showAiTerminal(aiTabs[0].id);
  }, [editor.documents, editor.activate, aiTabs, showAiTerminal, sessions, showTerminal]);
  const closeHistory = useCallback(() => { setHistoryOpen(false); if (historyActive) restoreWorkspace(); }, [historyActive, restoreWorkspace]);
  const openTool = (tab: ToolTab) => { setToolTab(tab); setToolsOpen(true); };
  const reconnectSession = (id: string) => {
    const session = sessions.find(candidate => candidate.id === id);
    if (!session || session.status === 'connecting' || session.status === 'hostKey') return;
    if (session.ephemeral) {
      setNotice('临时堡垒机会话需要新的有效链接，请重新打开会话链接。');
      void navigate('/settings?section=bastion');
      return;
    }
    if (query.isPending || query.error) {
      setNotice('连接配置尚未读取成功，请在连接列表读取完成后重试。');
      return;
    }
    if (!query.data?.some(profile => profile.id === session.profile.id)) {
      setNotice('该会话的连接配置已删除，请在管理连接中重新配置。');
      return;
    }
    void navigate(`/connections/${session.profile.id}/connect`);
  };
  const reconnect = () => { if (active) reconnectSession(active.id); };
  const disconnectSession = (id: string) => {
    if (sessions.find(session => session.id === id)?.status !== 'connected') return;
    void desktopCommand('ssh_disconnect', {sessionId: id}).catch(error => setNotice(errorMessage(error)));
  };
  const saveSelection = (text: string) => {
    if (!text.trim()) return;
    if (new TextEncoder().encode(text).byteLength > 64 * 1024 || text.includes('\0')) {
      setNotice('选区包含空字符或超过 64 KiB，请缩小选区后再保存为快捷指令。');
      return;
    }
    stageCommand(text);
    void navigate('/commands/new?return=workbench');
  };
  const openProfile = (profile: ConnectionProfile) => {
    const existing = sessions.find(session => session.profile.id === profile.id && session.status !== 'closed' && session.status !== 'error');
    if (existing) showTerminal(existing.id); else void navigate(`/connections/${profile.id}/connect`);
  };

  useEffect(() => {
    const changes: ConnectionLog[] = [];
    for (const session of sessions) {
      const state = `${session.status}:${session.message}`;
      if (lastStates.current.get(session.id) === state) continue;
      lastStates.current.set(session.id, state);
      changes.push({id: crypto.randomUUID(), time: new Date().toLocaleTimeString('zh-CN', {hour12: false}), name: session.profile.name, message: session.message});
    }
    if (changes.length) setLogs(previous => [...previous, ...changes].slice(-200));
  }, [sessions]);
  useEffect(() => {
    const added = transfers.some(transfer => !knownTransfers.current.has(transfer.id));
    knownTransfers.current = new Set(transfers.map(transfer => transfer.id));
    if (added) { setToolsOpen(true); setToolTab('transfers'); }
  }, [transfers]);
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => {
      if (event.isComposing || location.pathname !== '/' || pendingHost || pendingProfile) return;
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest('[role="menu"]')) return;
      const consume = () => { event.preventDefault(); event.stopPropagation(); };
      if (event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey && event.code === 'KeyS') {
        consume(); if (!historyActive && editor.activeId && !target?.closest('dialog, [role="dialog"], [role="alertdialog"]')) void editor.save(editor.activeId); return;
      }
      if (target?.closest('dialog, [role="dialog"], [role="alertdialog"]')) return;
      if (event.key === 'Escape' && searchOpen && (activeId || activeAiId)) { consume(); closeSearch(); return; }
      if (!event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.shiftKey && event.code === 'KeyT') { consume(); void navigate('/connections/new'); return; }
      if (event.shiftKey && event.code === 'KeyW') {
        if (historyActive) { consume(); closeHistory(); return; }
        if (activeAiId) { consume(); closeAiTerminal(activeAiId); return; }
        if (editor.activeId) { consume(); editor.requestClose(editor.activeId); return; }
        if (activeId) { consume(); close(activeId); return; }
      }
      if (target?.closest('input, textarea, [contenteditable="true"]') && !target.closest('.terminal-pane')) return;
      if (event.code === 'KeyJ') { consume(); setToolsOpen(value => !value); return; }
      if (event.code === 'KeyB') { consume(); setShowServers(!showServers); return; }
      if (historyActive) { if (event.code === 'KeyF') { consume(); setHistoryFind(value => value + 1); } return; }
      if (!activeId && !activeAiId) return;
      if (event.code === 'KeyF') { consume(); if (editor.activeId) editor.find(); else openSearch(); return; }
      if (editor.activeId) return;
      if (event.code === 'Equal' || event.code === 'NumpadAdd') { consume(); setFontSize(value => value + 1); }
      else if (event.code === 'Minus' || event.code === 'NumpadSubtract') { consume(); setFontSize(value => value - 1); }
      else if (event.code === 'Digit0' || event.code === 'Numpad0') { consume(); setFontSize(17); }
    };
    window.addEventListener('keydown', keyboard, true);
    return () => window.removeEventListener('keydown', keyboard, true);
  }, [navigate, location.pathname, activeId, pendingHost, pendingProfile, close, openSearch, closeSearch, searchOpen, setFontSize, setShowServers, showServers, editor.activeId, editor.save, editor.requestClose, editor.find, activeAiId, closeAiTerminal, historyActive, closeHistory]);

  function windowAction(action: 'minimize' | 'maximize' | 'close') {
    if (!isDesktop) return;
    const current = getCurrentWindow();
    void (action === 'minimize' ? current.minimize() : action === 'maximize' ? current.toggleMaximize() : desktopCommand('desktop_close_window')).catch(error => setNotice(errorMessage(error)));
  }
  const menus: IdeMenu[] = [
    {label: '文件', items: [
      {label: '新建连接', endContent: <Text className="menu-shortcut">Ctrl + Shift + T</Text>, onClick: () => { void navigate('/connections/new'); }},
      {label: '管理连接', onClick: () => { void navigate('/connections'); }},
      {label: '打开会话链接', onClick: () => { void navigate('/settings?section=bastion'); }},
      {type: 'divider'},
      {label: '保存远程文件', endContent: <Text className="menu-shortcut">Ctrl + S</Text>, isDisabled: historyActive || !editor.active || !isDirty(editor.active) || editor.active.saving || active?.status !== 'connected', onClick: () => { if (editor.active) void editor.save(editor.active.id); }},
      {label: '关闭文件标签', isDisabled: historyActive || !editor.active || editor.active.saving, onClick: () => { if (editor.active) editor.requestClose(editor.active.id); }},
      {type: 'divider'},
      {label: '设置', onClick: () => { void navigate('/settings'); }},
      {label: '后台运行', isDisabled: !isDesktop || !desktop.data?.trayAvailable, onClick: () => { void desktopCommand('desktop_hide_window').catch(failure => setNotice(errorMessage(failure))); }},
      {label: '退出 OrbitShell', isDisabled: !isDesktop, onClick: () => { void desktopCommand('desktop_request_exit').catch(failure => setNotice(errorMessage(failure))); }},
    ]},
    {label: '编辑', items: [
      {label: '复制终端选区', endContent: <Text className="menu-shortcut">Ctrl + C</Text>, isDisabled: historyActive || (!active && !activeAi) || Boolean(editor.active), onClick: () => {
        const text = focusedTerminal()?.getSelection();
        if (text) void navigator.clipboard.writeText(text).catch(error => setNotice(errorMessage(error)));
      }},
      {label: '粘贴到终端', endContent: <Text className="menu-shortcut">Ctrl + V</Text>, isDisabled: historyActive || Boolean(activeAi || activeTakeover) || active?.status !== 'connected' || Boolean(editor.active), onClick: () => {
        void navigator.clipboard.readText().then(text => {
          if (active) getRuntime(active.id)?.terminal?.paste(text);
        }).catch(error => setNotice(errorMessage(error)));
      }},
      {label: historyActive ? '搜索执行历史' : editor.active ? '查找文件内容' : '查找终端内容', endContent: <Text className="menu-shortcut">Ctrl + F</Text>, isDisabled: !historyActive && !active && !activeAi && !editor.active, onClick: historyActive ? () => setHistoryFind(value => value + 1) : editor.active ? editor.find : openSearch},
      {label: '快捷指令管理', onClick: () => { void navigate('/commands'); }},
      {label: '清空终端显示', isDisabled: historyActive || (!active && !activeAi) || Boolean(editor.active), onClick: () => focusedTerminal()?.clear()},
    ]},
    {label: '视图', items: [
      {label: showServers ? '收起连接列表' : '显示连接列表', endContent: <Text className="menu-shortcut">Ctrl + B</Text>, onClick: () => setShowServers(!showServers)},
      {label: filesVisible ? '收起远程文件' : '显示远程文件', onClick: toggleFiles},
      {label: toolsOpen ? '折叠工具面板' : '展开工具面板', endContent: <Text className="menu-shortcut">Ctrl + J</Text>, onClick: () => setToolsOpen(!toolsOpen)},
      {label: '执行历史', onClick: showHistory},
      {label: 'Agent 连接状态', onClick: () => openTool('ai')},
      {label: '快捷指令', onClick: () => openTool('commands')},
      {type: 'divider'},
      {label: '深色主题', onClick: () => theme.setMode('dark')},
      {label: '浅色主题', onClick: () => theme.setMode('light')},
      {label: '跟随系统', onClick: () => theme.setMode('system')},
    ]},
    {label: '会话', items: [
      {label: '打开会话', onClick: () => { void navigate('/connections'); }},
      {label: '重新连接', isDisabled: historyActive || Boolean(activeAi || activeTakeover) || !active || active.status === 'connecting' || active.status === 'hostKey', onClick: reconnect},
      {label: '关闭当前标签', endContent: <Text className="menu-shortcut">Ctrl + Shift + W</Text>, isDisabled: !historyActive && !active && !activeAi && !editor.active, onClick: () => { if (historyActive) closeHistory(); else if (activeAi) closeAiTerminal(activeAi.id); else if (editor.active) editor.requestClose(editor.active.id); else if (active) close(active.id); }},
      {label: '断开连接', isDisabled: historyActive || Boolean(activeAi || activeTakeover) || active?.status !== 'connected', onClick: () => {
        if (active) disconnectSession(active.id);
      }},
    ]},
    {label: '帮助', items: [
      {label: '快捷键', onClick: () => { void navigate('/settings'); }},
      {label: '关于 OrbitShell', onClick: () => setAbout(true)},
    ]},
  ];
  const ai = <AgentPanel onOpenTerminal={showAiTerminal} mcpEnabled={Boolean(mcp.data?.enabled)} mcpPending={isDesktop && mcp.isPending}
    mcpError={mcp.error ? errorMessage(mcp.error) : mcp.data?.startupError ?? ''} onRefreshMcp={() => { void mcp.refetch(); }} />;

  return <>
    <IdeLayout titlebar={<IdeTitlebar menus={menus} onWindow={windowAction} />}
      activity={<ActivityRail showServers={showServers} showFiles={filesVisible} toolTab={toolTab} toolsOpen={toolsOpen} onServers={() => setShowServers(!showServers)}
        onFiles={toggleFiles} historyActive={historyActive} onHistory={showHistory} onSessions={() => { void navigate('/connections'); }} onTool={openTool} onSettings={() => { void navigate('/settings'); }} />}
      sidebar={<ConnectionExplorer profiles={query.data ?? []} sessions={sessions} activeId={activeId} search={params.get('q') ?? ''} onSearch={value => {
        const next = new URLSearchParams(params); if (value) next.set('q', value); else next.delete('q'); void setParams(next, {replace: true});
      }} onOpen={openProfile} onCreate={() => { void navigate('/connections/new'); }} onManage={() => { void navigate('/connections'); }} pending={query.isPending}
        {...(query.error ? {error: errorMessage(query.error)} : {})} onRetry={() => { void query.refetch(); }} />}
      files={<FilePanel session={active} onCollapse={() => { if (starting) setStartFilesOpen(false); else setShowFiles(false); }} onEdit={(session, entry) => { setHistoryActive(false); void editor.open(session, entry); }} />}
      footer={<IdeStatusbar active={active} mcpEnabled={Boolean(mcp.data?.enabled)}>{viewedAi && <HStack className="status-segment ai-status-segment" gap={1} vAlign="center"><Bot size="1em" /><Text>{viewedAi.mode === 'takeover' && viewedAi.status !== 'closed' ? 'AI 接管中' : viewedAi.status === 'running' ? 'AI 操作中' : 'AI 终端'} · {viewedAi.agentName}</Text></HStack>}</IdeStatusbar>}
      showServers={showServers} showFiles={filesVisible} serverWidth={startPanel.size} filesWidth={endPanel.size} serverResize={startPanel.props} filesResize={endPanel.props}>
      <VStack className="workspace-stack" height="100%" gap={0}>
        <HStack className="session-tabs" gap={0} vAlign="center"><StackItem className="session-strip-container" size="fill"><SessionTabs sessions={sessions} activeId={activeId} onSelect={showTerminal} onClose={close} onReconnect={reconnectSession} onDisconnect={disconnectSession} onCreate={() => { void navigate('/connections'); }}
          editors={editor.tabs} activeEditorId={editor.activeId} onSelectEditor={id => { setHistoryActive(false); editor.activate(id); }} onCloseEditor={editor.requestClose} onSaveEditor={id => { void editor.save(id); }} onEditorHistory={editor.history} aiTerminals={aiTabs} activeAiId={activeAiId} unreadAiIds={aiWorkspace.unread} onSelectAi={showAiTerminal} onCloseAi={closeAiTerminal} utilityTabs={historyOpen ? [{id: HISTORY_TAB_ID, label: '执行历史', icon: <History size="1em" />}] : []} activeUtilityId={historyActive ? HISTORY_TAB_ID : null} onSelectUtility={showHistory} onCloseUtility={closeHistory} takeovers={takeovers} onTakeOverAi={setTakeoverSessionId} onReleaseAi={id => { void desktopCommand('ai_release_session', {id}).then(() => { void aiFeed.refresh(); }).catch(error => setNotice(errorMessage(error))); }} /></StackItem></HStack>
        {editor.opening > 0 && <Banner className="compact-feedback" status="info" title="正在打开远程文件…" />}
        {editor.error && <Banner className="compact-feedback" status="error" title="远程文件操作未完成" description={editor.error} isDismissable onDismiss={editor.clearError} />}
        {desktop.data?.error && <Banner className="compact-feedback" status="warning" title="系统托盘不可用" description={desktop.data.error} />}
        {recent.data?.warning && <Banner className="compact-feedback" status="warning" title="最近会话未能持久保存" description={recent.data.warning} />}
        {notice && <Banner status="info" title={notice} isDismissable onDismiss={() => setNotice('')} />}
        {launchError && <Banner status="error" title="无法打开外部会话" description={launchError} isDismissable onDismiss={clearLaunchError} />}
        {pendingProfile && location.pathname !== '/' && <Banner status="info" title="有连接正在等待登录" endContent={<Button label="前往登录" href="/" size="sm" />} />}
        {active && !historyActive && !activeAi && !editor.active && <SessionContextBar session={active} onSearch={openSearch} onReconnect={reconnect} onFiles={toggleFiles} aiControlled={Boolean(activeTakeover)} onTakeOverAi={setTakeoverSessionId} />}
        {activeTakeover && !historyActive && <HStack className="ai-takeover-toolbar" gap={2} vAlign="center"><Bot size="1em" /><Text maxLines={1}>AI 接管中 · {activeTakeover.agentName} · 人工输入已锁定</Text><StackItem size="fill" />
          <Button label="交回人工" size="sm" onClick={() => { void desktopCommand('ai_release_session', {id: activeTakeover.id}).then(() => { setNotice('已交回人工，已发出的远端命令可能仍在运行。'); void aiFeed.refresh(); }).catch(error => setNotice(errorMessage(error))); }} /></HStack>}
        <StackItem size="fill" className="terminal-area">
          {historyOpen && <CommandHistoryPane active={historyActive} findRequest={historyFind} />}
          {starting && <WorkspaceStart history={recent.data} pending={isDesktop && recent.isPending} historyError={recent.error ? errorMessage(recent.error) : recent.liveError}
            onRetry={recent.retryHistory} onOpenProfile={id => {
              const profile = query.data?.find(profile => profile.id === id);
              if (profile) openProfile(profile); else void navigate(`/connections/${id}/connect`);
            }} onLocateSession={id => {
              const session = sessions.find(session => session.id === id);
              if (!session || session.status === 'closed' || session.status === 'error') return false;
              showTerminal(id); return true;
            }} />}
          {sessions.map(session => { const runtime = getRuntime(session.id); return runtime ? <TerminalPane key={session.id} session={session} runtime={runtime} readOnly={takeovers.some(terminal => terminal.sessionId === session.id)} active={!historyActive && !activeAi && !editor.active && session.id === activeId} fontSize={fontSize}
            searchOpen={searchOpen} searchRequest={session.id === activeId ? searchRequest : 0} onCloseSearch={closeSearch} onOpenSearch={openSearch} onSaveSelection={saveSelection} /> : null; })}
          {aiTabs.map(info => <AiTerminalPane key={info.id} info={info} active={!historyActive && info.id === activeAiId} fontSize={fontSize}
            searchOpen={searchOpen} searchRequest={info.id === activeAiId ? searchRequest : 0} onOpenSearch={openSearch} onCloseSearch={closeSearch} />)}
          {editor.documents.map(file => <Suspense key={file.id} fallback={file.id === editor.activeId ? <Text className="panel-message">正在准备编辑器…</Text> : null}>
            <RemoteEditorPane file={file} active={!historyActive && file.id === editor.activeId} connected={sessions.some(session => session.id === file.sessionId && session.status === 'connected')} controller={editor} />
          </Suspense>)}
        </StackItem>
        {active?.credentialWarning && <Banner className="compact-feedback" status="warning" title="连接已建立，但凭据未保存" description={active.credentialWarning} isDismissable onDismiss={() => getRuntime(active.id)?.receive({type: 'credentialWarning', message: ''})} />}
        {!historyActive && !activeAi && active?.status === 'connecting' && <Banner className="compact-feedback" status="info" title="正在连接服务器…" />}
        {!historyActive && !activeAi && active?.status === 'closed' && <Banner className="compact-feedback" status="info" title="连接已断开" endContent={<Button label="重新连接" size="sm" onClick={reconnect} />} />}
        {!historyActive && !activeAi && active?.status === 'error' && <Banner className="compact-feedback" status="error" title="连接失败" description={active.message}
          endContent={<Button label="重新连接" size="sm" onClick={reconnect} />} />}
        <WorkspaceTools transfers={transfers} logs={logs} tab={toolTab} open={toolsOpen} onTab={openTool} onToggle={() => setToolsOpen(!toolsOpen)} onClear={clearCompleted} ai={ai} commands={<QuickCommandPanel readOnly={historyActive || Boolean(activeAi || activeTakeover)} onLocateTerminal={showTerminal} />} />
      </VStack>
    </IdeLayout>
    <Outlet />
    {takeoverSession && <AiTakeoverDialog key={takeoverSession.id} session={takeoverSession} onClose={() => setTakeoverSessionId(null)} onTakenOver={info => {
      setTakeoverSessionId(null); showTerminal(info.sessionId); setNotice(`原终端已交给 ${info.agentName} 接管，人工输入已锁定。`);
    }} />}
    {editor.documents.length > 0 && <Suspense fallback={null}><EditorDialogs controller={editor} /></Suspense>}
    {pendingHost && <HostKeyPrompt key={pendingHost.id} session={pendingHost} />}
    {!pendingHost && pendingProfile && location.pathname === '/' && <CredentialsDialog key={pendingProfile.profile.id} profile={pendingProfile.profile} ephemeral={pendingProfile.ephemeral} onClose={dismissPending} />}
    {about && <Dialog isOpen className="orbit-dialog about-dialog" width="calc(var(--orbit-unit) * 36)" padding={0} onOpenChange={setAbout}>
      <VStack className="orbit-dialog-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title="关于 OrbitShell" hasDivider onOpenChange={setAbout} />
        <HStack className="orbit-dialog-body about-content" gap={4} vAlign="center"><OrbitMark className="about-mark" />
          <VStack gap={2}><Text className="about-name">OrbitShell</Text><Text color="secondary">0.1.0</Text><Text>远程 SSH / SFTP 工作台</Text><Text type="supporting" color="secondary">Tauri · Rust · React · Astryx · xterm.js</Text></VStack>
        </HStack>
        <HStack className="orbit-dialog-footer" hAlign="end"><Button label="关闭" onClick={() => setAbout(false)} /></HStack>
      </VStack>
    </Dialog>}
  </>;
}
