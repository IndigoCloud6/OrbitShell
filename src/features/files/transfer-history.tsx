import {useState} from 'react';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Table, proportional, pixel} from '@astryxdesign/core/Table';
import {Banner} from '@astryxdesign/core/Banner';
import {ArrowLeft, Copy, Search} from 'lucide-react';
import {usePreferences} from '../../app/preferences';
import {useCopyText} from '../../shared/context-menu';
import {formatBytes} from './file-utils';
import type {Transfer} from './transfer-provider';

const labels: Record<Transfer['status'], string> = {choosing: '等待选择', running: '传输中', done: '已完成', error: '失败 / 中断', cancelled: '已停止'};
const date = new Intl.DateTimeFormat('zh-CN', {dateStyle: 'short', timeStyle: 'medium'});
function time(value?: number | null) { return value ? date.format(value) : '—'; }

export function TransferHistoryDialog({open, records, warning, onClose}: {
  open: boolean; records: Transfer[]; warning: string; onClose: () => void;
}) {
  const [search, setSearch] = useState('');
  const [page, setPage] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const {uiDensity} = usePreferences();
  const copy = useCopyText();
  if (!open) return null;
  const selected = records.find(row => row.id === selectedId);
  const query = search.trim().toLocaleLowerCase();
  const filtered = records.filter(row => !query || [row.name, row.sessionName, row.hostname, row.agentName, row.localPath, row.remotePath,
    row.direction === 'upload' ? '上传' : '下载', row.agentId ? 'AI' : '人工', labels[row.status]].some(value => value?.toLocaleLowerCase().includes(query))).reverse();
  const pages = Math.max(1, Math.ceil(filtered.length / 20));
  const current = Math.min(page, pages - 1);
  const close = () => { setSelectedId(null); onClose(); };
  return <Dialog isOpen className="orbit-dialog transfer-history-dialog" data-density={uiDensity}
    width="calc(var(--orbit-unit) * 76)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" padding={0}
    onOpenChange={value => { if (!value) close(); }}>
    <VStack className="orbit-dialog-layout" gap={0}>
      <DialogHeader className="orbit-dialog-header" title={selected ? '传输详情' : '传输记录'} hasDivider onOpenChange={value => { if (!value) close(); }} />
      <VStack className="orbit-dialog-body" gap={3}>
        {warning && <Banner className="compact-feedback" status="warning" title="传输记录提示" description={warning} />}
        {selected ? <>
          <HStack gap={3} wrap="wrap" vAlign="center">
            <Button label="返回记录" icon={<ArrowLeft size="1em" />} variant="ghost" size="sm" onClick={() => setSelectedId(null)} />
            <Text>{selected.direction === 'upload' ? '上传' : '下载'} · {labels[selected.status]}</Text>
            <Text color="secondary">{selected.agentId ? `AI · ${selected.agentName || 'Agent'}` : '人工操作'}</Text>
          </HStack>
          <VStack gap={1}><Text color="secondary">所属主机</Text><Text className="transfer-record-path">{selected.sessionName ?? '—'}{selected.hostname ? ` · ${selected.hostname}:${selected.port ?? 22}` : ''}</Text></VStack>
          <VStack gap={1}>
            <HStack gap={2} vAlign="center"><Text color="secondary">本机文件</Text><IconButton label="复制本机路径" tooltip="复制本机路径" size="sm" variant="ghost" icon={<Copy size="1em" />} isDisabled={!selected.localPath} onClick={() => { if (selected.localPath) void copy(selected.localPath); }} /></HStack>
            <Text className="transfer-record-path mono-text">{selected.localPath || '—'}</Text>
          </VStack>
          <VStack gap={1}>
            <HStack gap={2} vAlign="center"><Text color="secondary">远程文件</Text><IconButton label="复制远程路径" tooltip="复制远程路径" size="sm" variant="ghost" icon={<Copy size="1em" />} isDisabled={!selected.remotePath} onClick={() => { if (selected.remotePath) void copy(selected.remotePath); }} /></HStack>
            <Text className="transfer-record-path mono-text">{selected.remotePath || '—'}</Text>
          </VStack>
          <HStack gap={4} wrap="wrap">
            <VStack gap={1}><Text color="secondary">开始时间</Text><Text hasTabularNumbers>{time(selected.createdAt)}</Text></VStack>
            <VStack gap={1}><Text color="secondary">结束时间</Text><Text hasTabularNumbers>{time(selected.finishedAt)}</Text></VStack>
            <VStack gap={1}><Text color="secondary">耗时</Text><Text hasTabularNumbers>{selected.finishedAt != null && selected.elapsedMs != null ? `${(selected.elapsedMs / 1000).toFixed(1)} 秒` : '—'}</Text></VStack>
            <VStack gap={1}><Text color="secondary">已传输 / 总大小</Text><Text hasTabularNumbers>{formatBytes(selected.bytes)} / {selected.total !== null ? formatBytes(selected.total) : '未知'}</Text></VStack>
          </HStack>
          <VStack gap={1}><Text color="secondary">结果说明</Text><Text className="transfer-record-path">{selected.message}</Text></VStack>
          {selected.temporary && <Text color="secondary">临时会话记录仅保留在本次运行中。</Text>}
        </> : <>
          <TextInput label="搜索传输记录" isLabelHidden placeholder="搜索文件、主机、路径或 Agent…" startIcon={<Search size="1em" />}
            value={search} onChange={value => { setSearch(value); setPage(0); }} size="sm" />
          <StackItem className="transfer-history-table-region" isScrollable>
            {filtered.length ? <Table<Transfer> className="transfer-history-table" aria-label="上传下载记录" data={filtered.slice(current * 20, (current + 1) * 20)} idKey="id"
              density="compact" dividers="rows" textOverflow="truncate" columns={[
                {key: 'createdAt', header: '开始时间', width: pixel(150), renderCell: row => <Text hasTabularNumbers>{time(row.createdAt)}</Text>},
                {key: 'name', header: '文件', width: proportional(2, {minWidth: 180}), renderCell: row => <Button label={`查看 ${row.name} 的传输详情`} tooltip={`查看 ${row.name} 的传输详情`} className="transfer-record-button" variant="ghost" size="sm" onClick={() => setSelectedId(row.id)}><Text maxLines={1}>{row.name}</Text></Button>},
                {key: 'sessionName', header: '主机', width: proportional(1, {minWidth: 110}), renderCell: row => <Text maxLines={1} hasTruncateTooltip>{row.sessionName || '—'}</Text>},
                {key: 'agentName', header: '来源', width: proportional(1, {minWidth: 110}), renderCell: row => <Text maxLines={1} hasTruncateTooltip>{row.agentId ? `AI · ${row.agentName || 'Agent'}` : '人工'}</Text>},
                {key: 'direction', header: '方向', width: pixel(68), renderCell: row => <Text>{row.direction === 'upload' ? '上传' : '下载'}</Text>},
                {key: 'bytes', header: '已传输', width: pixel(100), renderCell: row => <Text hasTabularNumbers>{formatBytes(row.bytes)}</Text>},
                {key: 'status', header: '状态', width: pixel(110), renderCell: row => <Text maxLines={1} hasTruncateTooltip>{labels[row.status]}</Text>},
              ]} /> : <Text color="secondary">{records.length ? '没有匹配的传输记录' : '暂无传输记录'}</Text>}
          </StackItem>
          <HStack hAlign="between" vAlign="center" gap={2} wrap="wrap">
            <Text color="secondary">{filtered.length} 条记录 · 普通连接本机保存，最多 1,000 条</Text>
            <HStack gap={2} vAlign="center"><Button label="上一页" variant="ghost" size="sm" isDisabled={current === 0} onClick={() => setPage(current - 1)} /><Text hasTabularNumbers>{current + 1} / {pages}</Text><Button label="下一页" variant="ghost" size="sm" isDisabled={current + 1 >= pages} onClick={() => setPage(current + 1)} /></HStack>
          </HStack>
        </>}
      </VStack>
      <HStack className="orbit-dialog-footer" gap={2} hAlign="end"><Button label="关闭" onClick={close} /></HStack>
    </VStack>
  </Dialog>;
}
