import {useState} from 'react';
import {useNavigate, useSearchParams} from 'react-router';
import {useQuery, useQueryClient} from '@tanstack/react-query';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {VStack, HStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Selector} from '@astryxdesign/core/Selector';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Banner} from '@astryxdesign/core/Banner';
import {CheckboxInput} from '@astryxdesign/core/CheckboxInput';
import {List, ListItem} from '@astryxdesign/core/List';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Moon, Sun, Monitor, Palette, Network, ShieldCheck, Copy, Link2} from 'lucide-react';
import {usePreferences} from '../app/preferences';
import {useThemePreference} from '../app/theme';
import {desktopCommand, errorMessage, isDesktop} from '../shared/desktop';
import {desktopStatusKey} from '../shared/desktop-lifecycle';
import type {DesktopStatus} from '../shared/desktop-lifecycle';
import {McpToolList} from '../features/workbench/mcp-tool-list';
import {useMcpStatus} from '../features/ai/mcp-status';

const shortcuts = [
  {label: '新建连接', keys: 'Ctrl + Shift + T'},
  {label: '关闭当前会话', keys: 'Ctrl + Shift + W'},
  {label: '查找终端', keys: 'Ctrl + F'},
  {label: '复制终端选区', keys: 'Ctrl + C'},
  {label: '粘贴到终端', keys: 'Ctrl + V'},
  {label: '调整终端字号', keys: 'Ctrl + 加号 / 减号'},
  {label: '恢复默认字号', keys: 'Ctrl + 0'},
  {label: '工具面板', keys: 'Ctrl + J'},
  {label: '连接列表', keys: 'Ctrl + B'},
];

