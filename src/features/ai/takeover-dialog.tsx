import {useState} from 'react';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {VStack, HStack} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Selector} from '@astryxdesign/core/Selector';
import {Banner} from '@astryxdesign/core/Banner';
import {Bot} from 'lucide-react';
import {usePreferences} from '../../app/preferences';
import {desktopCommand, errorMessage} from '../../shared/desktop';
import type {ShellSession} from '../terminal/runtime';
import {useAiWorkspace, useAiWorkspaceFeed} from './ai-workspace';
import type {AiTerminalInfo} from './ai-workspace';

export function AiTakeoverDialog({session, onClose, onTakenOver}: {
  session: ShellSession; onClose: () => void; onTakenOver: (info: AiTerminalInfo) => void;
}) {
  const {uiDensity} = usePreferences();
  const workspace = useAiWorkspace();
  const feed = useAiWorkspaceFeed();
  const agents = workspace.agents.filter(agent => agent.connected);
  const [selected, setSelected] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const agent = agents.find(agent => agent.id === selected);
  const host = session.profile.hostname.includes(':') ? `[${session.profile.hostname}]` : session.profile.hostname;
  async function takeOver() {
    if (busy || !agent || session.status !== 'connected') return;
    setBusy(true); setError('');
    try {
      const info = await desktopCommand<AiTerminalInfo>('ai_take_over_session', {sessionId: session.id, agentId: agent.id});
      void feed.refresh(); onTakenOver(info);
    } catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  }
  return <Dialog isOpen className="orbit-dialog" data-density={uiDensity} purpose={busy ? 'required' : 'form'} width="calc(var(--orbit-unit) * 42)" padding={0}
    maxHeight="calc(100dvh - var(--orbit-unit) * 6)" onOpenChange={open => { if (!open && !busy) onClose(); }}>
    <VStack className="orbit-dialog-layout" gap={0}>
      <DialogHeader className="orbit-dialog-header" title="交给 AI 接管原终端" hasDivider startContent={<Bot size="1.2em" />} onOpenChange={open => { if (!open && !busy) onClose(); }} />
      <VStack className="orbit-dialog-body" gap={4}>
        <VStack gap={1}><Text>{session.profile.name}</Text><Text className="mono-text" color="secondary">{host}:{session.profile.port}</Text></VStack>
        <Text>接管当前标签页正在使用的 Shell，保留目录、环境变量和堡垒机登录状态。人工输入暂时锁定，可随时交回人工。</Text>
        {workspace.error && <Banner status="warning" title="无法读取 Agent 状态" description={workspace.error} endContent={<Button label="刷新" size="sm" onClick={() => { void feed.refresh(); }} />} />}
        {agents.length ? <Selector label="接管的 Agent" value={selected} onChange={setSelected} placeholder="选择已连接的 Agent" width="100%" isDisabled={busy}
          options={agents.map(agent => ({value: agent.id, label: `${agent.name} · ${agent.id.slice(0, 5)}${agent.version ? ` · ${agent.version}` : ''}`}))} /> : <Banner status="info" title={workspace.loading ? '正在读取 Agent…' : '暂无已连接的 Agent'} description="先开启 MCP，并在客户端连接 OrbitShell。"
            endContent={<Button label="MCP 设置" size="sm" href="/settings?section=mcp" />} />}
        {selected && !agent && <Text color="secondary">所选 Agent 已断开，请重新选择。</Text>}
        {error && <Banner status="error" title="接管未完成" description={error} />}
      </VStack>
      <HStack className="orbit-dialog-footer" gap={2} hAlign="end"><Button label="取消" isDisabled={busy} onClick={onClose} /><Button label="接管原终端" icon={<Bot size="1em" />} variant="primary" isLoading={busy}
        isDisabled={busy || !agent || session.status !== 'connected'} onClick={() => { void takeOver(); }} /></HStack>
    </VStack>
  </Dialog>;
}
