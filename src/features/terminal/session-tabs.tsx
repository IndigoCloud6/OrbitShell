// Adapted from Astryx's ItemDocumentTabs block: label and close are sibling controls.
import {useEffect, useRef} from 'react';
import type {KeyboardEvent, ReactNode} from 'react';
import {HStack} from '@astryxdesign/core/Stack';
import {Item} from '@astryxdesign/core/Item';
import {IconButton} from '@astryxdesign/core/IconButton';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Text} from '@astryxdesign/core/Text';
import {ContextMenu} from '@astryxdesign/core/ContextMenu';
import {Bot, Copy, FileText, History, LoaderCircle, Plus, RefreshCw, Save, Unplug, X} from 'lucide-react';
import type {ShellSession} from './runtime';
import {MenuShortcut, openContextMenuFromKeyboard, useCopyText} from '../../shared/context-menu';
import type {EditorTab} from '../editor/types';
import {aiStatusLabels} from '../ai/ai-workspace';
import type {AiTerminalInfo} from '../ai/ai-workspace';

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
  aiTerminals?: AiTerminalInfo[];
  activeAiId?: string | null;
  unreadAiIds?: ReadonlySet<string>;
  onSelectAi?: (id: string) => void;
  onCloseAi?: (id: string) => void;
  takeovers?: AiTerminalInfo[];
  onReleaseAi?: (id: string) => void;
  onTakeOverAi?: (id: string) => void;
  utilityTabs?: {id: string; label: string; icon: ReactNode}[];
  activeUtilityId?: string | null;
  onSelectUtility?: (id: string) => void;
  onCloseUtility?: (id: string) => void;
}

