import {useEffect, useRef, useState} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Selector} from '@astryxdesign/core/Selector';
import {CheckboxInput} from '@astryxdesign/core/CheckboxInput';
import {Banner} from '@astryxdesign/core/Banner';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import {Table, proportional} from '@astryxdesign/core/Table';
import type {TableColumn} from '@astryxdesign/core/Table';
import {ArrowLeft, Network, Search, Terminal, LockKeyhole, Copy, ExternalLink, ListChecks, MoreHorizontal} from 'lucide-react';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {usePreferences} from '../../app/preferences';
import {McpToolList} from '../workbench/mcp-tool-list';
import {executionLabels, formatElapsed, useExecutionFeed, useExecutions} from './execution-feed';
import type {ExecutionOutput, ExecutionRecord, ExecutionStatus} from './execution-feed';

const statusVariants = {
  running: 'warning', completed: 'success', failed: 'error', timedOut: 'warning', truncated: 'warning', cancelled: 'neutral', sent: 'success',
} as const;
function elapsed(record: ExecutionRecord, now: number) { return record.status === 'running' ? Math.max(record.elapsedMs, now - record.startedAt) : record.elapsedMs; }
function displayCommand(record: ExecutionRecord) { return record.command ?? '终端输入（内容隐藏）'; }
function ExecutionState({status}: {status: ExecutionStatus}) {
  return <HStack gap={1} vAlign="center" className="execution-status" data-status={status}><StatusDot label={executionLabels[status]} variant={statusVariants[status]} isPulsing={status === 'running'} /><Text>{executionLabels[status]}</Text></HStack>;
}

// Preserve parser state across packets: a split ANSI/OSC sequence must remain inert.
class PlainOutput {
  private mode: 'text' | 'escape' | 'csi' | 'osc' | 'oscEscape' = 'text';
  write(text: string) {
    let result = '';
    for (const character of text) {
      if (this.mode === 'oscEscape') { this.mode = character === '\\' ? 'text' : 'osc'; continue; }
      if (this.mode === 'osc') { if (character === '\x07') this.mode = 'text'; else if (character === '\x1b') this.mode = 'oscEscape'; continue; }
      if (this.mode === 'csi') { if (character >= '@' && character <= '~') this.mode = 'text'; continue; }
      if (this.mode === 'escape') { this.mode = character === '[' ? 'csi' : character === ']' ? 'osc' : 'text'; continue; }
      if (character === '\x1b') { this.mode = 'escape'; continue; }
      if (character === '\x9b') { this.mode = 'csi'; continue; }
      if (character === '\x9d') { this.mode = 'osc'; continue; }
      if (character === '\t' || character === '\n' || character === '\r' || (character >= ' ' && !(character >= '\x7f' && character <= '\x9f'))) result += character;
    }
    return result;
  }
}

type ExecutionPanelFeed = Pick<ReturnType<typeof useExecutionFeed>, 'subscribeOutput' | 'clear' | 'refresh'>;
type ReadExecutionOutput = (id: string, cursor: number) => Promise<ExecutionOutput>;
const readDesktopOutput: ReadExecutionOutput = (id, cursor) => desktopCommand<ExecutionOutput>('execution_output', {id, cursor});