export default function SettingsDialog() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const tab = params.get('section') === 'bastion' ? 'bastion' : params.get('section') === 'mcp' ? 'mcp' : 'appearance';
  const theme = useThemePreference();
  const preferences = usePreferences();
  const cache = useQueryClient();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState('');
  const mcp = useMcpStatus();
  const desktop = useQuery<DesktopStatus>({queryKey: desktopStatusKey, queryFn: () => desktopCommand('desktop_status'), enabled: isDesktop, retry: false});
  const [link, setLink] = useState('');
  const [usernameMode, setUsernameMode] = useState('preserve');

  function selectSection(section: 'appearance' | 'mcp' | 'bastion') {
    const next = new URLSearchParams(params);
    next.set('section', section);
    void setParams(next, {replace: true});
    setError(''); setFeedback('');
  }

  async function toggleMcp() {
    setError(''); setBusy(true); setFeedback('');
    try { await desktopCommand('mcp_set_enabled', {enabled: !mcp.data?.enabled}); await mcp.refetch(); }
    catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  }

  async function openLink() {
    setError(''); setBusy(true);
    try {
      await desktopCommand('open_external_link', {url: link, usernameMode});
      setLink(''); void navigate('/');
    } catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  }
  async function setCloseBehavior(value: string) {
    if (busy) return;
    setBusy(true); setError(''); setFeedback('');
    try {
      const status = await desktopCommand<DesktopStatus>('desktop_set_close_to_tray', {enabled: value === 'tray'});
      cache.setQueryData(desktopStatusKey, status);
      setFeedback('窗口关闭行为已保存');
    } catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  }

  return <Dialog isOpen className="orbit-dialog settings-dialog" data-density={preferences.uiDensity} purpose="form" padding={0}
    width="calc(var(--orbit-unit) * 76)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" onOpenChange={open => { if (!open) void navigate('/'); }}>
    <VStack className="orbit-dialog-layout settings-layout" gap={0}>
      <DialogHeader className="orbit-dialog-header" title="设置" hasDivider onOpenChange={open => { if (!open) void navigate('/'); }} />
      <HStack className="settings-body" gap={0} vAlign="stretch">
        <VStack as="nav" className="settings-navigation" gap={0} aria-label="设置分类">
          <List density="compact" edge="flush">
            <ListItem label="外观" className="settings-navigation-item" startContent={<Palette aria-hidden />} isSelected={tab === 'appearance'} data-active={String(tab === 'appearance')} onClick={() => selectSection('appearance')} />
            <ListItem label="AI 与 MCP" className="settings-navigation-item" startContent={<Network aria-hidden />} isSelected={tab === 'mcp'} data-active={String(tab === 'mcp')} onClick={() => selectSection('mcp')} />
            <ListItem label="堡垒机联动" className="settings-navigation-item" startContent={<ShieldCheck aria-hidden />} isSelected={tab === 'bastion'} data-active={String(tab === 'bastion')} onClick={() => selectSection('bastion')} />
          </List>
        </VStack>
        <StackItem className="settings-content" size="fill" isScrollable aria-label={tab === 'appearance' ? '外观设置' : tab === 'mcp' ? 'AI 与 MCP 设置' : '堡垒机联动设置'}>
          <VStack gap={5}>
            {error && <Banner status="error" title="操作未完成" description={error} />}
            {tab === 'appearance' && <>
              <VStack gap={4}>
                <HStack className="settings-row" gap={4} vAlign="center">
                  <Text className="settings-label">界面主题</Text>
                  <HStack className="choice-group" role="group" aria-label="界面主题" gap={0}>
                    <Button label="浅色" icon={<Sun size="1em" />} variant="ghost" aria-pressed={theme.mode === 'light'} onClick={() => theme.setMode('light')} />
                    <Button label="深色" icon={<Moon size="1em" />} variant="ghost" aria-pressed={theme.mode === 'dark'} onClick={() => theme.setMode('dark')} />
                    <Button label="跟随系统" icon={<Monitor size="1em" />} variant="ghost" aria-pressed={theme.mode === 'system'} onClick={() => theme.setMode('system')} />
                  </HStack>
                </HStack>
                <HStack className="settings-row" gap={4} vAlign="center">
                  <Text className="settings-label">界面密度</Text>
                  <HStack className="choice-group" role="group" aria-label="界面密度" gap={0}>
                    <Button label="紧凑" variant="ghost" aria-pressed={preferences.uiDensity === 'compact'} onClick={() => preferences.setUiDensity('compact')} />
                    <Button label="舒适" variant="ghost" aria-pressed={preferences.uiDensity === 'comfortable'} onClick={() => preferences.setUiDensity('comfortable')} />
                  </HStack>
                </HStack>
                <HStack className="settings-row" gap={4} vAlign="start">
                  <Text className="settings-label">终端字号</Text>
                  <VStack className="settings-row-content" gap={2}>
                    <Selector label="终端字号" isLabelHidden value={String(preferences.fontSize)} options={Array.from({length: 14}, (_, index) => ({value: String(index + 11), label: `${index + 11}`}))}
                      onChange={value => preferences.setFontSize(Number(value))} width="calc(var(--orbit-unit) * 8)" />
                    <Text type="supporting" color="secondary">终端字号独立于界面密度</Text>
                    <Text className="settings-terminal-preview" style={{fontSize: preferences.fontSize}}>Aa Bb 0123456789 · 终端文字预览</Text>
                  </VStack>
                </HStack>
              </VStack>
              <VStack className="settings-section" gap={3}>
                <Text className="settings-section-title">窗口与后台运行</Text>
                {!isDesktop ? <Text color="secondary">系统托盘与后台运行功能在桌面端可用。</Text> : <>
                  {desktop.error && <Banner status="error" title="无法读取窗口设置" description={errorMessage(desktop.error)} endContent={<Button label="重试" size="sm" onClick={() => { void desktop.refetch(); }} />} />}
                  {desktop.data?.error && <Banner status="warning" title="系统托盘不可用" description={desktop.data.error} />}
                  <HStack className="settings-row" gap={4} vAlign="start">
                    <Text className="settings-label">关闭窗口时</Text>
                    <VStack className="settings-row-content" gap={2}>
                      <Selector label="关闭主窗口时" isLabelHidden width="100%" value={desktop.data?.trayAvailable && desktop.data.closeToTray ? 'tray' : 'exit'}
                        options={[{value: 'tray', label: '后台运行（隐藏到系统托盘）'}, {value: 'exit', label: '退出应用'}]}
                        isDisabled={desktop.isPending || Boolean(desktop.error) || !desktop.data?.trayAvailable || busy} onChange={value => { void setCloseBehavior(value); }} />
                      <Text type="supporting" color="secondary">后台运行时保留当前会话、传输任务和编辑草稿。点击系统托盘图标可唤起窗口。</Text>
                      <Text type="supporting" color="secondary">完全退出请从托盘或“文件”菜单选择“退出 OrbitShell”，有未保存文件时会先提示处理。</Text>
                      {feedback && <Text type="supporting" color="accent" role="status">{feedback}</Text>}
                    </VStack>
                  </HStack>
                </>}
              </VStack>
              <VStack className="settings-section" gap={3}>
                <Text className="settings-section-title">快捷键</Text>
                <List className="shortcut-list" density="compact" edge="flush" hasDividers>
                  {shortcuts.map(shortcut => <ListItem key={shortcut.label} label={shortcut.label} endContent={<Text className="shortcut-keys">{shortcut.keys}</Text>} />)}
                </List>
                <Text type="supporting" color="secondary">查找时按 Enter / Shift + Enter 切换匹配，Esc 关闭。鼠标中键可关闭会话标签。</Text>
                <Text type="supporting" color="secondary">终端有选区时 Ctrl + C 复制，无选区时中断命令。Ctrl + Shift + C / V 同样支持复制粘贴。</Text>
                <Text type="supporting" color="secondary">文件单击选中，双击或 Enter 打开目录。</Text>
              </VStack>
              {(theme.persistenceError || preferences.persistenceError) && <Banner status="warning" title="偏好未保存" description={theme.persistenceError ?? preferences.persistenceError ?? ''} />}
              <Text className="settings-autosave" color="secondary">更改后自动保存</Text>
            </>}
            {tab === 'mcp' && <>
              {!isDesktop ? <Banner status="info" title="请在桌面端开启 MCP" /> : <>
                {mcp.error && <Banner status="error" title="无法读取 MCP 设置" description={errorMessage(mcp.error)} endContent={<Button label="重试" size="sm" onClick={() => { void mcp.refetch(); }} />} />}
                <HStack hAlign="between" gap={3} vAlign="center">
                  <HStack gap={2} vAlign="center"><StatusDot label={mcp.data?.enabled ? '已开启' : '已关闭'} variant={mcp.data?.enabled ? 'success' : 'neutral'} />
                    <Text className="settings-section-title">{mcp.isPending ? '正在读取 MCP…' : mcp.data?.enabled ? 'MCP 已开启' : 'MCP 已关闭'}</Text></HStack>
                  <Button label={mcp.data?.enabled ? '关闭 MCP' : '开启 MCP'} variant={mcp.data?.enabled ? 'secondary' : 'primary'} isLoading={busy}
                    isDisabled={mcp.isPending || Boolean(mcp.error)} onClick={() => { void toggleMcp(); }} />
                </HStack>
                <CheckboxInput label="启动程序时自动开启 MCP" value={Boolean(mcp.data?.autoStart)} isDisabled={busy || mcp.isPending || Boolean(mcp.error)}
                  description="关闭时，每次启动后需要手动开启。此设置不改变当前服务状态。" onChange={value => {
                    setBusy(true); setError('');
                    void desktopCommand('mcp_set_auto_start', {autoStart: value}).then(() => mcp.refetch()).catch(failure => setError(errorMessage(failure))).finally(() => setBusy(false));
                  }} />
                {mcp.data?.startupError && <Banner status="warning" title="MCP 自动启动失败" description={mcp.data.startupError} />}
                <Text type="supporting" color="secondary">仅监听本机回环地址，启动方式保存在本机。</Text>
                {mcp.data?.enabled && <VStack gap={4}>
                  <TextInput label="MCP 地址" value={mcp.data.url ?? ''} isReadOnly width="100%" controlClassName="orbit-input" />
                  <TextInput label="访问令牌" value={mcp.data.token} type="password" isReadOnly autoComplete="off" width="100%" controlClassName="orbit-input" />
                  <HStack gap={3} vAlign="center" wrap="wrap">
                    <Button label="复制 MCP 客户端配置" variant="primary" icon={<Copy size="1em" />} onClick={() => {
                      if (!mcp.data?.enabled) return;
                      const config = {mcpServers: {OrbitShell: {type: 'http', url: mcp.data.url, headers: {Authorization: `Bearer ${mcp.data.token}`}}}};
                      void navigator.clipboard.writeText(JSON.stringify(config, null, 2)).then(() => setFeedback('已复制客户端配置')).catch(failure => setError(errorMessage(failure)));
                    }} />
                    {feedback && <Text color="accent" role="status">{feedback}</Text>}
                  </HStack>
                  <VStack className="settings-section" gap={3}>
                    <Text className="settings-section-title">服务端已注册工具 · {mcp.data.tools.length}</Text>
                    <Text type="supporting" color="secondary">会话控制协议：{mcp.data.protocolVersion}</Text>
                    <McpToolList tools={mcp.data.tools} />
                    <Text type="supporting" color="secondary">此清单来自服务端真实路由。若 Agent 宿主只显示旧工具，可用 send_input 的 action=takeover / release 控制原终端；控制操作必须使用空 data，不发送按键。</Text>
                  </VStack>
                  <Text type="supporting" color="secondary">首次连接服务器仍需确认主机指纹。</Text>
                </VStack>}
              </>}
            </>}
            {tab === 'bastion' && <>
              <TextInput label="会话链接" value={link} onChange={setLink} type="password" autoComplete="off" placeholder="粘贴 SSH 会话链接"
                description="认证信息仅用于本次会话。" width="100%" controlClassName="orbit-input" />
              <Selector label="堡垒机用户名处理" value={usernameMode} onChange={setUsernameMode} width="100%" options={[
                {value: 'preserve', label: '原样发送用户名（默认）'},
                {value: 'decodeBase64', label: '解码 b64>> 前缀用户名'},
              ]} />
              <HStack><Button label="打开会话链接" icon={<Link2 size="1em" />} variant="primary" isDisabled={!isDesktop || !link.trim()} isLoading={busy} onClick={() => { void openLink(); }} /></HStack>
              <VStack className="settings-section" gap={3}>
                <Text className="settings-section-title">SSH 链接入口</Text>
                <HStack><Button label="注册 SSH 链接入口" isDisabled={!isDesktop} onClick={() => {
                  setError(''); setFeedback('');
                  void desktopCommand<string>('register_ssh_protocol').then(setFeedback).catch(failure => setError(errorMessage(failure)));
                }} /></HStack>
                <Text type="supporting" color="secondary">注册后，在系统默认应用中选择 OrbitShell。</Text>
                {feedback && <Text type="supporting" color="accent" role="status">{feedback}</Text>}
              </VStack>
              <VStack className="settings-section" gap={3}>
                <Text className="settings-section-title">Xshell 启动参数</Text>
                <Text className="settings-command mono-text">{'.\\OrbitShell.exe -url <SSH 链接> -newtab <标签名>'}</Text>
                <Text type="supporting" color="secondary">项目附带 Xshell 参数转接器。运维助手的接入方式取决于其客户端路径设置。</Text>
              </VStack>
            </>}
          </VStack>
        </StackItem>
      </HStack>
    </VStack>
  </Dialog>;
}
