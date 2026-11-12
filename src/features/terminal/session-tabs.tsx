// Adapted from Astryx's ItemDocumentTabs block: label and close are sibling controls.
import {useEffect, useRef} from 'react';
import {HStack} from '@astryxdesign/core/Stack';
import {Item} from '@astryxdesign/core/Item';
import {IconButton} from '@astryxdesign/core/IconButton';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Text} from '@astryxdesign/core/Text';
import {ContextMenu} from '@astryxdesign/core/ContextMenu';
import {Copy, Plus, RefreshCw, Unplug, X} from 'lucide-react';
import type {ShellSession} from './runtime';
import {MenuShortcut, openContextMenuFromKeyboard, useCopyText} from '../../shared/context-menu';

interface Props {
  sessions: ShellSession[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onCreate: () => void;
  onReconnect?: (id: string) => void;
  onDisconnect?: (id: string) => void;
}

export function SessionTabs({sessions, activeId, onSelect, onClose, onCreate, onReconnect, onDisconnect}: Props) {
  const items = useRef(new Map<string, HTMLElement>());
  const copy = useCopyText();
  useEffect(() => {
    if (activeId) items.current.get(activeId)?.scrollIntoView({block: 'nearest', inline: 'nearest'});
  }, [activeId, sessions.length]);

  if (!sessions.length) return <HStack gap={0} vAlign="center"><Text className="tab-placeholder" type="supporting" color="secondary">终端工作区</Text><IconButton className="new-session" label="打开新会话" tooltip="打开新会话" icon={<Plus />} variant="ghost" onClick={onCreate} /></HStack>;
  return <HStack role="group" aria-label="SSH 会话" gap={0} className="session-strip">
    {sessions.map((session, index) => {
      const active = session.id === activeId;
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
        onKeyDown={event => {
          if (event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
          if (event.key === 'Delete') { event.preventDefault(); onClose(session.id); return; }
          const nextIndex = event.key === 'ArrowRight' ? (index + 1) % sessions.length :
            event.key === 'ArrowLeft' ? (index + sessions.length - 1) % sessions.length :
            event.key === 'Home' ? 0 : event.key === 'End' ? sessions.length - 1 : null;
          const next = nextIndex === null ? null : sessions[nextIndex];
          if (!next) return;
          event.preventDefault();
          onSelect(next.id);
          items.current.get(next.id)?.querySelector<HTMLButtonElement>('button')?.focus();
        }} />
      </ContextMenu>;
    })}
    <IconButton className="new-session" label="打开新会话" tooltip="打开新会话" icon={<Plus />} variant="ghost" onClick={onCreate} />
  </HStack>;
}
