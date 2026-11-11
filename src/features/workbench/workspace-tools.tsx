import type {ReactNode} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Table, proportional, pixel} from '@astryxdesign/core/Table';
import {ProgressBar} from '@astryxdesign/core/ProgressBar';
import {ChevronDown, ChevronUp, ArrowUp, ArrowDown, File, CheckCircle2, CircleAlert} from 'lucide-react';
import {formatBytes} from '../files/file-utils';
import type {Transfer} from '../files/transfer-provider';
import type {ToolTab} from './ide-chrome';

export interface ConnectionLog extends Record<string, unknown> {id: string; time: string; name: string; message: string}
export function WorkspaceTools({transfers, logs, tab, open, onTab, onToggle, onClear, ai}: {
  transfers: Transfer[]; logs: ConnectionLog[]; tab: ToolTab; open: boolean;
  onTab: (tab: ToolTab) => void; onToggle: () => void; onClear: () => void; ai: ReactNode;
}) {
  return <VStack className={open ? 'workspace-tools is-open' : 'workspace-tools'} gap={0}>
    <HStack className="tools-heading" vAlign="center" gap={0}>
      <HStack className="tools-tabs" role="tablist" aria-label="工作区工具" gap={0}>
        <Button className="tool-tab" data-active={String(tab === 'transfers')} label="传输队列" role="tab" aria-selected={tab === 'transfers'} aria-controls="tool-transfers" variant="ghost"
          endContent={transfers.length ? <Text className="count-label">{transfers.length}</Text> : undefined} onClick={() => onTab('transfers')} />
        <Button className="tool-tab" data-active={String(tab === 'logs')} label="连接日志" role="tab" aria-selected={tab === 'logs'} aria-controls="tool-logs" variant="ghost" onClick={() => onTab('logs')} />
        <Button className="tool-tab" data-active={String(tab === 'ai')} label="AI 工具" role="tab" aria-selected={tab === 'ai'} aria-controls="tool-ai" variant="ghost" onClick={() => onTab('ai')} />
      </HStack>
      <StackItem size="fill" />
      {tab === 'transfers' && transfers.some(row => row.status === 'done' || row.status === 'error') && <Button label="清理已完成" className="clear-transfers" size="sm" variant="ghost" onClick={onClear} />}
      <IconButton label={open ? '折叠工具面板' : '展开工具面板'} tooltip={open ? '折叠工具面板' : '展开工具面板'} className="tools-collapse" variant="ghost" size="sm" icon={open ? <ChevronDown /> : <ChevronUp />} onClick={onToggle} />
    </HStack>
    {open && <StackItem className="tools-content" size="fill" isScrollable>
      {tab === 'transfers' && <VStack id="tool-transfers" role="tabpanel" className="transfer-table-wrap" gap={0}>
        {transfers.length ? <Table<Transfer> className="transfer-table" data={transfers} idKey="id" density="compact" dividers="rows" textOverflow="truncate"
          columns={[
            {key: 'name', header: '文件', width: proportional(1.8, {minWidth: 160}), renderCell: row => <HStack gap={2} vAlign="center"><File className="transfer-file-icon" /><Text maxLines={1}>{row.name}</Text></HStack>},
            {key: 'direction', header: '方向', width: pixel(104), renderCell: row => <HStack gap={2} vAlign="center">{row.direction === 'upload' ? <ArrowUp size="1em" /> : <ArrowDown size="1em" />}<Text>{row.direction === 'upload' ? '上传' : '下载'}</Text></HStack>},
            {key: 'progress', header: '进度', width: proportional(1.35, {minWidth: 150}), renderCell: row => row.status === 'done' ?
              <HStack gap={2} vAlign="center" className="success-icon"><CheckCircle2 size="1.1em" /><Text>已完成</Text></HStack> :
              row.status === 'error' ? <HStack gap={2} vAlign="center" className="error-icon"><CircleAlert size="1em" /><Text>失败</Text></HStack> :
              <HStack className="transfer-progress" gap={2} vAlign="center"><StackItem size="fill"><ProgressBar label={`${row.name}传输进度`} isLabelHidden value={row.total ? row.bytes / row.total * 100 : 0} isIndeterminate={!row.total} /></StackItem>
                <Text hasTabularNumbers>{row.total ? `${Math.floor(row.bytes / row.total * 100)}%` : '—'}</Text></HStack>},
            {key: 'bytes', header: '大小', width: proportional(1.5, {minWidth: 160}), renderCell: row => <Text hasTabularNumbers>{row.status === 'done' ? formatBytes(row.bytes) : row.total ? `${formatBytes(row.bytes)} / ${formatBytes(row.total)}` : formatBytes(row.bytes)}</Text>},
            {key: 'speed', header: '速度', width: pixel(112), renderCell: row => <Text hasTabularNumbers>{row.status === 'running' && row.speed ? `${formatBytes(row.speed)}/s` : '—'}</Text>},
            {key: 'status', header: '状态', width: pixel(116), renderCell: row => <Text className={row.status === 'done' ? 'success-icon' : row.status === 'error' ? 'error-icon' : ''} maxLines={1} hasTruncateTooltip>{row.status === 'done' ? '✓ 已完成' : row.message}</Text>},
          ]} /> : <Text className="tools-empty" color="secondary">暂无传输任务 · 在右侧文件区上传或下载文件</Text>}
      </VStack>}
      {tab === 'logs' && <VStack id="tool-logs" role="tabpanel" gap={0}>
        {logs.length ? <Table<ConnectionLog> className="connection-log-table" data={logs} idKey="id" density="compact" dividers="rows"
          columns={[{key: 'time', header: '时间', width: pixel(104)}, {key: 'name', header: '连接', width: pixel(160)}, {key: 'message', header: '事件', width: proportional(1)}]} /> :
          <Text className="tools-empty" color="secondary">连接状态变化将在这里显示，日志仅保留在本次运行中。</Text>}
      </VStack>}
      {tab === 'ai' && <VStack id="tool-ai" role="tabpanel" className="ai-tools" gap={3}>{ai}</VStack>}
    </StackItem>}
  </VStack>;
}
