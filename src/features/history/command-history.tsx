import {useEffect, useRef, useState} from 'react';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {listen} from '@tauri-apps/api/event';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Selector} from '@astryxdesign/core/Selector';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {List, ListItem} from '@astryxdesign/core/List';
import {Table, proportional, pixel} from '@astryxdesign/core/Table';
import {Banner} from '@astryxdesign/core/Banner';
import {Bot, Check, Copy, History, Info, Layers, RefreshCw, Search, Server, UserRound} from 'lucide-react';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {usePreferences} from '../../app/preferences';

export const HISTORY_TAB_ID = 'orbit-command-history';
interface Entry extends Record<string, unknown> {
  id: string; hostKey: string; hostname: string; port: number; username: string; sessionName: string;
  source: 'human' | 'ai'; agentName: string | null; command: string; createdAt: number; redacted: boolean; temporary: boolean;
}
interface Host {key: string; hostname: string; port: number; name: string; count: number; temporary: boolean}
interface Snapshot {entries: Entry[]; hosts: Host[]; total: number; count: number; warning: string}
const recordHelp = '仅记录通过 OrbitShell 提交的命令，不读取远端 history。人工命令需可识别的 Shell 提示符，密码提示与全屏交互程序不记录。普通连接历史保存在本机，最多 10,000 条；临时会话仅保留本次运行记录。';
const dateFormat = new Intl.DateTimeFormat('zh-CN', {year: 'numeric', month: '2-digit', day: '2-digit'});
const timeFormat = new Intl.DateTimeFormat('zh-CN', {hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false});
const address = (hostname: string, port: number) => `${hostname.includes(':') ? `[${hostname}]` : hostname}:${port}`;

