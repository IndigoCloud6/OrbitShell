import {useEffect, useState} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Banner} from '@astryxdesign/core/Banner';
import {Drawer, DrawerHeader} from '@astryxdesign/core/Drawer';
import {List, ListItem} from '@astryxdesign/core/List';
import {Clock3, FileText, History, Save, Search} from 'lucide-react';
import {desktopCommand, errorMessage} from '../../shared/desktop';
import {formatBytes} from '../files/file-utils';
import {CodeEditor, languageName} from './code-editor';
import {isDirty, snapshotLabel} from './types';
import type {EditorDocument, RemoteText, SnapshotInfo} from './types';
import type {EditorController} from './use-remote-editor';

export function RemoteEditorPane({file, active, connected, controller}: {file: EditorDocument; active: boolean; connected: boolean; controller: EditorController}) {
  const [historyOpen, setHistoryOpen] = useState(false);
  const [snapshots, setSnapshots] = useState<SnapshotInfo[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [preview, setPreview] = useState<RemoteText | null>(null);
  const [historyError, setHistoryError] = useState('');
  const [loading, setLoading] = useState(false);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [reload, setReload] = useState(0);
  const [cursor, setCursor] = useState({line: 1, column: 1});
  useEffect(() => { if (file.historyRequest) setHistoryOpen(true); }, [file.historyRequest]);
  useEffect(() => { if (!active) setHistoryOpen(false); }, [active]);
  useEffect(() => {
    if (!historyOpen) return;
    let cancelled = false;
    setLoading(true); setHistoryError('');
    void desktopCommand<SnapshotInfo[]>('editor_snapshots', {id: file.id}).then(items => {
      if (cancelled) return;
      setSnapshots(items); setSelectedId(previous => items.some(item => item.id === previous) ? previous : items[0]?.id ?? null);
    }).catch(failure => { if (!cancelled) setHistoryError(errorMessage(failure)); }).finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [historyOpen, file.id, file.document.revision, reload]);
  useEffect(() => {
    setPreview(null);
    if (!historyOpen || !selectedId) return;
    let cancelled = false;
    setPreviewLoading(true); setHistoryError('');
    void desktopCommand<RemoteText>('editor_snapshot_read', {id: file.id, snapshotId: selectedId}).then(text => {
      if (!cancelled) setPreview(text);
    }).catch(failure => { if (!cancelled) setHistoryError(errorMessage(failure)); }).finally(() => { if (!cancelled) setPreviewLoading(false); });
    return () => { cancelled = true; };
  }, [historyOpen, selectedId, file.id, reload]);
  const selected = snapshots.find(item => item.id === selectedId);
  return <VStack className="remote-editor" height="100%" gap={0} hidden={!active} role="region" aria-label={`${file.name}远程文件编辑器`}>
    <HStack className="editor-toolbar" gap={2} vAlign="center" wrap="wrap">
      <FileText className="editor-file-mark" />
      <StackItem size="fill" className="editor-location"><VStack gap={0}>
        <Text className="editor-path mono-text" maxLines={1} hasTruncateTooltip>{file.path}</Text>
        <Text className="editor-owner" color="secondary" maxLines={1} hasTruncateTooltip>{file.sessionName} · {file.address}</Text>
      </VStack></StackItem>
      <HStack gap={1} vAlign="center" className="editor-actions">
        <IconButton label="查找文件内容" tooltip="查找 / 替换 · Ctrl + F" size="sm" icon={<Search />} variant="ghost" onClick={() => controller.patch(file.id, {searchRequest: file.searchRequest + 1})} />
        <Button label="快照历史" size="sm" variant="ghost" icon={<History size="1em" />} onClick={() => setHistoryOpen(true)} />
        <Button label="保存到远程" tooltip={!connected ? '所属会话已断开，草稿仍可编辑和复制' : '保存到远程 · Ctrl + S'} size="sm" variant="primary" icon={<Save size="1em" />}
          isLoading={file.saving} isDisabled={!connected || !isDirty(file)} onClick={() => { void controller.save(file.id); }} />
      </HStack>
    </HStack>
    {!connected && <Banner className="compact-feedback editor-feedback" status="warning" title="所属会话已断开" description="草稿和本地快照仍可查看，当前标签不能保存到远程。" />}
    {file.error && <Banner className="compact-feedback editor-feedback" status="error" title="文件操作未完成" description={file.error} isDismissable onDismiss={() => controller.patch(file.id, {error: ''})} />}
    {file.feedback && <Banner className="compact-feedback editor-feedback" status="success" title={file.feedback} isDismissable onDismiss={() => controller.patch(file.id, {feedback: ''})} />}
    {file.conflict && <Banner className="compact-feedback editor-feedback" status="warning" title="远程文件已变更，当前草稿尚未保存" endContent={<Button label="查看差异" size="sm" onClick={() => controller.setComparison({documentId: file.id, original: file.conflict!, title: '当前远程版本', kind: 'remote'})} />} />}
    <StackItem size="fill" className="editor-code-region"><CodeEditor name={file.name} value={file.content} active={active} readOnly={file.saving} findRequest={file.searchRequest}
      onChange={content => controller.patch(file.id, {content, feedback: ''})} onCursor={(line, column) => setCursor({line, column})} /></StackItem>
    <HStack className="editor-status" hAlign="between" vAlign="center" gap={2} wrap="wrap">
      <HStack gap={1} vAlign="center"><History size="1em" /><Text>{file.saving ? '正在记录快照并保存…' : isDirty(file) ? '未保存 · 保存前自动记录远程原文件快照' : '未修改 · 保存前自动记录远程原文件快照'}</Text></HStack>
      <Text className="mono-text" hasTabularNumbers>UTF-8{file.document.bom ? ' BOM' : ''} · {file.document.lineEnding} · {languageName(file.name)} · {cursor.line}:{cursor.column}</Text>
    </HStack>
    <Drawer isOpen={historyOpen && active} onOpenChange={setHistoryOpen} label={`${file.name}快照历史`} hasScrim={false} width="min(90vw, calc(var(--orbit-unit) * 34))" padding={0} className="editor-history-drawer">
      <VStack height="100%" gap={0} className="editor-history-layout">
        <DrawerHeader title="快照历史" hasDivider startContent={<History size="1em" />} onOpenChange={setHistoryOpen} className="editor-history-heading" />
        <VStack className="editor-history-location" gap={1}><Text className="mono-text" maxLines={2}>{file.path}</Text><Text color="secondary">{file.sessionName} · {file.address}</Text></VStack>
        {historyError && <Banner className="compact-feedback editor-feedback" status="error" title="快照读取未完成" description={historyError} endContent={<Button label="重试" size="sm" onClick={() => setReload(count => count + 1)} />} />}
        <StackItem size="fill" isScrollable className="editor-history-list" aria-label="本地快照版本">
          {loading ? <Text className="panel-message" color="secondary">正在读取快照…</Text> : snapshots.length ? <List density="compact" edge="flush">
            {snapshots.map(item => <ListItem key={item.id} className="editor-snapshot-row" isSelected={item.id === selectedId} data-active={String(item.id === selectedId)}
              startContent={<Clock3 size="1em" />} label={<Text hasTabularNumbers maxLines={1}>{new Date(item.createdAt).toLocaleString('zh-CN', {hour12: false})}</Text>}
              description={snapshotLabel(item.reason)} endContent={<Text color="secondary" className="editor-snapshot-size">{formatBytes(item.bytes)}</Text>} onClick={() => setSelectedId(item.id)} />)}
          </List> : <Text className="panel-message" color="secondary">暂无快照，保存前会自动记录远程原文件。</Text>}
        </StackItem>
        {selected && <VStack className="editor-snapshot-preview" gap={0}>
          <HStack className="editor-preview-heading" gap={2} vAlign="center"><Text>快照预览 · {snapshotLabel(selected.reason)}</Text><Text color="secondary">{preview ? `UTF-8${preview.bom ? ' BOM' : ''} · ${preview.lineEnding}` : ''}</Text></HStack>
          {previewLoading ? <Text className="panel-message">正在读取版本…</Text> : preview && <CodeEditor name={file.name} value={preview.content} active={false} readOnly findRequest={0} onChange={() => {}} onCursor={() => {}} />}
        </VStack>}
        <VStack className="editor-history-footer" gap={2}>
          <Text color="secondary">快照仅保存在本机 · 每个文件保留最近 20 个版本</Text>
          <HStack gap={2} hAlign="end" wrap="wrap"><Button label="对比当前内容" size="sm" isDisabled={!preview || !selected || file.saving} onClick={() => {
            if (preview && selected) { controller.setComparison({documentId: file.id, original: preview, title: `历史快照 · ${new Date(selected.createdAt).toLocaleString('zh-CN', {hour12: false})}`, kind: 'snapshot'}); setHistoryOpen(false); }
          }} /><Button label="恢复到编辑器" size="sm" variant="primary" isDisabled={!preview || file.saving} onClick={() => {
            if (preview) { controller.setRestoreRequest({id: file.id, text: preview}); setHistoryOpen(false); }
          }} /></HStack>
          <Text color="secondary">恢复后需保存到远程才会生效。</Text>
        </VStack>
      </VStack>
    </Drawer>
  </VStack>;
}
