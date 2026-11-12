import {useCallback, useEffect, useRef, useState} from 'react';
import {Outlet, useLocation, useNavigate, useSearchParams} from 'react-router';
import {useQuery} from '@tanstack/react-query';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {useResizable} from '@astryxdesign/core/Resizable';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text, Heading} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {Banner} from '@astryxdesign/core/Banner';
import {Plus, ShieldCheck, SquareTerminal, Link2} from 'lucide-react';
import {CredentialsDialog, useConnections} from '../connections/dialogs';
import {ConnectionExplorer} from '../connections/connection-explorer';
import type {ConnectionProfile} from '../connections/types';
import {useSessions} from '../terminal/session-provider';
import {TerminalPane} from '../terminal/terminal-pane';
import {SessionTabs} from '../terminal/session-tabs';
import type {ShellSession} from '../terminal/runtime';
import {FilePanel} from '../files/file-panel';
import {useTransfers} from '../files/transfer-provider';
import {usePreferences} from '../../app/preferences';
import {useThemePreference} from '../../app/theme';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {IdeLayout} from './ide-layout';
import {ActivityRail, IdeStatusbar, IdeTitlebar, OrbitMark, SessionContextBar} from './ide-chrome';
import type {IdeMenu, ToolTab} from './ide-chrome';
import {WorkspaceTools} from './workspace-tools';
import {AiExecutionPanel} from '../ai/execution-panel';
import {useExecutions} from '../ai/execution-feed';
import {QuickCommandPanel} from '../quick-commands/quick-commands';
import {useQuickCommandDraft} from '../quick-commands/command-draft';
import type {ConnectionLog} from './workspace-tools';

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

function Welcome() {
  return <VStack className="welcome" height="100%" hAlign="center" vAlign="center" gap={4} padding={6}>
    <SquareTerminal className="welcome-mark" />
    <Heading level={1}>开始远程会话</Heading>
    <Text color="secondary">从左侧选择连接，或添加一台服务器。</Text>
    <HStack gap={3}><Button label="新建连接" href="/connections/new" variant="primary" icon={<Plus size="1em" />} /><Button label="打开会话链接" href="/settings?section=bastion" icon={<Link2 size="1em" />} /></HStack>
    <Text type="supporting" color="secondary">Ctrl + Shift + T 新建连接 · Ctrl + J 工作区工具</Text>
    {!isDesktop && <Text type="supporting" color="secondary">浏览器界面预览 · 远程连接请使用桌面端</Text>}
  </VStack>;
}