export function CommandHistoryPane({active, findRequest}: {active: boolean; findRequest: number}) {
  const cache = useQueryClient();
  const {uiDensity} = usePreferences();
  const input = useRef<HTMLInputElement | null>(null);
  const handledFind = useRef(findRequest);
  const [host, setHost] = useState('');
  const [search, setSearch] = useState('');
  const [term, setTerm] = useState('');
  const [source, setSource] = useState('');
  const [page, setPage] = useState(0);
  const [feedback, setFeedback] = useState('');
  const [error, setError] = useState('');
  const [selectedEntry, setSelectedEntry] = useState<Entry | null>(null);
  const history = useQuery({queryKey: ['command-history', host, term, source, page], enabled: isDesktop && active, retry: false,
    queryFn: () => desktopCommand<Snapshot>('command_history', {hostKey: host || null, query: term || null, source: source || null, offset: page * 100})});
  useEffect(() => { const timer = setTimeout(() => { setTerm(search); setPage(0); }, 200); return () => clearTimeout(timer); }, [search]);
  useEffect(() => {
    if (handledFind.current === findRequest) return;
    handledFind.current = findRequest;
    if (active) { input.current?.focus(); input.current?.select(); }
  }, [active, findRequest]);
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const invalidate = () => { if (!timer) timer = setTimeout(() => { timer = null; void cache.invalidateQueries({queryKey: ['command-history']}); }, 150); };
    void listen('orbitshell:command-history-changed', invalidate).then(unlisten => { if (disposed) unlisten(); else { stop = unlisten; invalidate(); } }).catch(failure => setError(errorMessage(failure)));
    return () => { disposed = true; stop?.(); if (timer) clearTimeout(timer); };
  }, [cache]);
  const current = history.data?.hosts.find(item => item.key === host);
  const copyCommand = (entry: Entry) => {
    if (entry.redacted) return;
    setError('');
    void navigator.clipboard.writeText(entry.command).then(() => setFeedback('命令已复制')).catch(failure => setError(errorMessage(failure)));
  };
  return <><VStack className="command-history-pane" hidden={!active} gap={0}>
    <HStack className="history-toolbar" gap={3} vAlign="center" wrap="wrap">
      <HStack className="history-title" gap={2} vAlign="center"><History size="1.1em" /><Text className="history-heading">执行历史</Text>
        <Text className="history-context" color="secondary" maxLines={1} hasTruncateTooltip>{current ? current.name : '全部主机'}</Text></HStack>
      <HStack className="history-filters" gap={1} vAlign="center" wrap="wrap">
        <StackItem className="history-host-picker"><Selector label="筛选主机" isLabelHidden value={host} onChange={value => { setHost(value); setPage(0); }}
          options={[{value: '', label: '全部主机'}, ...(history.data?.hosts ?? []).map(item => ({value: item.key, label: `${item.name} · ${item.temporary ? '临时会话' : address(item.hostname, item.port)}`}))]} width="100%" /></StackItem>
        <StackItem className="history-search-field"><TextInput ref={input} label="搜索执行历史" isLabelHidden value={search} onChange={setSearch} placeholder="搜索命令、主机、Agent" size="sm" hasClear startIcon={<Search size="1em" />} controlClassName="history-search" /></StackItem>
        <StackItem className="history-source-filter"><Selector label="命令来源" isLabelHidden value={source} onChange={value => { setSource(value); setPage(0); }} options={[{value: '', label: '全部来源'}, {value: 'human', label: '人工输入'}, {value: 'ai', label: 'AI 输入'}]} width="100%" /></StackItem>
        <IconButton label="刷新执行历史" tooltip="刷新执行历史" icon={<RefreshCw size="1em" />} variant="ghost" size="sm" isDisabled={!isDesktop || history.isFetching} onClick={() => { void history.refetch(); }} />
        <IconButton label="记录说明" tooltip={recordHelp} icon={<Info size="1em" />} variant="ghost" size="sm" />
      </HStack>
    </HStack>
    {(history.error || error) && <Banner className="compact-feedback" status="warning" title="执行历史暂不可用" description={history.error ? errorMessage(history.error) : error} endContent={<Button label="重试" size="sm" onClick={() => { setError(''); void history.refetch(); }} />} />}
    {history.data?.warning && <Banner className="compact-feedback" status="warning" title="执行历史未能保存" description={history.data.warning} endContent={<Button label="重试保存" size="sm" onClick={() => {
      void desktopCommand('command_history_retry').then(() => history.refetch()).catch(failure => setError(errorMessage(failure)));
    }} />} />}
    <HStack className="history-content" gap={0} vAlign="stretch">
      <VStack className="history-hosts" gap={0} as="nav" aria-label="执行历史主机列表">
        <HStack className="history-hosts-heading" hAlign="between" vAlign="center"><Text>主机</Text><Text hasTabularNumbers>{history.data?.hosts.length ?? 0}</Text></HStack>
        <List density="compact" edge="flush">
          <ListItem className="history-host" label="全部主机" startContent={<Layers size="1em" />} isSelected={!host} data-active={String(!host)} onClick={() => { setHost(''); setPage(0); }} endContent={<Text className="count-label">{history.data?.count ?? 0}</Text>} />
          {(history.data?.hosts ?? []).map(item => <ListItem key={item.key} className="history-host" isSelected={host === item.key} data-active={String(host === item.key)} startContent={<Server size="1em" />} onClick={() => { setHost(item.key); setPage(0); }}
            label={<VStack className="history-host-label" gap={0}><Text maxLines={1} hasTruncateTooltip>{item.name}</Text><Text className="history-cell-meta" color="secondary" maxLines={1}>{item.temporary ? '临时会话' : address(item.hostname, item.port)}</Text></VStack>}
            endContent={<Text className="count-label">{item.count}</Text>} />)}
        </List>
      </VStack>
      <StackItem className="history-table-region" size="fill" isScrollable>
        {history.data?.entries.length ? <Table<Entry> className="history-table" data={history.data.entries} idKey="id" density="compact" dividers="rows" textOverflow="truncate" hasHover columns={[
          {key: 'createdAt', header: '时间', width: pixel(96), renderCell: row => <VStack className="history-time" gap={0}><Text hasTabularNumbers>{timeFormat.format(row.createdAt)}</Text><Text className="history-cell-meta" color="secondary" hasTabularNumbers>{dateFormat.format(row.createdAt)}</Text></VStack>},
          {key: 'command', header: '执行命令', width: proportional(3.6, {minWidth: 180}), renderCell: row => <Button className="history-command-button" label="查看完整命令"
            tooltip="查看完整命令" variant="ghost" size="sm" onClick={() => { setSelectedEntry(row); setFeedback(''); setError(''); }}>
            <Text className="mono-text history-command" maxLines={1} hasTruncateTooltip={false} color={row.redacted ? 'secondary' : 'primary'}>{row.command}</Text>
          </Button>},
          {key: 'sessionName', header: '主机 / 用户', width: proportional(1.2, {minWidth: 120}), renderCell: row => <VStack className="history-cell-stack" gap={0}><Text maxLines={1} hasTruncateTooltip>{row.sessionName}</Text><Text className="history-cell-meta" color="secondary" maxLines={1} hasTruncateTooltip>{row.username}@{address(row.hostname, row.port)}</Text></VStack>},
          {key: 'source', header: '来源', width: proportional(1, {minWidth: 110}), renderCell: row => <HStack className="history-source" gap={1} vAlign="center">{row.source === 'ai' ? <Bot size="1em" /> : <UserRound size="1em" />}<Text maxLines={1} hasTruncateTooltip>{row.source === 'ai' ? row.agentName || 'AI' : '人工'}</Text></HStack>},
          {key: 'actions', header: '', width: pixel(42), renderCell: row => <IconButton label="复制命令" tooltip="复制命令" className="history-copy" icon={<Copy size="1em" />} variant="ghost" size="sm" isDisabled={row.redacted} onClick={() => copyCommand(row)} />},
        ]} /> : <VStack className="history-empty" gap={2} hAlign="center" vAlign="center"><History size="1.8em" /><Text color="secondary">{!isDesktop ? '请在桌面端查看本机执行历史。' : history.isPending ? '正在读取执行历史…' : host || term || source ? '没有匹配的执行记录' : '暂无执行历史，提交命令后会显示在这里。'}</Text></VStack>}
      </StackItem>
    </HStack>
    <HStack className="history-footer" hAlign="between" vAlign="center" gap={2}>
      <HStack className="history-footer-summary" gap={2} vAlign="center"><Text color="secondary">{history.data?.total ?? 0} 条记录 · 本机保存</Text>
        {feedback && <HStack className="history-copy-status" gap={1} vAlign="center" role="status"><Check size="1em" /><Text>{feedback}</Text></HStack>}</HStack>
      <HStack className="history-pagination" gap={1} vAlign="center"><Button label="上一页" variant="ghost" size="sm" isDisabled={!page || history.isFetching} onClick={() => setPage(value => Math.max(0, value - 1))} /><Text color="secondary" hasTabularNumbers>{page + 1} / {Math.max(1, Math.ceil((history.data?.total ?? 0) / 100))}</Text><Button label="下一页" variant="ghost" size="sm" isDisabled={(page + 1) * 100 >= (history.data?.total ?? 0) || history.isFetching} onClick={() => setPage(value => value + 1)} /></HStack>
    </HStack>
  </VStack>
  {active && selectedEntry && <Dialog isOpen className="orbit-dialog history-detail-dialog" data-density={uiDensity} width="calc(var(--orbit-unit) * 54)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" padding={0} onOpenChange={open => { if (!open) setSelectedEntry(null); }}>
    <VStack className="orbit-dialog-layout" gap={0}>
      <DialogHeader className="orbit-dialog-header" title="命令详情" hasDivider onOpenChange={open => { if (!open) setSelectedEntry(null); }} />
      <VStack className="orbit-dialog-body" gap={3}>
        <HStack className="history-detail-meta" gap={3} wrap="wrap"><Text>{selectedEntry.sessionName} · {selectedEntry.username}@{address(selectedEntry.hostname, selectedEntry.port)}</Text><Text>{dateFormat.format(selectedEntry.createdAt)} {timeFormat.format(selectedEntry.createdAt)}</Text><Text>{selectedEntry.source === 'ai' ? `AI · ${selectedEntry.agentName || 'Agent'}` : '人工输入'}</Text></HStack>
        <VStack className="history-command-preview" gap={0}><Text className="history-command-detail">{selectedEntry.command}</Text></VStack>
        {error && <Banner className="compact-feedback" status="warning" title="复制未完成" description={error} />}
      </VStack>
      <HStack className="orbit-dialog-footer" gap={2} hAlign="between"><Text className="history-detail-feedback" color="accent" role="status">{feedback}</Text><HStack gap={2}><Button label="关闭" onClick={() => setSelectedEntry(null)} /><Button label="复制命令" icon={<Copy size="1em" />} isDisabled={selectedEntry.redacted} onClick={() => copyCommand(selectedEntry)} /></HStack></HStack>
    </VStack>
  </Dialog>}</>;
}
