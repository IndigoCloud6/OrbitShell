import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {TextInput} from '@astryxdesign/core/TextInput';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import {Banner} from '@astryxdesign/core/Banner';
import {List, ListItem} from '@astryxdesign/core/List';
import {ArrowUp, RefreshCw, Eye, EyeOff, Upload, Download, MoreHorizontal, Folder, File, PanelRightClose} from 'lucide-react';
import {formatBytes} from './file-utils';
import type {RemoteEntry} from './file-utils';

interface Props {
  connected: boolean; entries: RemoteEntry[]; path: string; selectedPath: string | null;
  compact: boolean; showHidden: boolean; loading?: boolean; fetching?: boolean; error?: string;
  emptyTitle?: string; emptyDescription?: string;
  onPath: (value: string) => void; onNavigate: (path: string) => void; onRefresh: () => void;
  onHidden: () => void; onDensity: () => void; onUpload: () => void; onDownload: () => void;
  onSelect: (entry: RemoteEntry) => void; onCollapse: () => void;
}
export function FileBrowserView(props: Props) {
  const entries = props.showHidden ? props.entries : props.entries.filter(entry => !entry.name.startsWith('.'));
  const selected = entries.find(entry => entry.path === props.selectedPath);
  return <VStack className="file-panel" height="100%" gap={0}>
    <HStack className="panel-heading file-heading" hAlign="between" vAlign="center"><Text className="panel-title">远程文件</Text>
      <IconButton label="收起远程文件" tooltip="收起远程文件" className="panel-collapse" variant="ghost" size="sm" icon={<PanelRightClose />} onClick={props.onCollapse} />
    </HStack>
    {props.connected && <VStack className="file-controls" gap={0}>
      <TextInput controlClassName="ide-field-control" label="远程目录" isLabelHidden size="md" width="100%" startIcon={<Folder size="1.1em" />}
        value={props.path} onChange={props.onPath} onEnter={() => props.onNavigate(props.path)} />
      <HStack className="file-toolbar" hAlign="between" gap={1} vAlign="center" role="group" aria-label="文件操作">
        <IconButton className="file-action" size="lg" label="上级目录" tooltip="上级目录" icon={<ArrowUp />} variant="secondary" onClick={() => props.onNavigate(`${props.path}/..`)} />
        <IconButton className="file-action" size="lg" label="刷新目录" tooltip="刷新目录" icon={<RefreshCw />} variant="secondary" isLoading={Boolean(props.fetching)} onClick={props.onRefresh} />
        <IconButton className="file-action" size="lg" label="显示隐藏文件" tooltip={props.showHidden ? '隐藏点号文件' : '显示隐藏文件'} aria-pressed={props.showHidden} data-active={String(props.showHidden)}
          icon={props.showHidden ? <Eye /> : <EyeOff />} variant="secondary" onClick={props.onHidden} />
        <IconButton className="file-action" size="lg" label="上传文件" tooltip="上传文件" icon={<Upload />} variant="secondary" onClick={props.onUpload} />
        <IconButton className="file-action" size="lg" label="下载文件" tooltip="下载选中文件" icon={<Download />} variant="secondary" isDisabled={!selected || selected.isDirectory} onClick={props.onDownload} />
        <DropdownMenu hasChevron={false} presentation="popover" alignment="end" button={{label: '更多文件操作', isIconOnly: true, icon: <MoreHorizontal />, variant: 'secondary', size: 'lg', className: 'file-action'}}
          items={[{label: '打开选中目录', isDisabled: !selected?.isDirectory, onClick: () => { if (selected?.isDirectory) props.onNavigate(selected.path); }},
            {label: props.compact ? '切换为舒适列表' : '切换为紧凑列表', onClick: props.onDensity},
            {label: '复制目录路径', onClick: () => { void navigator.clipboard.writeText(props.path).catch(() => {}); }}]} />
      </HStack>
    </VStack>}
    <StackItem size="fill" isScrollable className="file-browser" aria-label="远程文件列表">
      {!props.connected ? <VStack className="file-empty" height="100%" padding={5} gap={3} hAlign="center" vAlign="center">
        <Folder className="file-empty-mark muted-icon" /><Text className="empty-title" justify="center">{props.emptyTitle ?? '尚未连接服务器'}</Text>
        <Text color="secondary" type="supporting" justify="center">{props.emptyDescription ?? '从左侧选择连接，浏览和传输远程文件。'}</Text>
      </VStack> : props.loading ? <Text className="panel-message">正在读取目录…</Text> : props.error ?
        <VStack padding={3} gap={2}><Banner status="error" title="无法读取目录" description={props.error} /><Button label="重试" size="sm" onClick={props.onRefresh} /></VStack> : <>
          <HStack className="file-columns" hAlign="between" vAlign="center"><Text>名称</Text><Text>大小</Text></HStack>
          <List className={props.compact ? 'file-list compact' : 'file-list'} density="compact" edge="flush">
            {entries.map(entry => <ListItem key={entry.path} className="file-row" data-active={String(selected?.path === entry.path)} isSelected={selected?.path === entry.path}
              label={<Text className="file-name" maxLines={1}>{entry.name}</Text>}
              startContent={entry.isDirectory ? <Folder /> : <File />}
              endContent={<Text className="file-size" color="secondary" hasTabularNumbers>{entry.isDirectory ? '—' : formatBytes(entry.size)}</Text>}
              onClick={() => props.onSelect(entry)} onDoubleClick={() => { if (entry.isDirectory) props.onNavigate(entry.path); }}
              onKeyDown={event => {
                if (!event.nativeEvent.isComposing && (event.key === 'Enter' || event.key === 'ArrowRight') && entry.isDirectory) { event.preventDefault(); props.onNavigate(entry.path); }
              }} />)}
          </List>
          {!entries.length && <Text className="panel-message" color="secondary">{props.entries.length ? '仅有隐藏文件，可点击眼睛图标显示。' : '这个目录是空的。'}</Text>}
        </>}
    </StackItem>
    <HStack className="file-summary" vAlign="center"><Text>{props.connected ? `${entries.length} 项${selected ? ' · 1 项已选' : ''}${props.entries.length > entries.length ? ` · 隐藏 ${props.entries.length - entries.length} 项` : ''}` : 'SFTP 文件浏览器'}</Text></HStack>
  </VStack>;
}