export function SessionTabs({sessions, activeId, onSelect, onClose, onCreate, onReconnect, onDisconnect, editors = [], activeEditorId, onSelectEditor, onCloseEditor, onSaveEditor, onEditorHistory, aiTerminals = [], activeAiId, unreadAiIds, onSelectAi, onCloseAi, takeovers = [], onReleaseAi, onTakeOverAi, utilityTabs = [], activeUtilityId, onSelectUtility, onCloseUtility}: Props) {
  const items = useRef(new Map<string, HTMLElement>());
  const strip = useRef<HTMLDivElement | null>(null);
  const copy = useCopyText();
  useEffect(() => {
    const id = activeUtilityId ?? activeAiId ?? activeEditorId ?? activeId;
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
  }, [activeId, activeEditorId, activeAiId, sessions.length, editors.length, aiTerminals.length, activeUtilityId, utilityTabs.length]);
  const tabIds = [...sessions.map(item => item.id), ...editors.map(item => item.id), ...aiTerminals.map(item => item.id), ...utilityTabs.map(item => item.id)];
  const navigateTabs = (event: KeyboardEvent<HTMLElement>, index: number) => {
    if (event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === 'Delete') {
      event.preventDefault(); const id = tabIds[index];
      if (id) { if (index < sessions.length) onClose(id); else if (index < sessions.length + editors.length) onCloseEditor?.(id); else if (index < sessions.length + editors.length + aiTerminals.length) onCloseAi?.(id); else onCloseUtility?.(id); }
      return;
    }
    const nextIndex = event.key === 'ArrowRight' ? (index + 1) % tabIds.length : event.key === 'ArrowLeft' ? (index + tabIds.length - 1) % tabIds.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabIds.length - 1 : null;
    const id = nextIndex === null ? null : tabIds[nextIndex];
    if (!id || nextIndex === null) return;
    event.preventDefault();
    if (nextIndex < sessions.length) onSelect(id); else if (nextIndex < sessions.length + editors.length) onSelectEditor?.(id); else if (nextIndex < sessions.length + editors.length + aiTerminals.length) onSelectAi?.(id); else onSelectUtility?.(id);
    items.current.get(id)?.querySelector<HTMLButtonElement>('button')?.focus();
  };

  if (!sessions.length && !editors.length && !aiTerminals.length && !utilityTabs.length) return <HStack gap={0} vAlign="center"><Text className="tab-placeholder session-start-label">开始</Text><IconButton className="new-session" label="打开新会话" tooltip="打开新会话" icon={<Plus />} variant="ghost" onClick={onCreate} /></HStack>;
  return <HStack ref={strip} role="group" aria-label="SSH 会话" gap={0} className="session-strip">
    {sessions.map((session, index) => {
      const active = !activeUtilityId && !activeAiId && !activeEditorId && session.id === activeId;
      const owner = takeovers.find(terminal => terminal.sessionId === session.id);
      const label = {connecting: '正在连接', hostKey: '等待确认指纹', connected: '已连接', closed: '已断开', error: '连接失败'}[session.status];
      const host = session.profile.hostname;
      const address = `${host.includes(':') && !host.startsWith('[') ? `[${host}]` : host}:${session.profile.port}`;
      return <ContextMenu key={session.id} data-orbit-context="session" className="orbit-context-menu" label={`${session.profile.name}会话操作`} size="sm" menuWidth="calc(var(--orbit-unit) * 23)"
        onContextMenu={event => event.stopPropagation()} onKeyDownCapture={openContextMenuFromKeyboard} items={[
          ...(owner ? [{label: '交回人工', icon: <Bot />, onClick: () => onReleaseAi?.(owner.id)}] : [{label: '交给 AI 接管原终端…', icon: <Bot />, isDisabled: !onTakeOverAi || session.status !== 'connected', onClick: () => onTakeOverAi?.(session.id)}]),
          {label: '重新连接', icon: <RefreshCw />, isDisabled: Boolean(owner) || !onReconnect || session.status === 'connecting' || session.status === 'hostKey', onClick: () => onReconnect?.(session.id)},
          {label: '复制服务器地址', icon: <Copy />, onClick: () => { void copy(address); }},
          {type: 'divider'},
          {label: '断开连接', icon: <Unplug />, isDisabled: !onDisconnect || session.status !== 'connected', onClick: () => onDisconnect?.(session.id)},
          {label: '关闭会话标签', icon: <X />, endContent: <MenuShortcut>鼠标中键</MenuShortcut>, onClick: () => onClose(session.id)},
        ]}>
      <Item ref={element => { if (element) items.current.set(session.id, element); else items.current.delete(session.id); }}
        className={owner ? 'session-tab ai-session-tab' : 'session-tab'} data-active={String(active)} density="compact" isSelected={active}
        label={<Text className="session-tab-label" maxLines={1}>{session.profile.name}{owner ? ` · ${owner.agentName}` : ''}</Text>} labelLines={1} aria-current={active ? 'true' : undefined}
        aria-description={`${session.profile.hostname}:${session.profile.port} · ${label}`}
        startContent={owner ? <HStack className="ai-tab-mark" gap={1} vAlign="center"><Bot size="1em" /><Text>AI</Text></HStack> : <StatusDot label={label} variant={session.status === 'connected' ? 'success' : session.status === 'error' ? 'error' : session.status === 'closed' ? 'neutral' : 'warning'} isPulsing={session.status === 'connecting'} />}
        endContent={<HStack gap={1} vAlign="center">{owner && <LoaderCircle className="ai-running-spinner" size="1em" aria-label="AI 接管中" />}
          {owner && !active && unreadAiIds?.has(owner.id) && <StatusDot label="AI 会话有新输出" variant="accent" />}
          <IconButton className="session-close" label={`关闭会话 ${session.profile.name}`} tooltip={owner ? '关闭会话并断开 SSH 连接' : '关闭会话 · 鼠标中键'}
          icon={<X size="1em" />} size="sm" variant="ghost" onClick={event => { event.stopPropagation(); onClose(session.id); }} /></HStack>}
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
        data-active={String(!activeUtilityId && !activeAiId && file.id === activeEditorId)} isSelected={!activeUtilityId && !activeAiId && file.id === activeEditorId} aria-current={!activeUtilityId && !activeAiId && file.id === activeEditorId ? 'true' : undefined}
        aria-description={`${file.sessionName} · ${file.path}${file.dirty ? ' · 未保存' : ''}`} label={<Text className="session-tab-label" maxLines={1}>{file.name}</Text>}
        startContent={<FileText className="session-file-icon" aria-hidden />}
        endContent={<HStack gap={1} vAlign="center">{(file.dirty || file.saving) && <StatusDot className="editor-dirty-dot" variant="warning" label={file.saving ? '正在保存' : '未保存'} isPulsing={file.saving} />}
          <IconButton className="session-close" label={`关闭文件 ${file.name}`} tooltip="关闭文件 · 鼠标中键" icon={<X size="1em" />} size="sm" variant="ghost" isDisabled={file.saving}
            onClick={event => { event.stopPropagation(); onCloseEditor?.(file.id); }} /></HStack>}
        onClick={() => onSelectEditor?.(file.id)} onMouseDown={event => { if (event.button === 1) event.preventDefault(); }}
        onAuxClick={event => { if (event.button === 1) { event.preventDefault(); if (!file.saving) onCloseEditor?.(file.id); } }} onKeyDown={event => navigateTabs(event, sessions.length + index)} />
    </ContextMenu>)}
    {aiTerminals.map((terminal, index) => {
      const active = !activeUtilityId && terminal.id === activeAiId;
      const running = terminal.status === 'running' || terminal.status === 'connecting';
      const name = `${terminal.agentName} · ${terminal.sessionName}`;
      return <ContextMenu key={terminal.id} data-orbit-context="session" className="orbit-context-menu" label={`${name} AI 标签操作`} size="sm" menuWidth="calc(var(--orbit-unit) * 23)"
        onContextMenu={event => event.stopPropagation()} onKeyDownCapture={openContextMenuFromKeyboard} items={[
          {label: '复制服务器地址', icon: <Copy />, onClick: () => { void copy(`${terminal.hostname.includes(':') ? `[${terminal.hostname}]` : terminal.hostname}:${terminal.port}`); }},
          {label: '关闭视图，后台继续运行', icon: <X />, onClick: () => onCloseAi?.(terminal.id)},
        ]}>
        <Item ref={element => { if (element) items.current.set(terminal.id, element); else items.current.delete(terminal.id); }} className="session-tab ai-session-tab" density="compact" labelLines={1}
          data-active={String(active)} isSelected={active} aria-current={active ? 'true' : undefined} aria-description={`AI 操作 · ${aiStatusLabels[terminal.status]}${unreadAiIds?.has(terminal.id) ? ' · 有新输出' : ''}`}
          label={<Text className="session-tab-label" maxLines={1}>{name}</Text>}
          startContent={<HStack className="ai-tab-mark" gap={1} vAlign="center"><Bot size="1em" /><Text>AI</Text></HStack>}
          endContent={<HStack gap={1} vAlign="center">{running && <LoaderCircle className="ai-running-spinner" size="1em" aria-label="AI 后台运行中" />}
            {unreadAiIds?.has(terminal.id) && !active && <StatusDot label="有新输出" variant="accent" />}
            <IconButton className="session-close" label={`关闭 AI 视图 ${name}`} tooltip="关闭视图 · 后台任务继续运行" icon={<X size="1em" />} size="sm" variant="ghost" onClick={event => { event.stopPropagation(); onCloseAi?.(terminal.id); }} /></HStack>}
          onClick={() => onSelectAi?.(terminal.id)} onMouseDown={event => { if (event.button === 1) event.preventDefault(); }}
          onAuxClick={event => { if (event.button === 1) { event.preventDefault(); onCloseAi?.(terminal.id); } }}
          onKeyDown={event => navigateTabs(event, sessions.length + editors.length + index)} />
      </ContextMenu>;
    })}
    {utilityTabs.map((tab, index) => <ContextMenu key={tab.id} data-orbit-context="session" className="orbit-context-menu" label={`${tab.label}标签操作`} size="sm"
      onContextMenu={event => event.stopPropagation()} onKeyDownCapture={openContextMenuFromKeyboard} items={[{label: '关闭标签页', icon: <X />, onClick: () => onCloseUtility?.(tab.id)}]}>
      <Item ref={element => { if (element) items.current.set(tab.id, element); else items.current.delete(tab.id); }} className="session-tab utility-tab" density="compact"
        isSelected={activeUtilityId === tab.id} data-active={String(activeUtilityId === tab.id)} aria-current={activeUtilityId === tab.id ? 'true' : undefined}
        label={<Text className="session-tab-label">{tab.label}</Text>} startContent={tab.icon}
        endContent={<IconButton className="session-close" label={`关闭${tab.label}`} tooltip="关闭标签 · 鼠标中键" icon={<X size="1em" />} size="sm" variant="ghost" onClick={event => { event.stopPropagation(); onCloseUtility?.(tab.id); }} />}
        onClick={() => onSelectUtility?.(tab.id)} onMouseDown={event => { if (event.button === 1) event.preventDefault(); }} onAuxClick={event => { if (event.button === 1) { event.preventDefault(); onCloseUtility?.(tab.id); } }}
        onKeyDown={event => navigateTabs(event, sessions.length + editors.length + aiTerminals.length + index)} />
    </ContextMenu>)}
    <IconButton className="new-session" label="打开新会话" tooltip="打开新会话" icon={<Plus />} variant="ghost" onClick={onCreate} />
  </HStack>;
}
