import {useState} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {TextInput} from '@astryxdesign/core/TextInput';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import {Banner} from '@astryxdesign/core/Banner';
import {List, ListItem} from '@astryxdesign/core/List';
import {ContextMenu} from '@astryxdesign/core/ContextMenu';
import type {ContextMenuOption} from '@astryxdesign/core/ContextMenu';
import {ArrowLeft, RefreshCw, Eye, EyeOff, Upload, Download, MoreHorizontal, Folder, FolderOpen, Copy, PanelRightClose, FilePenLine} from 'lucide-react';
import {formatBytes} from './file-utils';
import type {RemoteEntry} from './file-utils';
import {RemoteFileIcon} from './remote-file-icon';
import {openContextMenuFromKeyboard, useCopyText} from '../../shared/context-menu';
import {canEditFile} from '../editor/file-types';

interface Props {
  connected: boolean; entries: RemoteEntry[]; path: string; selectedPath: string | null;
  compact: boolean; showHidden: boolean; preparing?: boolean; loading?: boolean; fetching?: boolean; error?: string;
  emptyTitle?: string; emptyDescription?: string;
  contextKey?: string; directoryPath?: string;
  onPath: (value: string) => void; onNavigate: (path: string) => void; onRefresh: () => void;
  onHidden: () => void; onDensity: () => void; onUpload: (directory?: string) => void; onDownload: (entry?: RemoteEntry) => void;
  onSelect: (entry: RemoteEntry) => void; onCollapse: () => void;
  onEdit?: (entry: RemoteEntry) => void;
}
export function FileBrowserView(props: Props) {
  const entries = props.showHidden ? props.entries : props.entries.filter(entry => !entry.name.startsWith('.'));
  const isDirectoryEmpty = props.connected && !props.loading && !props.error && entries.length === 0;
  const selected = entries.find(entry => entry.path === props.selectedPath);
  const copy = useCopyText();
  const [context, setContext] = useState<{props: Props; entry: RemoteEntry | null} | null>(null);
  const snapshot = context?.props ?? props;
  const entry = context?.entry ?? null;
  const directory = snapshot.directoryPath ?? snapshot.path;
  const unavailable = !props.connected || Boolean(props.preparing) || !snapshot.connected || Boolean(snapshot.preparing);
  const menu: ContextMenuOption[] = entry ? [
    ...(entry.isDirectory ? [
      {label: '打开目录', icon: <FolderOpen />, isDisabled: unavailable, onClick: () => snapshot.onNavigate(entry.path)},
      {label: '上传到此目录', icon: <Upload />, isDisabled: unavailable, onClick: () => snapshot.onUpload(entry.path)},
    ] : [
      ...(snapshot.onEdit && canEditFile(entry) ? [{label: '编辑文件', icon: <FilePenLine />, isDisabled: unavailable, onClick: () => snapshot.onEdit?.(entry)}] : []),
      {label: '下载文件', icon: <Download />, isDisabled: unavailable, onClick: () => snapshot.onDownload(entry)},
    ]),
    {type: 'divider'},
    {label: '复制名称', icon: <Copy />, onClick: () => { void copy(entry.name); }},
    {label: entry.isDirectory ? '复制目录路径' : '复制完整路径', icon: <Copy />, onClick: () => { void copy(entry.path); }},
  ] : [
    {label: '上传文件', icon: <Upload />, isDisabled: unavailable, onClick: () => snapshot.onUpload(directory)},
    {label: '刷新目录', icon: <RefreshCw />, isDisabled: unavailable, onClick: snapshot.onRefresh},
    {label: '返回上级目录', icon: <ArrowLeft />, isDisabled: unavailable, onClick: () => snapshot.onNavigate(`${directory}/..`)},
    {type: 'divider'},
    {label: snapshot.showHidden ? '隐藏点号文件' : '显示隐藏文件', icon: snapshot.showHidden ? <EyeOff /> : <Eye />, onClick: snapshot.onHidden},
    {label: '复制目录路径', icon: <Copy />, isDisabled: !snapshot.connected, onClick: () => { void copy(directory); }},
  ];
  return <VStack className="file-panel" height="100%" gap={0}>
    <HStack className="panel-heading file-heading" hAlign="between" vAlign="center"><Text className="panel-title">远程文件</Text>
      <IconButton label="收起远程文件" tooltip="收起远程文件" className="panel-collapse" variant="ghost" size="sm" icon={<PanelRightClose />} onClick={props.onCollapse} />
    </HStack>
    {props.connected && <VStack className="file-controls" gap={0}>
      <TextInput controlClassName="ide-field-control" label="远程目录" isLabelHidden size="md" width="100%" startIcon={<Folder size="1.1em" />}
        value={props.path} isDisabled={Boolean(props.preparing)} onChange={props.onPath} onEnter={() => props.onNavigate(props.path)} />
      <HStack className="file-toolbar" hAlign="between" gap={1} vAlign="center" role="group" aria-label="文件操作">
        <HStack className="file-toolbar-group" gap={1}>
          <IconButton className="file-action" size="sm" label="上级目录" tooltip="上级目录" icon={<ArrowLeft />} variant="ghost" isDisabled={Boolean(props.preparing)} onClick={() => props.onNavigate(`${props.directoryPath ?? props.path}/..`)} />
          <IconButton className="file-action" size="sm" label="刷新目录" tooltip="刷新目录" icon={<RefreshCw />} variant="ghost" isDisabled={Boolean(props.preparing)} isLoading={Boolean(props.fetching)} onClick={props.onRefresh} />
          <IconButton className="file-action" size="sm" label={props.showHidden ? '隐藏点号文件' : '显示隐藏文件'} tooltip={props.showHidden ? '隐藏点号文件' : '显示隐藏文件'} aria-pressed={props.showHidden} data-active={String(props.showHidden)}
            icon={props.showHidden ? <Eye /> : <EyeOff />} variant="ghost" onClick={props.onHidden} />
        </HStack>
        <HStack className="file-toolbar-group" gap={1}>
        <IconButton className="file-action" size="sm" label="上传文件" tooltip="上传文件" icon={<Upload />} variant="ghost" isDisabled={Boolean(props.preparing)} onClick={() => props.onUpload()} />
        <IconButton className="file-action" size="sm" label="下载文件" tooltip="下载选中文件" icon={<Download />} variant="ghost" isDisabled={Boolean(props.preparing) || !selected || selected.isDirectory} onClick={() => props.onDownload()} />
        <DropdownMenu hasChevron={false} presentation="popover" alignment="end" button={{label: '更多文件操作', tooltip: '更多文件操作', isIconOnly: true, icon: <MoreHorizontal />, variant: 'ghost', size: 'sm', className: 'file-action'}}
          items={[{label: '打开选中目录', isDisabled: Boolean(props.preparing) || !selected?.isDirectory, onClick: () => { if (selected?.isDirectory) props.onNavigate(selected.path); }},
            {label: props.compact ? '切换为舒适列表' : '切换为紧凑列表', onClick: props.onDensity},
            {label: '复制目录路径', onClick: () => { void copy(props.directoryPath ?? props.path); }}]} />
        </HStack>
      </HStack>
    </VStack>}
    <StackItem size="fill" isScrollable className="file-browser" aria-label="远程文件列表">
    <ContextMenu key={props.contextKey ?? 'files'} data-orbit-context="files" data-directory-empty={String(isDirectoryEmpty)} className="orbit-context-menu" label={entry?.name ?? '远程文件'} size="sm" menuWidth="calc(var(--orbit-unit) * 18)" items={menu}
      onKeyDownCapture={openContextMenuFromKeyboard}
      onContextMenuCapture={event => {
        const row = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-remote-path]') : null;
        const target = entries.find(candidate => candidate.path === row?.dataset.remotePath) ?? null;
        setContext({props, entry: target});
        if (target) props.onSelect(target);
      }} onContextMenu={event => event.stopPropagation()} onOpenChange={open => { if (!open) setContext(null); }}>
      {!props.connected ? <VStack className="file-empty" height="100%" padding={5} gap={3} hAlign="center" vAlign="center">
        <Folder className="file-empty-mark muted-icon" /><Text className="empty-title" justify="center">{props.emptyTitle ?? '尚未连接服务器'}</Text>
        <Text color="secondary" type="supporting" justify="center">{props.emptyDescription ?? '从左侧选择连接，浏览和传输远程文件。'}</Text>
      </VStack> : props.loading ? <Text className="panel-message">{props.preparing ? '正在准备远程文件，等待服务器就绪…' : '正在读取目录…'}</Text> : props.error ?
        <VStack padding={3} gap={2}><Banner className="compact-feedback" status="error" title="无法读取目录" description={props.error}
          endContent={<Button label="重试" size="sm" onClick={props.onRefresh} />} /></VStack> : <>
          <HStack className="file-columns" hAlign="between" vAlign="center"><Text>名称</Text><Text>大小</Text></HStack>
          {entries.length > 0 ? <List className={props.compact ? 'file-list compact' : 'file-list'} density="compact" edge="flush">
            {entries.map(entry => <ListItem key={entry.path} className="file-row" data-remote-path={entry.path} data-active={String(selected?.path === entry.path)} isSelected={selected?.path === entry.path}
              label={<Text className="file-name" maxLines={1}>{entry.name}</Text>}
              startContent={<RemoteFileIcon entry={entry} />}
              endContent={<Text className="file-size" color="secondary" hasTabularNumbers>{entry.isDirectory ? '—' : formatBytes(entry.size)}</Text>}
              onClick={() => props.onSelect(entry)} onDoubleClick={() => { if (entry.isDirectory) props.onNavigate(entry.path); }}
              onKeyDown={event => {
                if (!event.nativeEvent.isComposing && (event.key === 'Enter' || event.key === 'ArrowRight') && entry.isDirectory) { event.preventDefault(); props.onNavigate(entry.path); }
              }} />)}
          </List> : <VStack className="file-directory-empty" padding={3} hAlign="center" vAlign="center">
            <Text className="file-empty-message" color="secondary" justify="center">{props.entries.length ? '仅有隐藏文件，可点击眼睛图标显示。' : '这个目录是空的。'}</Text>
          </VStack>}
        </>}
    </ContextMenu>
    </StackItem>
    <HStack className="file-summary" vAlign="center"><Text>{props.connected ? `${entries.length} 项${selected ? ' · 1 项已选' : ''}${props.entries.length > entries.length ? ` · 隐藏 ${props.entries.length - entries.length} 项` : ''}` : 'SFTP 文件浏览器'}</Text></HStack>
  </VStack>;
}