function OutputView({record, feed, readOutput}: {record: ExecutionRecord; feed: ExecutionPanelFeed; readOutput: ReadExecutionOutput}) {
  const output = useRef<HTMLElement | null>(null);
  const loadRef = useRef<(() => void) | null>(null);
  const statusRef = useRef(record.status);
  statusRef.current = record.status;
  const [stream, setStream] = useState('all');
  const [follow, setFollow] = useState(true);
  const followRef = useRef(follow);
  followRef.current = follow;
  const [error, setError] = useState('');
  const [hasOutput, setHasOutput] = useState(false);

  useEffect(() => {
    const element = output.current;
    if (!element) return;
    element.replaceChildren();
    setError('');
    setHasOutput(false);
    let disposed = false;
    let cursor = 0;
    let busy = false;
    let again = false;
    let flushed = false;
    const decoders = {stdout: new TextDecoder(), stderr: new TextDecoder()};
    const parsers = {stdout: new PlainOutput(), stderr: new PlainOutput()};
    const append = (kind: 'stdout' | 'stderr', text: string) => {
      const safe = parsers[kind].write(text);
      if (stream !== 'all' && stream !== kind) return;
      if (!safe) return;
      if (!element.hasChildNodes()) setHasOutput(true);
      let line = element.lastElementChild;
      if (!(line instanceof HTMLElement) || line.dataset.stream !== kind) {
        line = document.createElement('code');
        line.setAttribute('data-stream', kind);
        line.className = kind === 'stderr' ? 'execution-stderr' : 'execution-stdout';
        element.append(line);
      }
      line.append(document.createTextNode(safe));
    };
    const load = async () => {
      if (disposed) return;
      if (busy) { again = true; return; }
      busy = true;
      try {
        do {
          again = false;
          const next = await readOutput(record.id, cursor);
          if (disposed) return;
          for (const chunk of next.chunks) append(chunk.stream, decoders[chunk.stream].decode(Uint8Array.from(chunk.data), {stream: true}));
          cursor = next.cursor;
          if (next.hasMore) again = true;
          else if (!flushed && statusRef.current !== 'running') {
            flushed = true;
            append('stdout', decoders.stdout.decode()); append('stderr', decoders.stderr.decode());
          }
          if (followRef.current) element.scrollTop = element.scrollHeight;
        } while (again && !disposed);
      } catch (failure) { if (!disposed) setError(errorMessage(failure)); }
      finally { busy = false; }
    };
    const request = () => { void load(); };
    loadRef.current = request;
    const stop = feed.subscribeOutput(record.id, request);
    request();
    return () => { disposed = true; stop(); loadRef.current = null; };
  }, [feed, record.id, stream, readOutput]);
  useEffect(() => { loadRef.current?.(); }, [record.status]);
  useEffect(() => { if (follow && output.current) output.current.scrollTop = output.current.scrollHeight; }, [follow]);

  return <VStack className="execution-output-section" gap={0}>
    <HStack className="execution-output-toolbar" gap={2} vAlign="center" wrap="wrap">
      <HStack className="choice-group execution-stream-choice" gap={0} role="group" aria-label="输出类型">
        {['all', 'stdout', 'stderr'].map(value => <Button key={value} label={value === 'all' ? '全部' : value} size="sm" variant="ghost" aria-pressed={value === stream} onClick={() => setStream(value)} />)}
      </HStack>
      <StackItem size="fill" /><CheckboxInput label="自动跟随" size="sm" value={follow} onChange={setFollow} />
      <Button label="复制输出" size="sm" variant="ghost" icon={<Copy size="1em" />} isDisabled={!hasOutput} onClick={() => {
        void navigator.clipboard.writeText(output.current?.textContent ?? '').catch(failure => setError(errorMessage(failure)));
      }} />
    </HStack>
    {error && <Banner className="compact-feedback" status="error" title="输出操作未完成" description={error} isDismissable onDismiss={() => setError('')} />}
    <StackItem ref={output} as="pre" size="fill" isScrollable className="execution-output" tabIndex={0} role="log" aria-live="off" aria-label="命令实时输出"
      data-placeholder={record.status === 'running' ? '等待命令输出…' : '暂无输出'} />
  </VStack>;
}

interface ExecutionPanelProps {
  activeId: string | null; availableSessionIds: string[]; onLocateSession: (id: string) => void;
  mcpEnabled: boolean; mcpPending: boolean; mcpError: string; mcpTools: string[]; onRefreshMcp: () => void;
}

export function AiExecutionPanel(props: ExecutionPanelProps) {
  const feed = useExecutionFeed();
  const snapshot = useExecutions();
  return <AiExecutionPanelView {...props} feed={feed} snapshot={snapshot} />;
}

