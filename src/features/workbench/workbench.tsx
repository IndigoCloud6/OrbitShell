import {useCallback, useEffect, useRef, useState} from 'react';
import {Outlet, useLocation, useNavigate, useSearchParams} from 'react-router';
import {useQuery} from '@tanstack/react-query';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {useResizable} from '@astryxdesign/core/Resizable';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text, Heading} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {Banner} from '@astryxdesign/core/Banner';
import {Plus, ShieldCheck, SquareTerminal, Link2, Network} from 'lucide-react';
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
import {ActivityRail, IdeStatusbar, IdeTitlebar, SessionContextBar} from './ide-chrome';
import type {IdeMenu, ToolTab} from './ide-chrome';
import {WorkspaceTools} from './workspace-tools';
import type {ConnectionLog} from './workspace-tools';

function HostKeyPrompt({session}: {session: ShellSession}) {
  const {trustHost} = useSessions();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const answer = async (accept: boolean) => {
    setBusy(true); setError('');
    try { await trustHost(session.id, accept); } catch (failure) { setError(errorMessage(failure)); } finally { setBusy(false); }
  };
  return <Dialog isOpen purpose="required" width="36rem" onOpenChange={() => {}}>
    <DialogHeader title="确认服务器身份" subtitle={`${session.hostKey?.hostname}:${session.hostKey?.port}`} startContent={<ShieldCheck size="1em" />} />
    <VStack gap={4}><Text>这是第一次连接这台服务器。请核对主机指纹，确认后会保存到本机。</Text>
      <TextInput label="密钥算法" value={session.hostKey?.algorithm ?? ''} isReadOnly />
      <TextInput label="SHA-256 指纹" value={session.hostKey?.fingerprint ?? ''} isReadOnly />
      {error && <Banner status="error" title="确认未完成" description={error} />}
      <HStack gap={2} hAlign="end"><Button label="取消连接" isDisabled={busy} onClick={() => { void answer(false); }} /><Button label="信任并连接" variant="primary" isLoading={busy} onClick={() => { void answer(true); }} /></HStack>
    </VStack>
  </Dialog>;
}

