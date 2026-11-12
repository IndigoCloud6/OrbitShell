import type {ReactNode} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import type {DropdownMenuOption} from '@astryxdesign/core/DropdownMenu';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Minus, Square, X, Server, Folder, Code2, Clock3, Network, Settings, Search, Columns2, RefreshCw} from 'lucide-react';
import type {ShellSession} from '../terminal/runtime';
import {statusLabel} from './status';
import orbitAppIcon from '../../../src-tauri/icons/icon.png';

export interface IdeMenu {label: string; items: DropdownMenuOption[]}
export type ToolTab = 'transfers' | 'logs' | 'ai' | 'commands';

export function OrbitMark({className}: {className?: string}) {
  return <img className={['orbit-mark', className].filter(Boolean).join(' ')} src={orbitAppIcon} alt="" aria-hidden="true" draggable={false} />;
}

export function IdeTitlebar({menus, onWindow}: {menus: IdeMenu[]; onWindow: (action: 'minimize' | 'maximize' | 'close') => void}) {
  return <HStack className="workspace-header" gap={0} vAlign="center" data-tauri-drag-region>
    <HStack className="brand" gap={2} data-tauri-drag-region><OrbitMark className="brand-orbit" /><Text className="brand-name" data-tauri-drag-region>OrbitShell</Text></HStack>
    <HStack className="ide-menubar" gap={0} aria-label="应用菜单">
      {menus.map(menu => <DropdownMenu key={menu.label} presentation="popover" hasChevron={false} menuWidth="max-content" button={{label: menu.label, variant: 'ghost', size: 'sm', className: 'ide-menu-button'}} items={menu.items} />)}
    </HStack>
    <Text className="workspace-title" color="secondary" data-tauri-drag-region>远程工作区</Text>
    <StackItem className="header-drag-space" size="fill" data-tauri-drag-region />
    <HStack className="window-controls" gap={0}>
      <IconButton label="最小化" tooltip="最小化" className="window-action" icon={<Minus />} variant="ghost" onClick={() => onWindow('minimize')} />
      <IconButton label="最大化或还原" tooltip="最大化或还原" className="window-action" icon={<Square />} variant="ghost" onClick={() => onWindow('maximize')} />
      <IconButton label="关闭窗口" tooltip="关闭窗口" className="window-action window-close" icon={<X />} variant="ghost" onClick={() => onWindow('close')} />
    </HStack>
  </HStack>;
}

export function ActivityRail({showServers, showFiles, toolTab, toolsOpen, onServers, onFiles, onSessions, onTool, onSettings}: {
  showServers: boolean; showFiles: boolean; toolTab: ToolTab; toolsOpen: boolean;
  onServers: () => void; onFiles: () => void; onSessions: () => void; onTool: (tab: ToolTab) => void; onSettings: () => void;
}) {
  return <VStack className="activity-rail" gap={0} height="100%">
    <IconButton label="连接列表" tooltip="连接列表" className="activity-action" icon={<Server aria-hidden />} data-active={String(showServers)} aria-pressed={showServers} variant="ghost" onClick={onServers} />
    <IconButton label="远程文件" tooltip="远程文件" className="activity-action" icon={<Folder aria-hidden />} data-active={String(showFiles)} aria-pressed={showFiles} variant="ghost" onClick={onFiles} />
    <IconButton label="打开会话" tooltip="打开会话" className="activity-action" icon={<Code2 aria-hidden />} variant="ghost" onClick={onSessions} />
    <IconButton label="连接日志" tooltip="连接日志" className="activity-action" icon={<Clock3 aria-hidden />} data-active={String(toolsOpen && toolTab === 'logs')} aria-pressed={toolsOpen && toolTab === 'logs'} variant="ghost" onClick={() => onTool('logs')} />
    <IconButton label="AI 工具" tooltip="AI 工具 · MCP" className="activity-action" icon={<Network aria-hidden />} data-active={String(toolsOpen && toolTab === 'ai')} aria-pressed={toolsOpen && toolTab === 'ai'} variant="ghost" onClick={() => onTool('ai')} />
    <StackItem size="fill" />
    <IconButton label="设置" tooltip="设置" className="activity-action activity-settings" icon={<Settings aria-hidden />} variant="ghost" onClick={onSettings} />
  </VStack>;
}

export function SessionContextBar({session, onSearch, onReconnect, onFiles}: {session: ShellSession; onSearch: () => void; onReconnect: () => void; onFiles: () => void}) {
  return <HStack className="session-info" vAlign="center" gap={2}>
    <Text className="context-breadcrumb" maxLines={1}>{session.profile.group} / {session.profile.name}</Text>
    <Text className="context-separator" color="secondary">·</Text>
    <Text className="session-address" color="secondary" maxLines={1}>{session.profile.username.startsWith('b64>>') ? '堡垒机会话' : session.profile.username}@{session.profile.hostname}:{session.profile.port}</Text>
    <StackItem size="fill" />
    <IconButton className="context-action" label="查找终端" tooltip="查找终端 · Ctrl + F" size="sm" variant="ghost" icon={<Search />} onClick={onSearch} />
    <IconButton className="context-action" label="显示或隐藏远程文件" tooltip="远程文件" size="sm" variant="ghost" icon={<Columns2 />} onClick={onFiles} />
    <IconButton className="context-action" label="重新连接" tooltip="重新连接" size="sm" variant="ghost" icon={<RefreshCw />} isDisabled={session.status === 'connecting' || session.status === 'hostKey'} onClick={onReconnect} />
  </HStack>;
}

export function IdeStatusbar({active, mcpEnabled, children}: {active: ShellSession | null; mcpEnabled: boolean; children?: ReactNode}) {
  return <HStack className="statusbar" hAlign="between" vAlign="center" gap={2}>
    <HStack className="status-left" gap={0} vAlign="center">
      <HStack gap={2} className="status-segment" vAlign="center"><StatusDot label={active ? statusLabel(active) : '暂无会话'} variant={active?.status === 'connected' ? 'success' : active?.status === 'error' ? 'error' : 'neutral'} /><Text>{active ? statusLabel(active) : '暂无会话'}</Text></HStack>
      {active && <Text className="status-segment">{active.profile.name}</Text>}
    </HStack>
    <HStack className="status-right" gap={0} vAlign="center">
      {children}
      <Text className="status-segment">UTF-8</Text>
      {active && <Text className="status-segment" hasTabularNumbers>{active.columns} × {active.rows}</Text>}
      <Text className="status-segment">SSH2</Text>
      <Button className="status-mcp" label="MCP" variant="ghost" size="sm" icon={<Network size="1em" />} data-enabled={String(mcpEnabled)} href="/settings?section=mcp" />
    </HStack>
  </HStack>;
}