export default function Workbench() {
  const navigate = useNavigate();
  const location = useLocation();
  const [params, setParams] = useSearchParams();
  const query = useConnections();
  const {sessions, activeId, setActiveId, getRuntime, close, pendingProfile, dismissPending, launchError, clearLaunchError} = useSessions();
  const {transfers, clearCompleted} = useTransfers();
  const {fontSize, setFontSize, showServers, setShowServers, showFiles, setShowFiles} = usePreferences();
  const theme = useThemePreference();
  const [notice, setNotice] = useState('');
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchRequest, setSearchRequest] = useState(0);
  const [toolsOpen, setToolsOpen] = useState(true);
  const [toolTab, setToolTab] = useState<ToolTab>('transfers');
  const [about, setAbout] = useState(false);
  const [logs, setLogs] = useState<ConnectionLog[]>([]);
  const lastStates = useRef(new Map<string, string>());
  const knownTransfers = useRef(new Set<string>());
  const knownExecutions = useRef(new Set<string>());
  const executions = useExecutions();
  const {stage: stageCommand} = useQuickCommandDraft();
  const active = sessions.find(session => session.id === activeId) ?? null;
  const pendingHost = sessions.find(session => session.hostKey);
  const startPanel = useResizable({defaultSize: '15%', minSize: 200, maxSize: 380, autoSaveId: 'orbitshell.ide.connections.v2'});
  const endPanel = useResizable({defaultSize: '20%', minSize: 260, maxSize: 480, autoSaveId: 'orbitshell.ide.files.v2'});
  const mcp = useQuery<{enabled: boolean; url: string | null; tools: string[]}>({
    queryKey: ['mcp-status'], queryFn: () => desktopCommand('mcp_status'), enabled: isDesktop, retry: false,
  });
  const openSearch = useCallback(() => { setSearchOpen(true); setSearchRequest(value => value + 1); }, []);
  const closeSearch = useCallback(() => setSearchOpen(false), []);
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
    if (existing) setActiveId(existing.id); else void navigate(`/connections/${profile.id}/connect`);
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
    const added = executions.records.some(record => !knownExecutions.current.has(record.id));
    knownExecutions.current = new Set(executions.records.map(record => record.id));
    if (added) { setToolsOpen(true); setToolTab('ai'); }
  }, [executions.records]);
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => {
      if (event.isComposing || location.pathname !== '/' || pendingHost || pendingProfile) return;
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest('[role="menu"]')) return;
      const consume = () => { event.preventDefault(); event.stopPropagation(); };
      if (event.key === 'Escape' && searchOpen && activeId) { consume(); closeSearch(); return; }
      if (!event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.shiftKey && event.code === 'KeyT') { consume(); void navigate('/connections/new'); return; }
      if (event.shiftKey && event.code === 'KeyW' && activeId) { consume(); close(activeId); return; }
      if (target?.closest('input, textarea, [contenteditable="true"]') && !target.closest('.terminal-pane')) return;
      if (event.code === 'KeyJ') { consume(); setToolsOpen(value => !value); return; }
      if (event.code === 'KeyB') { consume(); setShowServers(!showServers); return; }
      if (!activeId) return;
      if (event.code === 'KeyF') { consume(); openSearch(); return; }
      if (event.code === 'Equal' || event.code === 'NumpadAdd') { consume(); setFontSize(value => value + 1); }
      else if (event.code === 'Minus' || event.code === 'NumpadSubtract') { consume(); setFontSize(value => value - 1); }
      else if (event.code === 'Digit0' || event.code === 'Numpad0') { consume(); setFontSize(17); }
    };
    window.addEventListener('keydown', keyboard, true);
    return () => window.removeEventListener('keydown', keyboard, true);
  }, [navigate, location.pathname, activeId, pendingHost, pendingProfile, close, openSearch, closeSearch, searchOpen, setFontSize, setShowServers, showServers]);

  function windowAction(action: 'minimize' | 'maximize' | 'close') {
    if (!isDesktop) return;
    const current = getCurrentWindow();
    void (action === 'minimize' ? current.minimize() : action === 'maximize' ? current.toggleMaximize() : current.close()).catch(error => setNotice(errorMessage(error)));
  }
  const menus: IdeMenu[] = [
    {label: '文件', items: [
      {label: '新建连接', endContent: <Text className="menu-shortcut">Ctrl + Shift + T</Text>, onClick: () => { void navigate('/connections/new'); }},
      {label: '管理连接', onClick: () => { void navigate('/connections'); }},
      {label: '打开会话链接', onClick: () => { void navigate('/settings?section=bastion'); }},
      {type: 'divider'},
      {label: '设置', onClick: () => { void navigate('/settings'); }},
      {label: '退出', onClick: () => windowAction('close')},
    ]},
    {label: '编辑', items: [
      {label: '复制终端选区', endContent: <Text className="menu-shortcut">Ctrl + C</Text>, isDisabled: !active, onClick: () => {
        const text = active ? getRuntime(active.id)?.terminal?.getSelection() : '';
        if (text) void navigator.clipboard.writeText(text).catch(error => setNotice(errorMessage(error)));
      }},
      {label: '粘贴到终端', endContent: <Text className="menu-shortcut">Ctrl + V</Text>, isDisabled: active?.status !== 'connected', onClick: () => {
        void navigator.clipboard.readText().then(text => {
          if (active) getRuntime(active.id)?.terminal?.paste(text);
        }).catch(error => setNotice(errorMessage(error)));
      }},
      {label: '查找终端内容', endContent: <Text className="menu-shortcut">Ctrl + F</Text>, isDisabled: !active, onClick: openSearch},
      {label: '快捷指令管理', onClick: () => { void navigate('/commands'); }},
      {label: '清空终端显示', isDisabled: !active, onClick: () => { if (active) getRuntime(active.id)?.terminal?.clear(); }},
    ]},
    {label: '视图', items: [
      {label: showServers ? '收起连接列表' : '显示连接列表', endContent: <Text className="menu-shortcut">Ctrl + B</Text>, onClick: () => setShowServers(!showServers)},
      {label: showFiles ? '收起远程文件' : '显示远程文件', onClick: () => setShowFiles(!showFiles)},
      {label: toolsOpen ? '折叠工具面板' : '展开工具面板', endContent: <Text className="menu-shortcut">Ctrl + J</Text>, onClick: () => setToolsOpen(!toolsOpen)},
      {label: 'AI 执行记录', onClick: () => openTool('ai')},
      {label: '快捷指令', onClick: () => openTool('commands')},
      {type: 'divider'},
      {label: '深色主题', onClick: () => theme.setMode('dark')},
      {label: '浅色主题', onClick: () => theme.setMode('light')},
      {label: '跟随系统', onClick: () => theme.setMode('system')},
    ]},
    {label: '会话', items: [
      {label: '打开会话', onClick: () => { void navigate('/connections'); }},
      {label: '重新连接', isDisabled: !active || active.status === 'connecting' || active.status === 'hostKey', onClick: reconnect},
      {label: '关闭当前会话', endContent: <Text className="menu-shortcut">Ctrl + Shift + W</Text>, isDisabled: !active, onClick: () => { if (active) close(active.id); }},
      {label: '断开连接', isDisabled: active?.status !== 'connected', onClick: () => {
        if (active) disconnectSession(active.id);
      }},
    ]},
    {label: '帮助', items: [
      {label: '快捷键', onClick: () => { void navigate('/settings'); }},
      {label: '关于 OrbitShell', onClick: () => setAbout(true)},
    ]},
  ];
  const ai = <AiExecutionPanel activeId={activeId} availableSessionIds={sessions.map(session => session.id)} onLocateSession={id => {
    setActiveId(id); setSearchOpen(false);
    requestAnimationFrame(() => getRuntime(id)?.terminal?.focus());
  }} mcpEnabled={Boolean(mcp.data?.enabled)} mcpPending={isDesktop && mcp.isPending} mcpError={mcp.error ? errorMessage(mcp.error) : ''}
    mcpTools={mcp.data?.tools ?? []} onRefreshMcp={() => { void mcp.refetch(); }} />;

  return <>
    <IdeLayout titlebar={<IdeTitlebar menus={menus} onWindow={windowAction} />}
      activity={<ActivityRail showServers={showServers} showFiles={showFiles} toolTab={toolTab} toolsOpen={toolsOpen} onServers={() => setShowServers(!showServers)}
        onFiles={() => setShowFiles(!showFiles)} onSessions={() => { void navigate('/connections'); }} onTool={openTool} onSettings={() => { void navigate('/settings'); }} />}
      sidebar={<ConnectionExplorer profiles={query.data ?? []} sessions={sessions} activeId={activeId} search={params.get('q') ?? ''} onSearch={value => {
        const next = new URLSearchParams(params); if (value) next.set('q', value); else next.delete('q'); void setParams(next, {replace: true});
      }} onOpen={openProfile} onCreate={() => { void navigate('/connections/new'); }} onManage={() => { void navigate('/connections'); }} pending={query.isPending}
        {...(query.error ? {error: errorMessage(query.error)} : {})} onRetry={() => { void query.refetch(); }} />}
      files={<FilePanel session={active} onCollapse={() => setShowFiles(false)} />}
      footer={<IdeStatusbar active={active} mcpEnabled={Boolean(mcp.data?.enabled)} />}
      showServers={showServers} showFiles={showFiles} serverWidth={startPanel.size} filesWidth={endPanel.size} serverResize={startPanel.props} filesResize={endPanel.props}>
      <VStack className="workspace-stack" height="100%" gap={0}>
        <HStack className="session-tabs" gap={0} vAlign="center"><StackItem className="session-strip-container" size="fill"><SessionTabs sessions={sessions} activeId={activeId} onSelect={setActiveId} onClose={close} onReconnect={reconnectSession} onDisconnect={disconnectSession} onCreate={() => { void navigate('/connections'); }} /></StackItem></HStack>
        {notice && <Banner status="info" title={notice} isDismissable onDismiss={() => setNotice('')} />}
        {launchError && <Banner status="error" title="无法打开外部会话" description={launchError} isDismissable onDismiss={clearLaunchError} />}
        {pendingProfile && location.pathname !== '/' && <Banner status="info" title="有连接正在等待登录" endContent={<Button label="前往登录" href="/" size="sm" />} />}
        {active && <SessionContextBar session={active} onSearch={openSearch} onReconnect={reconnect} onFiles={() => setShowFiles(!showFiles)} />}
        <StackItem size="fill" className="terminal-area">
          {!sessions.length && <Welcome />}
          {sessions.map(session => { const runtime = getRuntime(session.id); return runtime ? <TerminalPane key={session.id} session={session} runtime={runtime} active={session.id === activeId} fontSize={fontSize}
            searchOpen={searchOpen} searchRequest={session.id === activeId ? searchRequest : 0} onCloseSearch={closeSearch} onOpenSearch={openSearch} onSaveSelection={saveSelection} /> : null; })}
        </StackItem>
        {active?.credentialWarning && <Banner className="compact-feedback" status="warning" title="连接已建立，但凭据未保存" description={active.credentialWarning} isDismissable onDismiss={() => getRuntime(active.id)?.receive({type: 'credentialWarning', message: ''})} />}
        {active?.status === 'connecting' && <Banner className="compact-feedback" status="info" title="正在连接服务器…" />}
        {active?.status === 'closed' && <Banner className="compact-feedback" status="info" title="连接已断开" endContent={<Button label="重新连接" size="sm" onClick={reconnect} />} />}
        {active?.status === 'error' && <Banner className="compact-feedback" status="error" title="连接失败" description={active.message}
          endContent={<Button label="重新连接" size="sm" onClick={reconnect} />} />}
        <WorkspaceTools transfers={transfers} logs={logs} tab={toolTab} open={toolsOpen} onTab={openTool} onToggle={() => setToolsOpen(!toolsOpen)} onClear={clearCompleted} ai={ai} commands={<QuickCommandPanel />} />
      </VStack>
    </IdeLayout>
    <Outlet />
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
