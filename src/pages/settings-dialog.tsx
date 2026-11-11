import {useState} from 'react';
import {useNavigate, useSearchParams} from 'react-router';
import {useQuery} from '@tanstack/react-query';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {VStack, HStack} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Selector} from '@astryxdesign/core/Selector';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Banner} from '@astryxdesign/core/Banner';
import {TabList, Tab} from '@astryxdesign/core/TabList';
import {usePreferences} from '../app/preferences';
import {useThemePreference} from '../app/theme';
import {desktopCommand, errorMessage, isDesktop} from '../shared/desktop';

interface McpStatus {enabled: boolean; url: string | null; token: string; tools: string[]}

export default function SettingsDialog() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const [tab, setTab] = useState(params.get('section') === 'bastion' ? 'bastion' : params.get('section') === 'mcp' ? 'mcp' : 'appearance');
  const theme = useThemePreference();
  const preferences = usePreferences();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState('');
  const mcp = useQuery({queryKey: ['mcp-status'], queryFn: () => desktopCommand<McpStatus>('mcp_status'), enabled: isDesktop, retry: false});
  const [link, setLink] = useState('');
  const [usernameMode, setUsernameMode] = useState('preserve');

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

  return <Dialog isOpen purpose="form" width="40rem" onOpenChange={open => { if (!open) void navigate('/'); }}>
    <DialogHeader title="设置" subtitle="OrbitShell" onOpenChange={open => { if (!open) void navigate('/'); }} />
    <VStack gap={5}>
      <TabList value={tab} onChange={value => { setTab(value); setError(''); setFeedback(''); }} hasDivider>
        <Tab value="appearance" label="外观" /><Tab value="mcp" label="AI 与 MCP" /><Tab value="bastion" label="堡垒机联动" />
      </TabList>
      {error && <Banner status="error" title="操作未完成" description={error} />}
      {tab === 'appearance' && <VStack gap={4}>
        <Selector label="界面密度" value={preferences.uiDensity} options={[{value: 'compact', label: '紧凑（推荐）'}, {value: 'comfortable', label: '舒适'}]}
          onChange={value => preferences.setUiDensity(value === 'comfortable' ? 'comfortable' : 'compact')} width="100%" />
        <Text type="supporting">界面密度调整菜单、面板和列表大小。终端文字使用下面的独立字号。</Text>
        <Selector label="界面主题" options={[{value: 'dark', label: '深色'}, {value: 'light', label: '浅色'}, {value: 'system', label: '跟随系统'}]}
          value={theme.mode} onChange={value => theme.setMode(value === 'light' ? 'light' : value === 'system' ? 'system' : 'dark')} width="100%" />
        <Selector label="终端字号" value={String(preferences.fontSize)} options={Array.from({length: 14}, (_, index) => ({value: String(index + 11), label: `${index + 11}`}))}
          onChange={value => preferences.setFontSize(Number(value))} width="100%" />
        <VStack gap={1}>
          <Text type="supporting">Ctrl + Shift + T 新建连接 · Ctrl + Shift + W 关闭当前会话</Text>
          <Text type="supporting">Ctrl + F 查找终端 · Enter / Shift + Enter 下一个 / 上一个 · Esc 关闭查找</Text>
          <Text type="supporting">Ctrl + 加号 / 减号，或 Ctrl + 滚轮调节字号 · Ctrl + 0 恢复默认</Text>
          <Text type="supporting">Ctrl + Shift + C / V 复制、粘贴 · 鼠标中键关闭会话标签</Text>
          <Text type="supporting">文件列表单击选中，双击或 Enter 打开目录。列表密度、隐藏文件和侧栏偏好会自动保存。</Text>
        </VStack>
        {(theme.persistenceError || preferences.persistenceError) && <Banner status="warning" title="偏好未保存" description={theme.persistenceError ?? preferences.persistenceError ?? ''} />}
      </VStack>}
      {tab === 'mcp' && <VStack gap={4}>
        <Text>让 AI 查看已保存的连接、打开会话、读取终端并执行命令。</Text>
        {!isDesktop ? <Banner status="info" title="请在桌面端开启 MCP" /> : <>
          {mcp.error && <Banner status="error" title="无法读取 MCP 设置" description={errorMessage(mcp.error)} />}
          <HStack hAlign="between" vAlign="center"><Text type="label">{mcp.data?.enabled ? 'MCP 已开启' : 'MCP 已关闭'}</Text>
            <Button label={mcp.data?.enabled ? '关闭 MCP' : '开启 MCP'} variant={mcp.data?.enabled ? 'secondary' : 'primary'} isLoading={busy} onClick={() => { void toggleMcp(); }} /></HStack>
          <Text type="supporting">服务仅监听本机，客户端使用专属令牌连接。首次连接服务器时仍需确认主机指纹。</Text>
          {mcp.data?.enabled && <>
            <TextInput label="MCP 地址" value={mcp.data.url ?? ''} isReadOnly />
            <TextInput label="访问令牌" value={mcp.data.token} type="password" isReadOnly autoComplete="off" />
            <Button label="复制 MCP 客户端配置" onClick={() => {
              const config = {mcpServers: {OrbitShell: {type: 'http', url: mcp.data!.url, headers: {Authorization: `Bearer ${mcp.data!.token}`}}}};
              void navigator.clipboard.writeText(JSON.stringify(config, null, 2)).then(() => setFeedback('已复制客户端配置')).catch(failure => setError(errorMessage(failure)));
            }} />
          </>}
          {feedback && <Text color="accent" role="status">{feedback}</Text>}
        </>}
      </VStack>}
      {tab === 'bastion' && <VStack gap={4}>
        <Text>支持 SSH 会话链接，以及 Xshell 风格的 -url、-newwin、-newtab 启动参数。</Text>
        <TextInput label="会话链接" value={link} onChange={setLink} type="password" autoComplete="off" placeholder="ssh://用户名:密码@堡垒机地址:端口" description="包含认证信息的链接仅用于本次会话。" />
        <Selector label="堡垒机用户名处理" value={usernameMode} onChange={setUsernameMode} width="100%" options={[
          {value: 'preserve', label: '原样发送用户名（默认）'},
          {value: 'decodeBase64', label: '解码 b64>> 前缀用户名'},
        ]} description="不同堡垒机对编码用户名的处理方式可能不同。默认保留完整用户名。" />
        <Button label="打开会话链接" variant="primary" isDisabled={!isDesktop || !link.trim()} isLoading={busy} onClick={() => { void openLink(); }} />
        <Button label="注册 SSH 链接入口" isDisabled={!isDesktop} onClick={() => {
          void desktopCommand<string>('register_ssh_protocol').then(setFeedback).catch(failure => setError(errorMessage(failure)));
        }} />
        {feedback && <Text type="supporting" role="status">{feedback}</Text>}
        <Text type="supporting">阿里云运维助手能否直接选择 OrbitShell，取决于助手的客户端路径设置。项目附带 Xshell 参数转接器源码。注册 SSH 入口后，可在系统默认应用中选择 OrbitShell。</Text>
      </VStack>}
    </VStack>
  </Dialog>;
}