export function AiExecutionPanelView({activeId, availableSessionIds, onLocateSession, mcpEnabled, mcpPending, mcpError, mcpTools, onRefreshMcp,
  feed, snapshot, readOutput = readDesktopOutput}: ExecutionPanelProps & {
  feed: ExecutionPanelFeed; snapshot: ReturnType<typeof useExecutions>; readOutput?: ReadExecutionOutput;
}) {
  const panelElement = useRef<HTMLDivElement | null>(null);
  const [compactRecords, setCompactRecords] = useState(true);
  useEffect(() => {
    const element = panelElement.current;
    if (!element) return;
    // The full six-column list needs 560px, about half the side-by-side dock.
    const observer = new ResizeObserver(([entry]) => { if (entry) setCompactRecords(entry.contentRect.width < 1120); });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const {uiDensity} = usePreferences();
  const [sessionFilter, setSessionFilter] = useState('all');
  const [statusFilter, setStatusFilter] = useState('all');
  const [search, setSearch] = useState('');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [now, setNow] = useState(Date.now);
  const hasRunning = snapshot.records.some(record => record.status === 'running');
  useEffect(() => {
    if (!hasRunning) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [hasRunning]);
  const knownSessions = [...new Map(snapshot.records.map(record => [record.sessionId, {value: record.sessionId, label: `${record.sessionName} · ${record.hostname}`}])).values()];
  const currentFilter = ['all', 'current', ...knownSessions.map(option => option.value)].includes(sessionFilter) ? sessionFilter : 'all';
  const words = search.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const records = snapshot.records.filter(record => (currentFilter === 'all' || record.sessionId === (currentFilter === 'current' ? activeId : currentFilter)) &&
    (statusFilter === 'all' || record.status === statusFilter) && words.every(word => `${displayCommand(record)} ${record.sessionName} ${record.hostname} ${record.tool}`.toLocaleLowerCase().includes(word)));
  const selected = records.find(record => record.id === selectedId) ?? records[0];
  const canLocate = selected ? availableSessionIds.includes(selected.sessionId) : false;
  const selectRecord = (id: string) => { setSelectedId(id); setDetailOpen(true); };
  const clearRecords = () => {
    setBusy(true); setError(''); void feed.clear().catch(failure => setError(errorMessage(failure))).finally(() => setBusy(false));
  };
  const copyCommand = () => { if (selected?.command) void navigator.clipboard.writeText(selected.command).catch(failure => setError(errorMessage(failure))); };
  const rows = records.map(record => ({...record}));
  const columns: TableColumn<(typeof rows)[number]>[] = [
    {key: 'startedAt', header: '时间', width: proportional(.85, {minWidth: 80}), renderCell: record => <Text hasTabularNumbers>{new Date(record.startedAt).toLocaleTimeString('zh-CN', {hour12: false})}</Text>},
    {key: 'sessionName', header: '所属会话', width: proportional(1, {minWidth: 110}), renderCell: record => <Text maxLines={1} hasTruncateTooltip>{record.sessionName}</Text>},
    {key: 'command', header: '执行命令', width: proportional(2, {minWidth: 160}), renderCell: record =>
      <Button className="execution-select-command mono-text" label={displayCommand(record)} tooltip={displayCommand(record)} size="sm" variant="ghost"
        aria-pressed={selected?.id === record.id} onClick={() => selectRecord(record.id)} />},
    {key: 'status', header: '状态', width: proportional(1, {minWidth: 84}), renderCell: record => <ExecutionState status={record.status} />},
    {key: 'exitCode', header: '退出码', width: proportional(.65, {minWidth: 54}), renderCell: record => <Text hasTabularNumbers>{record.exitCode ?? '—'}</Text>},
    {key: 'elapsedMs', header: '耗时', width: proportional(.8, {minWidth: 72}), renderCell: record => <Text hasTabularNumbers>{formatElapsed(elapsed(record, now))}</Text>},
  ];
  const visibleColumns = compactRecords ? columns.filter(column => ['sessionName', 'command', 'status'].includes(column.key)) : columns;
  return <VStack ref={panelElement} className="execution-panel" gap={0} height="100%">
    <HStack className="feature-toolbar execution-toolbar" gap={2} vAlign="center" wrap="wrap">
      <HStack gap={2} vAlign="center" className="execution-filters">
      <Text className="feature-toolbar-label">执行记录</Text>
      <HStack gap={1} vAlign="center" className="execution-mcp-indicator"><StatusDot label={mcpEnabled ? 'MCP 已开启' : 'MCP 已关闭'} variant={mcpEnabled ? 'success' : 'neutral'} /><Text className="execution-mcp-state">{mcpPending ? '读取 MCP…' : mcpEnabled ? 'MCP 已开启' : 'MCP 已关闭'}</Text></HStack>
      <Selector label="所属会话" isLabelHidden value={currentFilter} onChange={setSessionFilter} size="sm" width="calc(var(--orbit-unit) * 10)"
        options={[{value: 'all', label: '全部会话'}, {value: 'current', label: '当前会话'}, ...knownSessions]} />
      <Selector label="执行状态" isLabelHidden value={statusFilter} onChange={setStatusFilter} size="sm" width="calc(var(--orbit-unit) * 9)"
        options={[{value: 'all', label: '全部状态'}, ...Object.entries(executionLabels).map(([value, label]) => ({value, label}))]} />
      </HStack>
      <HStack gap={1} vAlign="center" className="execution-toolbar-actions">
        <TextInput label="搜索执行命令" isLabelHidden type="search" value={search} onChange={setSearch} placeholder="搜索执行命令…" startIcon={<Search size="1em" />} hasClear controlClassName="feature-search-control" />
        <DropdownMenu hasChevron={false} presentation="popover" alignment="end" button={{label: '执行记录操作', tooltip: '工具列表、MCP 配置与记录清理', icon: <MoreHorizontal />,
          isIconOnly: true, variant: 'ghost', size: 'sm', isDisabled: busy}} items={[
          {label: '工具列表', onClick: () => setToolsOpen(true)},
          {label: '配置 MCP', href: '/settings?section=mcp'},
          {type: 'divider'},
          {label: '清理已完成', isDisabled: !snapshot.records.some(record => record.status !== 'running'), onClick: clearRecords},
        ]} />
      </HStack>
    </HStack>
    {(snapshot.error || error) && <Banner className="compact-feedback" status="error" title="执行记录操作未完成" description={error || snapshot.error}
      endContent={<Button label="重试" size="sm" onClick={() => { setError(''); void feed.refresh(); }} />} />}
    {mcpError && <Banner className="compact-feedback" status="warning" title="无法读取 MCP 状态" description={mcpError} endContent={<Button label="重试" size="sm" onClick={onRefreshMcp} />} />}
    {records.length ? <HStack className="execution-split" data-detail-open={String(detailOpen)} gap={0} vAlign="stretch">
      <StackItem className="execution-record-list" size="fill" isScrollable aria-label="AI 执行记录">
        <Table className="execution-table" data={rows} columns={visibleColumns} idKey="id" density="compact" dividers="rows" textOverflow="truncate" aria-label="AI 命令执行记录"
          plugins={{selection: {transformBodyRow: (props, record) => ({...props, htmlProps: {...props.htmlProps,
            className: [props.htmlProps.className, 'execution-row'].filter(Boolean).join(' '), 'aria-selected': selected?.id === record.id,
            onClick: () => selectRecord(record.id),
          }})}}} />
      </StackItem>
      {selected && <VStack className="execution-detail" gap={0}>
        <VStack className="execution-detail-header" gap={2}>
          <HStack gap={2} vAlign="start"><IconButton className="execution-back" label="返回执行记录" tooltip="返回执行记录" icon={<ArrowLeft />} size="sm" variant="ghost" onClick={() => setDetailOpen(false)} />
            <Terminal size="1.1em" /><Text className="mono-text execution-command-title">{displayCommand(selected)}</Text><StackItem size="fill" /><ExecutionState status={selected.status} /></HStack>
          <Text className="execution-summary" hasTabularNumbers>会话：{selected.sessionName} · 退出码：{selected.exitCode ?? '—'} · {selected.status === 'running' ? '已运行' : '耗时'}：{formatElapsed(elapsed(selected, now))}</Text>
          <HStack className="execution-metadata" gap={3} wrap="wrap"><Text>会话：{selected.sessionName}</Text><Text color="secondary">分组：{selected.group}</Text></HStack>
          <Text className="execution-address mono-text" color="secondary" maxLines={1} hasTruncateTooltip>{selected.username}@{selected.hostname.includes(':') ? `[${selected.hostname}]` : selected.hostname}:{selected.port}</Text>
          <HStack className="execution-metadata" gap={3} wrap="wrap"><Text color="secondary">来源：{selected.tool}</Text><Text>退出码：{selected.exitCode ?? '—'}</Text>
            <Text hasTabularNumbers>{selected.tool === 'send_input' ? '发送耗时' : selected.status === 'running' ? '已运行' : '耗时'}：{formatElapsed(elapsed(selected, now))}</Text></HStack>
          <HStack className="execution-detail-actions" gap={1} hAlign="end" wrap="wrap"><Button label={canLocate ? '定位会话' : '会话已关闭'} size="sm" variant="ghost" icon={<ExternalLink size="1em" />} isDisabled={!canLocate} onClick={() => onLocateSession(selected.sessionId)} />
            {selected.command && <Button label="复制命令" size="sm" variant="ghost" icon={<Copy size="1em" />} onClick={copyCommand} />}</HStack>
          {selected.message && <Text className="execution-result-note" color="secondary">{selected.message}</Text>}
        </VStack>
        {selected.tool === 'send_input' ? <VStack className="execution-shared-output" gap={3} padding={3}><Text>会话输出（共享）</Text>
          <Text color="secondary">交互输入共用终端输出，不单独判断命令退出码。</Text><HStack gap={2} vAlign="center"><LockKeyhole size="1em" /><Text color="secondary">交互输入内容已隐藏</Text></HStack>
          <Button label="定位终端查看输出" size="sm" variant="ghost" isDisabled={!canLocate} onClick={() => onLocateSession(selected.sessionId)} />
        </VStack> : <OutputView key={selected.id} record={selected} feed={feed} readOutput={readOutput} />}
      </VStack>}
    </HStack> : <VStack className="feature-empty" hAlign="center" vAlign="center" gap={3}>
      <ListChecks className="muted-icon" /><Text>{snapshot.loading ? '正在读取执行记录…' : snapshot.records.length ? '没有匹配的执行记录' : '暂无 AI 执行记录'}</Text>
      <Text color="secondary">{snapshot.records.length ? '可调整会话、状态或命令筛选。' : isDesktop ? 'Agent 通过 MCP 执行命令后，记录将在这里实时显示。' : '在 OrbitShell 桌面端开启 MCP 后，可查看真实执行记录。'}</Text>
      {snapshot.records.length > 0 && <Button label="清除筛选" variant="ghost" size="sm" onClick={() => { setSessionFilter('all'); setStatusFilter('all'); setSearch(''); }} />}
    </VStack>}
    <Text className="feature-note execution-retention" color="secondary" maxLines={1} hasTruncateTooltip>仅保留本次运行的最近 50 条记录 · 单次输出最多 512 KiB · 清理不影响运行中的任务</Text>
    {toolsOpen && <Dialog isOpen className="orbit-dialog" data-density={uiDensity} width="calc(var(--orbit-unit) * 58)" padding={0} maxHeight="calc(100dvh - var(--orbit-unit) * 6)" onOpenChange={setToolsOpen}>
      <VStack className="orbit-dialog-layout" gap={0}><DialogHeader className="orbit-dialog-header" title="MCP 工具列表" hasDivider onOpenChange={setToolsOpen} startContent={<Network size="1em" />} />
        <VStack className="orbit-dialog-body" gap={3}>{mcpTools.length ? <McpToolList tools={mcpTools} /> : <Text color="secondary">{isDesktop ? '暂时无法读取工具列表，请刷新 MCP 状态。' : '工具列表在桌面端可用。'}</Text>}</VStack>
        <HStack className="orbit-dialog-footer" hAlign="end"><Button label="关闭" onClick={() => setToolsOpen(false)} /></HStack>
      </VStack>
    </Dialog>}
  </VStack>;
}
