import {useEffect, useState} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {ToggleButton} from '@astryxdesign/core/ToggleButton';
import {IconButton} from '@astryxdesign/core/IconButton';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {ArrowDownToLine, Bot, LockKeyhole, Search} from 'lucide-react';
import {TerminalPane} from '../terminal/terminal-pane';
import {useAiWorkspaceFeed, aiStatusLabels} from './ai-workspace';
import type {AiTerminalInfo} from './ai-workspace';
import {formatElapsed} from './execution-feed';

export function AiTerminalPane({info, active, fontSize, searchOpen, searchRequest, onOpenSearch, onCloseSearch}: {
  info: AiTerminalInfo; active: boolean; fontSize: number; searchOpen: boolean; searchRequest: number;
  onOpenSearch: () => void; onCloseSearch: () => void;
}) {
  const feed = useAiWorkspaceFeed();
  const runtime = feed.getRuntime(info.id);
  const [follow, setFollow] = useState(runtime.follow);
  const [clock, setClock] = useState(Date.now());
  useEffect(() => {
    if (!active || info.status !== 'running') return;
    setClock(Date.now());
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active, info.status]);
  const elapsed = info.status === 'running' && info.startedAt ? Math.max(0, clock - info.startedAt) : info.elapsedMs;
  const host = info.hostname.includes(':') ? `[${info.hostname}]` : info.hostname;
  return <VStack hidden={!active} className="ai-workspace-pane" gap={0}>
    <HStack className="ai-terminal-toolbar" gap={2} vAlign="center">
      <Bot className="ai-terminal-icon" aria-hidden /><Text className="ai-terminal-mode">{info.mode === 'takeover' ? '原标签接管' : '独立 Shell'}</Text>
      <Text className="ai-terminal-address" maxLines={1} hasTruncateTooltip>{info.agentName} / {info.sessionName} · {info.username}@{host}:{info.port}</Text>
      <StackItem size="fill" />
      <HStack className="ai-terminal-state" gap={1} vAlign="center"><StatusDot label={aiStatusLabels[info.status]} isPulsing={info.status === 'running' || info.status === 'connecting'}
        variant={info.status === 'error' ? 'error' : info.status === 'closed' ? 'neutral' : 'accent'} /><Text>{info.status === 'running' ? 'AI 操作中' : aiStatusLabels[info.status]}</Text></HStack>
      <HStack className="ai-terminal-readonly" gap={1} vAlign="center"><LockKeyhole size="1em" /><Text>仅查看</Text></HStack>
      <IconButton label="查找 AI 终端内容" tooltip="查找 · Ctrl + F" icon={<Search size="1em" />} size="sm" variant="ghost" onClick={onOpenSearch} />
      <ToggleButton label="自动跟随" tooltip="自动跟随最新输出" icon={<ArrowDownToLine size="1em" />} size="sm" isPressed={follow} onPressedChange={value => { runtime.follow = value; setFollow(value); if (value) runtime.terminal?.scrollToBottom(); }} />
    </HStack>
    <StackItem size="fill" className="ai-terminal-surface">
      <TerminalPane session={{id: info.id, profile: {name: `${info.agentName} · ${info.sessionName}`}, status: info.status === 'closed' ? 'closed' : info.status === 'error' ? 'error' : info.status === 'connecting' ? 'connecting' : 'connected'}}
        runtime={runtime} active={active} readOnly fontSize={fontSize} searchOpen={searchOpen} searchRequest={searchRequest} onOpenSearch={onOpenSearch} onCloseSearch={onCloseSearch} />
    </StackItem>
    <HStack className="ai-terminal-footer" hAlign="between" vAlign="center" gap={2}>
      <Text maxLines={1} hasTruncateTooltip>{info.message}</Text>
      <Text hasTabularNumbers>{info.status === 'running' ? `当前命令执行中 · ${formatElapsed(elapsed)}` : info.exitCode !== null ? `退出码 ${info.exitCode} · ${formatElapsed(elapsed)}` : '退出码 —'}</Text>
    </HStack>
  </VStack>;
}
