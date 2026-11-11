// Development-only visual fixture. Never loads saved connections or credentials.
import {useEffect, useRef, useState} from 'react';
import {useNavigate} from 'react-router';
import {useResizable} from '@astryxdesign/core/Resizable';
import {VStack, HStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {IdeLayout} from './ide-layout';
import {IdeTitlebar, ActivityRail, IdeStatusbar, SessionContextBar} from './ide-chrome';
import type {ToolTab} from './ide-chrome';
import {ConnectionExplorer} from '../connections/connection-explorer';
import type {ConnectionProfile} from '../connections/types';
import {SessionTabs} from '../terminal/session-tabs';
import {TerminalPane} from '../terminal/terminal-pane';
import {TerminalRuntime} from '../terminal/runtime';
import type {ShellSession} from '../terminal/runtime';
import {FileBrowserView} from '../files/file-browser-view';
import type {RemoteEntry} from '../files/file-utils';
import type {Transfer} from '../files/transfer-provider';
import {WorkspaceTools} from './workspace-tools';

const profiles: ConnectionProfile[] = [
  ['api-prod-01', '生产环境', '192.0.2.24'], ['db-primary', '生产环境', '192.0.2.25'],
  ['cache-01', '生产环境', '192.0.2.26'], ['bastion', '生产环境', '198.51.100.10'],
  ['staging', '开发环境', '198.51.100.20'], ['dev-local', '开发环境', '198.51.100.21'],
].map(([name = '', group = '', hostname = ''], index) => ({id: `design-profile-${index}`, name, group, hostname, port: 22, username: 'deploy', authMethod: 'password', privateKeyPath: null}));
const initialSessions: ShellSession[] = [profiles[0]!, profiles[1]!, profiles[4]!].map(profile => ({
  id: profile.id, profile, status: 'connected', message: '已连接', hostKey: null, columns: 120, rows: 36,
}));
const entries: RemoteEntry[] = [
  ...['config', 'logs', 'migrations', 'src', 'static'].map(name => ({name, path: `/srv/orbit-api/${name}`, isDirectory: true, isSymlink: false, size: 0})),
  ...[['compose.yml', 1229], ['Dockerfile', 780], ['README.md', 3686], ['package.json', 1843], ['nginx.conf', 2970]].map(([name, size]) =>
    ({name: String(name), path: `/srv/orbit-api/${name}`, isDirectory: false, isSymlink: false, size: Number(size)})),
];
const initialTransfers: Transfer[] = [
  {id: 'design-upload', name: 'release.tar.gz', direction: 'upload', status: 'running', bytes: 19503514, total: 30474240, speed: 8598323, message: '传输中'},
  {id: 'design-download', name: 'app.log', direction: 'download', status: 'done', bytes: 2516582, total: 2516582, speed: 0, message: '已完成'},
];
const prompt = '\x1b[1;32mdeploy@api-prod-01\x1b[0m:\x1b[1;34m/srv/orbit-api\x1b[0m$ ';
const terminalText = [
  prompt + 'docker compose ps',
  'NAME             IMAGE                STATUS          PORTS',
  'orbit-api        orbit/api:1.6.0       Up (healthy)    0.0.0.0:8080->8080/tcp',
  'orbit-worker     orbit/worker:1.6.0    Up (healthy)    —',
  'orbit-redis      redis:7-alpine       Up (healthy)    0.0.0.0:6379->6379/tcp',
  '', prompt + 'df -h /srv', 'Filesystem   Size   Used   Avail  Use% Mounted on',
  '/dev/sda2    100G    42G     54G   44% /srv',
  '', prompt + 'docker compose logs --tail=8 api',
  ...[
    ['10:14:37', 'Starting Orbit API v1.6.0'], ['10:14:37', 'Loading configuration from /app/config/app.yml'],
    ['10:14:38', 'Database connected (postgres://orbit@db:5432/orbit)'], ['10:14:38', 'Redis connected (redis://orbit-redis:6379/0)'],
    ['10:14:39', 'HTTP server listening on 0.0.0.0:8080'], ['10:15:02', '[req] GET /api/health   200  15 ms'],
    ['10:15:17', '[req] GET /api/v1/projects   200  42 ms'], ['10:15:23', '[req] POST /api/v1/deploy   200  318 ms'],
  ].map(([time, message]) => `2025-01-26 ${time}  \x1b[36mapi\x1b[0m  | INFO  ${message}`),
  '', prompt,
].join('\r\n');

export default function DesignReview() {
  const navigate = useNavigate();
  const [search, setSearch] = useState('');
  const [sessions, setSessions] = useState(initialSessions);
  const [activeId, setActive] = useState(initialSessions[0]!.id);
  const active = sessions.find(session => session.id === activeId) ?? sessions[0] ?? null;
  const [files, setFiles] = useState(true);
  const [servers, setServers] = useState(true);
  const [tools, setTools] = useState(true);
  const [tab, setTab] = useState<ToolTab>('transfers');
  const [transfers, setTransfers] = useState(initialTransfers);
  const [path, setPath] = useState('/srv/orbit-api');
  const [selected, setSelected] = useState('/srv/orbit-api/README.md');
  const [hidden, setHidden] = useState(false);
  const [compact, setCompact] = useState(true);
  const [find, setFind] = useState(false);
  const runtime = useRef<TerminalRuntime | null>(null);
  if (!runtime.current) runtime.current = new TerminalRuntime('design-terminal', () => {});
  const left = useResizable({defaultSize: 276, minSize: 200, maxSize: 380});
  const right = useResizable({defaultSize: 350, minSize: 260, maxSize: 480});
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      if (runtime.current?.terminal) runtime.current.terminal.options.cursorBlink = false;
      runtime.current?.receive({type: 'data', data: Array.from(new TextEncoder().encode(terminalText))});
    });
    return () => cancelAnimationFrame(frame);
  }, []);
  const openTool = (value: ToolTab) => { setTab(value); setTools(true); };
  return <IdeLayout titlebar={<IdeTitlebar menus={['文件', '编辑', '视图', '会话', '帮助'].map(label => ({label, items: [{label: '返回工作台', onClick: () => { void navigate('/'); }}]}))} onWindow={() => {}} />}
    activity={<ActivityRail showServers={servers} showFiles={files} toolTab={tab} toolsOpen={tools} onServers={() => setServers(!servers)} onFiles={() => setFiles(!files)}
      onSessions={() => { void navigate('/connections'); }} onTool={openTool} onSettings={() => { void navigate('/settings'); }} />}
    sidebar={<ConnectionExplorer profiles={profiles} sessions={initialSessions} activeId={active?.id ?? null} search={search} onSearch={setSearch}
      onOpen={profile => { const session = sessions.find(row => row.profile.id === profile.id); if (session) setActive(session.id); }}
      onCreate={() => { void navigate('/connections/new'); }} onManage={() => { void navigate('/connections'); }} />}
    files={<FileBrowserView connected entries={entries} path={path} selectedPath={selected} compact={compact} showHidden={hidden}
      onPath={setPath} onNavigate={setPath} onRefresh={() => {}} onHidden={() => setHidden(!hidden)} onDensity={() => setCompact(!compact)}
      onUpload={() => {}} onDownload={() => {}} onSelect={entry => setSelected(entry.path)} onCollapse={() => setFiles(false)} />}
    footer={<IdeStatusbar active={active} mcpEnabled={false} />}
    showServers={servers} showFiles={files} serverWidth={left.size} filesWidth={right.size} serverResize={left.props} filesResize={right.props}>
    <VStack className="workspace-stack" height="100%" gap={0}>
      <HStack className="session-tabs" gap={0} vAlign="center"><StackItem className="session-strip-container" size="fill"><SessionTabs sessions={sessions} activeId={active?.id ?? null} onSelect={setActive} onClose={id => setSessions(previous => previous.filter(row => row.id !== id))} onCreate={() => { void navigate('/connections'); }} /></StackItem></HStack>
      {active && <SessionContextBar session={active} onSearch={() => setFind(true)} onReconnect={() => {}} onFiles={() => setFiles(!files)} />}
      <StackItem size="fill" className="terminal-area">
        {active && runtime.current && <TerminalPane session={active} runtime={runtime.current} active fontSize={17} searchOpen={find} searchRequest={0} onCloseSearch={() => setFind(false)} />}
      </StackItem>
      <WorkspaceTools transfers={transfers} logs={[]} tab={tab} open={tools} onTab={openTool} onToggle={() => setTools(!tools)} onClear={() => setTransfers(previous => previous.filter(row => row.status === 'running'))}
        ai={<Text>本页面仅用于设计比对，未连接真实服务器。</Text>} />
    </VStack>
  </IdeLayout>;
}
