import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Table, pixel, proportional} from '@astryxdesign/core/Table';
import {Banner} from '@astryxdesign/core/Banner';
import {Bot, ExternalLink} from 'lucide-react';
import {isDesktop} from '../../shared/desktop';
import {useAiWorkspace, useAiWorkspaceFeed} from './ai-workspace';
import type {AiAgent, AiTerminalInfo} from './ai-workspace';

interface Row extends Record<string, unknown> {id: string; agent: AiAgent; targets: AiTerminalInfo[]; retained: AiTerminalInfo[]; status: string; running: boolean}
export function AgentPanel({mcpEnabled, mcpPending, mcpError, onRefreshMcp, onOpenTerminal}: {
  mcpEnabled: boolean; mcpPending: boolean; mcpError: string; onRefreshMcp: () => void; onOpenTerminal: (id: string) => void;
}) {
  const workspace = useAiWorkspace();
  const feed = useAiWorkspaceFeed();
  const connected = workspace.agents.filter(a => a.connected).length;
  const rows: Row[] = workspace.agents.map(agent => {
    const targets = workspace.terminals.filter(t => t.agentId === agent.id && t.status !== 'closed' && t.status !== 'error').sort((a, b) => b.updatedAt - a.updatedAt);
    const retained = workspace.terminals.filter(t => t.agentId === agent.id).sort((a, b) => b.updatedAt - a.updatedAt);
    const running = targets.some(t => t.status === 'running' || t.status === 'connecting');
    const interactive = targets.some(t => t.status === 'interactive');
    return {id: agent.id, agent, targets, retained, running,
      status: !agent.connected ? running ? '已断开 · 后台运行' : '已断开' : running ? '执行中' : interactive ? '交互中' : '已连接 · 空闲'};
  });
  return <VStack className="agent-panel" gap={0} height="100%">
    <HStack className="agent-toolbar" gap={2} vAlign="center">
      <StatusDot label={mcpEnabled ? 'MCP 已启用' : 'MCP 未启用'} variant={mcpEnabled ? 'success' : 'neutral'} />
      <Text>{mcpPending ? '正在读取 MCP 状态…' : mcpEnabled ? 'MCP 已启用' : 'MCP 未启用'}</Text>
      <Text color="secondary">{connected} 个 Agent 已连接</Text><StackItem size="fill" />
      <Button label="刷新" size="sm" variant="ghost" onClick={() => { onRefreshMcp(); void feed.refresh(); }} />
      <Button label="连接设置" href="/settings?section=mcp" size="sm" variant="ghost" />
    </HStack>
    {(mcpError || workspace.error) && <Banner className="compact-feedback" status="warning" title="Agent 状态暂不可用" description={mcpError || workspace.error} />}
    {rows.length ? <StackItem className="agent-table-wrap" size="fill" isScrollable><Table<Row> className="agent-table" data={rows} idKey="id" density="compact" dividers="rows" textOverflow="truncate" columns={[
      {key: 'agent', header: 'Agent', width: proportional(1.2, {minWidth: 150}), renderCell: row => <HStack gap={2} vAlign="center"><Bot size="1.1em" /><Text maxLines={1} hasTruncateTooltip>{row.agent.name} · {row.agent.id.slice(0, 5)}</Text></HStack>},
      {key: 'status', header: '连接状态', width: pixel(180), renderCell: row => <HStack gap={1} vAlign="center"><StatusDot label={row.status} isPulsing={row.running} variant={!row.agent.connected ? 'neutral' : row.running ? 'accent' : 'success'} /><Text>{row.status}</Text></HStack>},
      {key: 'targets', header: '正在操作的会话', width: proportional(1.8, {minWidth: 160}), renderCell: row => <HStack gap={1} wrap="wrap">{row.targets.length ? row.targets.map(target => <Button key={target.id} label={`${target.sessionName}${target.mode === 'takeover' ? ' · 原标签接管' : ' · 独立'}`} size="sm" variant="ghost" onClick={() => onOpenTerminal(target.id)} />) : <Text color="secondary">—</Text>}</HStack>},
      {key: 'action', header: '操作', width: pixel(140), renderCell: row => <Button label="查看标签页" endContent={<ExternalLink size="1em" />} size="sm" variant="ghost" isDisabled={!row.retained.length} onClick={() => { const target = row.targets[0] ?? row.retained[0]; if (target) onOpenTerminal(target.id); }} />},
    ]} /></StackItem> : <Text className="tools-empty" color="secondary">{workspace.loading ? '正在读取 Agent 连接…' : !isDesktop ? 'MCP 连接状态需要在桌面端查看。' : mcpEnabled ? '暂无 Agent 连接 · 在客户端配置 MCP 后，连接将显示在这里。' : '启用 MCP 后，可在这里查看 Agent 连接及所属会话。'}</Text>}
  </VStack>;
}
