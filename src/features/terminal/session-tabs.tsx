// Adapted from Astryx's ItemDocumentTabs block: label and close are sibling controls.
import {useEffect, useRef} from 'react';
import type {KeyboardEvent} from 'react';
import {HStack} from '@astryxdesign/core/Stack';
import {Item} from '@astryxdesign/core/Item';
import {IconButton} from '@astryxdesign/core/IconButton';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Text} from '@astryxdesign/core/Text';
import {ContextMenu} from '@astryxdesign/core/ContextMenu';
import {Copy, FileText, History, Plus, RefreshCw, Save, Unplug, X} from 'lucide-react';
import type {ShellSession} from './runtime';
import {MenuShortcut, openContextMenuFromKeyboard, useCopyText} from '../../shared/context-menu';
import type {EditorTab} from '../editor/types';

interface Props {
  sessions: ShellSession[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onCreate: () => void;
  onReconnect?: (id: string) => void;
  onDisconnect?: (id: string) => void;
  editors?: EditorTab[];
  activeEditorId?: string | null;
  onSelectEditor?: (id: string) => void;
  onCloseEditor?: (id: string) => void;
  onSaveEditor?: (id: string) => void;
  onEditorHistory?: (id: string) => void;
}

export function SessionTabs({sessions, activeId, onSelect, onClose, onCreate, onReconnect, onDisconnect, editors = [], activeEditorId, onSelectEditor, onCloseEditor, onSaveEditor, onEditorHistory}: Props) {
  const items = useRef(new Map<string, HTMLElement>());
  const strip = useRef<HTMLDivElement | null>(null);
  const copy = useCopyText();
  useEffect(() => {
    const id = activeEditorId ?? activeId;
    const element = strip.current;
    if (!id || !element) return;
    const keepVisible = () => {
      const item = items.current.get(id);
      if (!item) return;
      const row = element.getBoundingClientRect();
      const tab = item.getBoundingClientRect();
      // Scroll only the tab strip, including after a sidebar or window resize.
      if (tab.left < row.left) element.scrollLeft -= row.left - tab.left;
      else if (tab.right > row.right) element.scrollLeft += tab.right - row.right;
    };
    keepVisible();
    const observer = new ResizeObserver(keepVisible);
    observer.observe(element);
    return () => observer.disconnect();
  }, [activeId, activeEditorId, sessions.length, editors.length]);
  const tabIds = [...sessions.map(item => item.id), ...editors.map(item => item.id)];
  const navigateTabs = (event: KeyboardEvent<HTMLElement>, index: number) => {
    if (event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'Delete') {
      event.preventDefault(); const id = tabIds[index];
      if (id) { if (index < sessions.length) onClose(id); else onCloseEditor?.(id); }
      return;
    }
    const nextIndex = event.key === 'ArrowRight' ? (index + 1) % tabIds.length : event.key === 'ArrowLeft' ? (index + tabIds.length - 1) % tabIds.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabIds.length - 1 : null;
    const id = nextIndex === null ? null : tabIds[nextIndex];
    if (!id || nextIndex === null) return;
    event.preventDefault();
    if (nextIndex < sessions.length) onSelect(id); else onSelectEditor?.(id);
    items.current.get(id)?.querySelector<HTMLButtonElement>('button')?.focus();
  };

  if (!sessions.length && !editors.length) return <HStack gap={0} vAlign="center"><Text className="tab-placeholder" type="supporting" color="secondary">终端工作区</Text><IconButton className="new-session" label="打开新会话" tooltip="打开新会话" icon={<Plus />} variant="ghost" onClick={onCreate} /></HStack>;
  return <HStack ref={strip} role="group" aria-label="SSH 会话" gap={0} className="session-strip">
    {sessions.map((session, index) => {
      const active = !activeEditorId && session.id === activeId;
      const label = {connecting: '正在连接', hostKey: '等待确认指纹', connected: '已连接', closed: '已断开', error: '连接失败'}[session.status];
      const host = session.profile.hostname;
      const address = `${host.includes(':') && !host.startsWith('[') ? `[${host}]` : host}:${session.profile.port}`;
      return <ContextMenu key={session.id} data-orbit-context="session" className="orbit-context-menu" label={`${session.profile.name}会话操作`} size="sm" menuWidth="calc(var(--orbit-unit) * 23)"
        onContextMenu={event => event.stopPropagation()} onKeyDownCapture={openContextMenuFromKeyboard} items={[
          {label: '重新连接', icon: <RefreshCw />, isDisabled: !onReconnect || session.status === 'connecting' || session.status === 'hostKey', onClick: () => onReconnect?.(session.id)},
          {label: '复制服务器地址', icon: <Copy />, onClick: () => { void copy(address); }},
          {type: 'divider'},
          {label: '断开连接', icon: <Unplug />, isDisabled: !onDisconnect || session.status !== 'connected', onClick: () => onDisconnect?.(session.id)},
          {label: '关闭会话标签', icon: <X />, endContent: <MenuShortcut>鼠标中键</MenuShortcut>, onClick: () => onClose(session.id)},
        ]}>
      <Item ref={element => { if (element) items.current.set(session.id, element); else items.current.delete(session.id); }}
        className="session-tab" data-active={String(active)} density="compact" isSelected={active}
        label={<Text className="session-tab-label" maxLines={1}>{session.profile.name}</Text>} labelLines={1} aria-current={active ? 'true' : undefined}
        aria-description={`${session.profile.hostname}:${session.profile.port} · ${label}`}
        startContent={<StatusDot label={label} variant={session.status === 'connected' ? 'success' : session.status === 'error' ? 'error' : session.status === 'closed' ? 'neutral' : 'warning'} isPulsing={session.status === 'connecting'} />}
        endContent={<IconButton className="session-close" label={`关闭会话 ${session.profile.name}`} tooltip="关闭会话 · 鼠标中键"
          icon={<X size="1em" />} size="sm" variant="ghost" onClick={event => { event.stopPropagation(); onClose(session.id); }} />}
        onClick={() => onSelect(session.id)}
        onMouseDown={event => { if (event.button === 1) event.preventDefault(); }}
        onAuxClick={event => { if (event.button === 1) { event.preventDefault(); onClose(session.id); } }}
        onKeyDown={event => navigateTabs(event, index)} />
      </ContextMenu>;
    })}
    {editors.map((file, index) => <ContextMenu key={file.id} data-orbit-context="session" className="orbit-context-menu" label={`${file.name}文件标签操作`} size="sm" menuWidth="calc(var(--orbit-unit) * 20)"
      onContextMenu={event => event.stopPropagation()} onKeyDownCapture={openContextMenuFromKeyboard} items={[
        {label: '保存到远程', icon: <Save />, isDisabled: !file.canSave || !file.dirty || file.saving, onClick: () => onSaveEditor?.(file.id)},
        {label: '快照历史', icon: <History />, onClick: () => onEditorHistory?.(file.id)},
        {label: '复制远程路径', icon: <Copy />, onClick: () => { void copy(file.path); }},
        {type: 'divider'},
        {label: '关闭文件标签', icon: <X />, isDisabled: file.saving, onClick: () => onCloseEditor?.(file.id)},
      ]}>
      <Item ref={element => { if (element) items.current.set(file.id, element); else items.current.delete(file.id); }} className="session-tab editor-tab" density="compact" labelLines={1}
        data-active={String(file.id === activeEditorId)} isSelected={file.id === activeEditorId} aria-current={file.id === activeEditorId ? 'true' : undefined}
        aria-description={`${file.sessionName} · ${file.path}${file.dirty ? ' · 未保存' : ''}`} label={<Text className="session-tab-label" maxLines={1}>{file.name}</Text>}
        startContent={<FileText className="session-file-icon" aria-hidden />}
        endContent={<HStack gap={1} vAlign="center">{(file.dirty || file.saving) && <StatusDot className="editor-dirty-dot" variant="warning" label={file.saving ? '正在保存' : '未保存'} isPulsing={file.saving} />}
          <IconButton className="session-close" label={`关闭文件 ${file.name}`} tooltip="关闭文件 · 鼠标中键" icon={<X size="1em" />} size="sm" variant="ghost" isDisabled={file.saving}
            onClick={event => { event.stopPropagation(); onCloseEditor?.(file.id); }} /></HStack>}
        onClick={() => onSelectEditor?.(file.id)} onMouseDown={event => { if (event.button === 1) event.preventDefault(); }}
        onAuxClick={event => { if (event.button === 1) { event.preventDefault(); if (!file.saving) onCloseEditor?.(file.id); } }} onKeyDown={event => navigateTabs(event, sessions.length + index)} />
    </ContextMenu>)}
    <IconButton className="new-session" label="打开新会话" tooltip="打开新会话" icon={<Plus />} variant="ghost" onClick={onCreate} />
  </HStack>;
}