function Welcome() {
  return <VStack className="welcome" height="100%" hAlign="center" vAlign="center" gap={4} padding={6}>
    <SquareTerminal className="welcome-mark" />
    <Heading level={1}>开始远程会话</Heading>
    <Text color="secondary">从左侧选择连接，或添加一台服务器。</Text>
    <HStack gap={3}><Button label="新建连接" href="/connections/new" icon={<Plus size="1em" />} /><Button label="打开会话链接" href="/settings?section=bastion" icon={<Link2 size="1em" />} /></HStack>
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
  const active = sessions.find(session => session.id === activeId) ?? null;
  const pendingHost = sessions.find(session => session.hostKey);
  const startPanel = useResizable({defaultSize: '17.4%', minSize: 200, maxSize: 380, autoSaveId: 'orbitshell.ide.connections.v2'});
  const endPanel = useResizable({defaultSize: '22.1%', minSize: 260, maxSize: 480, autoSaveId: 'orbitshell.ide.files.v2'});
  const mcp = useQuery<{enabled: boolean; url: string | null; tools: string[]}>({
    queryKey: ['mcp-status'], queryFn: () => desktopCommand('mcp_status'), enabled: isDesktop, retry: false,
  });
  const openSearch = useCallback(() => { setSearchOpen(true); setSearchRequest(value => value + 1); }, []);
  const closeSearch = useCallback(() => setSearchOpen(false), []);
  const openTool = (tab: ToolTab) => { setToolTab(tab); setToolsOpen(true); };
  const reconnect = () => { if (active) void navigate(`/connections/${active.profile.id}/connect`); };
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
    const keyboard = (event: KeyboardEvent) => {
      if (event.isComposing || location.pathname !== '/' || pendingHost || pendingProfile) return;
      const consume = () => { event.preventDefault(); event.stopPropagation(); };
      if (event.key === 'Escape' && searchOpen && activeId) { consume(); closeSearch(); return; }
      if (!event.ctrlKey || event.altKey || event.metaKey) return;
      if (event.shiftKey && event.code === 'KeyT') { consume(); void navigate('/connections/new'); return; }
      if (event.shiftKey && event.code === 'KeyW' && activeId) { consume(); close(activeId); return; }
      const target = event.target instanceof Element ? event.target : null;
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
    {label: '文件', items: [{label: '新建连接', onClick: () => { void navigate('/connections/new'); }}, {label: '管理连接', onClick: () => { void navigate('/connections'); }}, {label: '打开会话链接', onClick: () => { void navigate('/settings?section=bastion'); }}, {type: 'divider'}, {label: '设置', onClick: () => { void navigate('/settings'); }}, {label: '退出', onClick: () => windowAction('close')}]},
    {label: '编辑', items: [{label: '复制终端选区', isDisabled: !active, onClick: () => { const text = active ? getRuntime(active.id)?.terminal?.getSelection() : ''; if (text) void navigator.clipboard.writeText(text).catch(error => setNotice(errorMessage(error))); }},
      {label: '粘贴到终端', isDisabled: active?.status !== 'connected', onClick: () => { void navigator.clipboard.readText().then(text => { if (active) getRuntime(active.id)?.terminal?.paste(text); }).catch(error => setNotice(errorMessage(error))); }},
      {label: '查找终端内容', isDisabled: !active, onClick: openSearch}, {label: '清空终端显示', isDisabled: !active, onClick: () => { if (active) getRuntime(active.id)?.terminal?.clear(); }}]},
    {label: '视图', items: [{label: showServers ? '收起连接列表' : '显示连接列表', onClick: () => setShowServers(!showServers)}, {label: showFiles ? '收起远程文件' : '显示远程文件', onClick: () => setShowFiles(!showFiles)}, {label: toolsOpen ? '折叠工具面板' : '展开工具面板', onClick: () => setToolsOpen(!toolsOpen)}, {type: 'divider'}, {label: '深色主题', onClick: () => theme.setMode('dark')}, {label: '浅色主题', onClick: () => theme.setMode('light')}, {label: '跟随系统', onClick: () => theme.setMode('system')}]},
    {label: '会话', items: [{label: '打开会话', onClick: () => { void navigate('/connections'); }}, {label: '重新连接', isDisabled: !active, onClick: reconnect}, {label: '关闭当前会话', isDisabled: !active, onClick: () => { if (active) close(active.id); }},
      {label: '断开连接', isDisabled: active?.status !== 'connected', onClick: () => { if (active) void desktopCommand('ssh_disconnect', {sessionId: active.id}).catch(error => setNotice(errorMessage(error))); }}]},
    {label: '帮助', items: [{label: '快捷键', onClick: () => { void navigate('/settings'); }}, {label: '关于 OrbitShell', onClick: () => setAbout(true)}]},
  ];
  const ai = <><HStack gap={2} vAlign="center"><Network size="1em" /><Text>MCP {mcp.data?.enabled ? '已开启' : '未开启'}</Text><StackItem size="fill" /><Button label="配置 AI 与 MCP" size="sm" href="/settings?section=mcp" /></HStack>
    <Text color="secondary" type="supporting">{mcp.error ? errorMessage(mcp.error) : mcp.data?.enabled ? `本机服务：${mcp.data.url}` : '在设置中开启 MCP，让 AI 操作工作台中的 SSH 会话。'}</Text>
    {mcp.data?.enabled && <Text type="code">{mcp.data.tools.join('  ·  ')}</Text>}</>;

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
        <HStack className="session-tabs" gap={0} vAlign="center"><StackItem className="session-strip-container" size="fill"><SessionTabs sessions={sessions} activeId={activeId} onSelect={setActiveId} onClose={close} onCreate={() => { void navigate('/connections'); }} /></StackItem></HStack>
        {notice && <Banner status="info" title={notice} isDismissable onDismiss={() => setNotice('')} />}
        {launchError && <Banner status="error" title="无法打开外部会话" description={launchError} isDismissable onDismiss={clearLaunchError} />}
        {pendingProfile && location.pathname !== '/' && <Banner status="info" title="有连接正在等待登录" endContent={<Button label="前往登录" href="/" size="sm" />} />}
        {active && <SessionContextBar session={active} onSearch={openSearch} onReconnect={reconnect} onFiles={() => setShowFiles(!showFiles)} />}
        <StackItem size="fill" className="terminal-area">
          {!sessions.length && <Welcome />}
          {sessions.map(session => { const runtime = getRuntime(session.id); return runtime ? <TerminalPane key={session.id} session={session} runtime={runtime} active={session.id === activeId} fontSize={fontSize}
            searchOpen={searchOpen} searchRequest={session.id === activeId ? searchRequest : 0} onCloseSearch={closeSearch} /> : null; })}
        </StackItem>
        {active?.credentialWarning && <Banner status="warning" title="密码未保存" description={active.credentialWarning} isDismissable onDismiss={() => getRuntime(active.id)?.receive({type: 'credentialWarning', message: ''})} />}
        {active?.status === 'error' && <Banner status="error" title="连接失败" description={active.message} />}
        <WorkspaceTools transfers={transfers} logs={logs} tab={toolTab} open={toolsOpen} onTab={openTool} onToggle={() => setToolsOpen(!toolsOpen)} onClear={clearCompleted} ai={ai} />
      </VStack>
    </IdeLayout>
    <Outlet />
    {pendingHost && <HostKeyPrompt key={pendingHost.id} session={pendingHost} />}
    {!pendingHost && pendingProfile && location.pathname === '/' && <CredentialsDialog key={pendingProfile.profile.id} profile={pendingProfile.profile} ephemeral={pendingProfile.ephemeral} onClose={dismissPending} />}
    {about && <Dialog isOpen width="28rem" onOpenChange={setAbout}><DialogHeader title="OrbitShell" subtitle="远程工作区 · 0.1.0" onOpenChange={setAbout} /><VStack gap={3}><Text>基于 Tauri、Rust、Astryx 与 xterm.js 的 SSH 工作台。</Text><Text color="secondary">SSH 多会话 · SFTP 文件传输 · AI 与 MCP</Text><Button label="关闭" onClick={() => setAbout(false)} /></VStack></Dialog>}
  </>;
}
