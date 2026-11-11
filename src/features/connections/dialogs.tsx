import {useState} from 'react';
import type {FormEvent} from 'react';
import {useNavigate, useParams} from 'react-router';
import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {VStack, HStack} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {CheckboxInput} from '@astryxdesign/core/CheckboxInput';
import {Selector} from '@astryxdesign/core/Selector';
import {Banner} from '@astryxdesign/core/Banner';
import {List, ListItem} from '@astryxdesign/core/List';
import {FolderOpen, Pencil, Plug, Plus, Server, Trash2} from 'lucide-react';
import {connectionKeys, deleteConnection, listConnections, saveConnection} from './service';
import {credentialStatusKey, forgetCredential, saveCredential, useCredentialStatus} from './credentials';
import type {ConnectionProfile} from './types';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {useSessions} from '../terminal/session-provider';

export function useConnections() {
  return useQuery({queryKey: connectionKeys.all, queryFn: ({signal}) => listConnections(signal)});
}

export function ConnectionEditorRoute() {
  const {connectionId} = useParams();
  const query = useConnections();
  const navigate = useNavigate();
  const profile = query.data?.find(item => item.id === connectionId);
  if (connectionId && !profile) {
    return <Dialog isOpen onOpenChange={open => { if (!open) void navigate('/'); }}>
      <DialogHeader title="编辑连接" />
      <Text>{query.isPending ? '正在读取连接…' : query.error ? errorMessage(query.error) : '连接配置不存在'}</Text>
      <Button label="返回工作台" href="/" />
    </Dialog>;
  }
  return <ConnectionEditor key={connectionId ?? 'new'} profile={profile ?? null} />;
}

function ConnectionEditor({profile}: {profile: ConnectionProfile | null}) {
  const navigate = useNavigate();
  const cache = useQueryClient();
  const {start} = useSessions();
  const [form, setForm] = useState<ConnectionProfile>(() => profile ?? {
    id: crypto.randomUUID(), name: '', group: '未分组', hostname: '', port: 22, username: 'root', authMethod: 'password', privateKeyPath: null,
  });
  const [port, setPort] = useState(String(form.port));
  const [credential, setCredential] = useState('');
  const [remember, setRemember] = useState(false);
  const [error, setError] = useState('');
  const [feedback, setFeedback] = useState('');
  const [busy, setBusy] = useState(false);
  const savedCredential = useCredentialStatus(profile?.id ?? null);
  const save = useMutation({mutationFn: saveConnection});
  const update = <K extends keyof ConnectionProfile>(field: K, value: ConnectionProfile[K]) => setForm(previous => ({...previous, [field]: value}));
  const credentialLabel = form.authMethod === 'privateKey' ? '私钥口令' : '登录密码';
  const identityChanged = Boolean(profile && (profile.hostname !== form.hostname.trim() || profile.port !== Number(port) ||
    profile.username !== form.username.trim() || profile.authMethod !== form.authMethod ||
    (profile.privateKeyPath ?? '') !== (form.authMethod === 'privateKey' ? form.privateKeyPath ?? '' : '')));

  async function clearSaved() {
    if (!profile || busy) return;
    setBusy(true); setError(''); setFeedback('');
    try {
      await forgetCredential(profile.id);
      cache.setQueryData(credentialStatusKey(profile.id), {available: Boolean(savedCredential.data?.available), saved: false});
      setRemember(false);
      setFeedback('已清除这条连接保存的密码或口令。');
      await savedCredential.refetch();
    } catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  }

  async function submit(connectAfterSave: boolean) {
    if (busy) return;
    setBusy(true); setError(''); setFeedback('');
    try {
      if (!/^\d+$/.test(port)) throw new Error('请填写有效的端口号');
      const saved = await save.mutateAsync({...form, port: Number(port)});
      await cache.invalidateQueries({queryKey: connectionKeys.all});
      await cache.invalidateQueries({queryKey: credentialStatusKey(saved.id)});
      if (connectAfterSave && credential) {
        start(saved, credential, false, {rememberCredential: remember && Boolean(savedCredential.data?.available)});
        setCredential('');
        void navigate('/');
      } else {
        if (remember && credential && savedCredential.data?.available) {
          try { await saveCredential(saved.id, credential); }
          catch (failure) { throw new Error(`连接配置已保存，但密码未保存：${errorMessage(failure)}`); }
          await cache.invalidateQueries({queryKey: credentialStatusKey(saved.id)});
        }
        setCredential('');
        void navigate(connectAfterSave ? `/connections/${saved.id}/connect` : '/');
      }
    } catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  }

  return <Dialog isOpen purpose={busy ? 'required' : 'form'} width="36rem" onOpenChange={open => { if (!open && !busy) void navigate('/'); }}>
    <DialogHeader title={profile ? '编辑连接' : '新建 SSH 连接'} subtitle="保存服务器信息，可选择记住本机登录凭据。" onOpenChange={open => { if (!open && !busy) void navigate('/'); }} />
    <VStack gap={4} as="form" onSubmit={(event: FormEvent) => { event.preventDefault(); void submit(false); }}>
      {error && <Banner status="error" title="保存未完成" description={error} />}
      {feedback && <Text type="supporting" color="accent" role="status">{feedback}</Text>}
      <TextInput label="连接名称" value={form.name} onChange={value => update('name', value)} placeholder="例如 web-prod-01" isRequired hasAutoFocus isDisabled={busy} />
      <HStack gap={3} vAlign="start">
        <TextInput label="服务器地址" value={form.hostname} onChange={value => update('hostname', value)} placeholder="IP 地址或域名" width="100%" isRequired isDisabled={busy} />
        <TextInput label="端口" value={port} onChange={setPort} inputMode="numeric" width="7rem" isRequired isDisabled={busy} />
      </HStack>
      <HStack gap={3}>
        <TextInput label="用户名" value={form.username} onChange={value => update('username', value)} width="100%" isRequired isDisabled={busy} />
        <TextInput label="分组" value={form.group} onChange={value => update('group', value)} placeholder="未分组" width="100%" isDisabled={busy} />
      </HStack>
      <Selector label="认证方式" value={form.authMethod} options={[{value: 'password', label: '密码'}, {value: 'privateKey', label: 'SSH 私钥'}]}
        onChange={value => { update('authMethod', value === 'privateKey' ? 'privateKey' : 'password'); setCredential(''); setRemember(false); }} width="100%" isDisabled={busy} />
      {form.authMethod === 'privateKey' && <VStack gap={2}>
        <TextInput label="私钥文件" value={form.privateKeyPath ?? ''} onChange={value => update('privateKeyPath', value || null)} placeholder="OpenSSH 或 PEM 私钥路径" isRequired isDisabled={busy} />
        <Button label="选择私钥文件" size="sm" icon={<FolderOpen size="1em" />} isDisabled={!isDesktop || busy} onClick={() => {
          void desktopCommand<string | null>('pick_private_key').then(path => { if (path) update('privateKeyPath', path); }).catch(failure => setError(errorMessage(failure)));
        }} />
      </VStack>}
      <VStack gap={2}>
        <TextInput label={credentialLabel} type="password" value={credential} onChange={setCredential} autoComplete="new-password" isOptional isDisabled={busy}
          placeholder={savedCredential.data?.saved && !identityChanged ? '已保存，留空保持现有凭据' : form.authMethod === 'privateKey' ? '未加密的私钥可留空' : '可在连接时再输入'}
          description={savedCredential.data?.saved && identityChanged ? '连接目标已更改，保存时将清除原凭据。' : '未勾选记住时，输入内容仅用于“保存并连接”。'} />
        <CheckboxInput label={`记住${credentialLabel}`} size="sm" value={remember} onChange={setRemember} isDisabled={busy || !savedCredential.data?.available}
          description={savedCredential.data?.available ? '安全保存在本机，下次连接无需重新输入。' : isDesktop && savedCredential.isPending ? '正在读取凭据存储状态…' : '凭据保存目前支持 Windows 桌面端。'} />
        {savedCredential.error && <Banner status="warning" title="无法读取已保存凭据" description="可重试，或手动输入本次登录凭据。"
          endContent={<Button label="重试" size="sm" isDisabled={busy} onClick={() => { void savedCredential.refetch(); }} />} />}
        {profile && (savedCredential.data?.saved || savedCredential.error) && <HStack hAlign="between" vAlign="center" gap={2}>
          <Text type="supporting" color="secondary">{savedCredential.data?.saved ? '这条连接已有保存的凭据' : '可清除旧凭据后重新保存'}</Text>
          <Button label="清除已保存凭据" size="sm" variant="ghost" isDisabled={busy} onClick={() => { void clearSaved(); }} />
        </HStack>}
      </VStack>
      <HStack gap={2} hAlign="end" paddingBlockStart={2} wrap="wrap">
        <Button label="取消" onClick={() => { void navigate('/'); }} isDisabled={busy} />
        <Button label="保存连接" type="submit" isLoading={busy} />
        <Button label="保存并连接" variant="primary" onClick={() => { void submit(true); }} isLoading={busy} />
      </HStack>
    </VStack>
  </Dialog>;
}

export function CredentialsRoute() {
  const {connectionId} = useParams();
  const query = useConnections();
  const navigate = useNavigate();
  const profile = query.data?.find(item => item.id === connectionId);
  if (!profile) return <Dialog isOpen onOpenChange={open => { if (!open) void navigate('/'); }}><DialogHeader title="打开连接" />
    <Text>{query.isPending ? '正在读取连接…' : query.error ? errorMessage(query.error) : '找不到这个连接'}</Text><Button label="返回" href="/" /></Dialog>;
  return <CredentialsDialog key={profile.id} profile={profile} onClose={() => { void navigate('/'); }} />;
}

export function CredentialsDialog({profile, onClose, ephemeral = false}: {profile: ConnectionProfile; onClose: () => void; ephemeral?: boolean}) {
  const cache = useQueryClient();
  const savedCredential = useCredentialStatus(profile.id, !ephemeral);
  const [credential, setCredential] = useState('');
  const [remember, setRemember] = useState(false);
  const [replaceSaved, setReplaceSaved] = useState(false);
  const [username, setUsername] = useState(profile.username);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const {start} = useSessions();
  const credentialLabel = profile.authMethod === 'privateKey' ? '私钥口令' : '登录密码';
  const useSaved = !ephemeral && Boolean(savedCredential.data?.saved) && !replaceSaved && !credential;
  const reading = isDesktop && !ephemeral && savedCredential.isPending;

  async function clearSaved() {
    if (busy) return;
    setBusy(true); setError('');
    try {
      await forgetCredential(profile.id);
      cache.setQueryData(credentialStatusKey(profile.id), {available: Boolean(savedCredential.data?.available), saved: false});
      setReplaceSaved(true); setRemember(false);
      await savedCredential.refetch();
    } catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  }
  const connect = (event: FormEvent) => {
    event.preventDefault();
    if (busy || reading) return;
    try {
      start({...profile, username}, useSaved ? '' : credential, ephemeral, {
        useSavedCredential: useSaved,
        rememberCredential: !ephemeral && !useSaved && remember && Boolean(savedCredential.data?.available) && Boolean(credential),
      });
      setCredential(''); onClose();
    } catch (failure) { setError(errorMessage(failure)); }
  };

  return <Dialog isOpen purpose={busy ? 'required' : 'form'} width="30rem" onOpenChange={open => { if (!open && !busy) onClose(); }}>
    <DialogHeader title={`连接 ${profile.name}`} subtitle={`${profile.hostname}:${profile.port}`} onOpenChange={open => { if (!open && !busy) onClose(); }} />
    <VStack gap={4} as="form" onSubmit={connect}>
      {error && <Banner status="error" title="暂时无法连接" description={error} />}
      {!isDesktop && <Banner status="info" title="当前为浏览器预览" description="请启动 OrbitShell 桌面端连接服务器。" />}
      {!profile.username && <TextInput label="用户名" value={username} onChange={setUsername} isRequired isDisabled={busy} />}
      {reading && <Text type="supporting" color="secondary" role="status">正在读取已保存凭据状态…</Text>}
      {!ephemeral && savedCredential.error && <Banner status="warning" title="无法读取已保存凭据" description="可手动输入本次密码，或清除旧凭据后重试。"
        endContent={<Button label="重试" size="sm" isDisabled={busy} onClick={() => { void savedCredential.refetch(); }} />} />}
      {useSaved ? <Banner status="info" title={`已保存${credentialLabel}`} description="连接时将使用本机保存的凭据，无需再次输入。" /> : <VStack gap={2}>
        <TextInput label={credentialLabel} type="password" value={credential} onChange={setCredential}
          autoComplete="off" hasAutoFocus isDisabled={busy} isOptional={profile.authMethod === 'privateKey'}
          description={ephemeral ? '临时会话的认证信息仅用于本次连接。' : profile.authMethod === 'privateKey' ? '未加密的私钥可留空。' : '可选择记住密码，下次连接直接使用。'} />
        {!ephemeral && <CheckboxInput label={`记住${credentialLabel}`} size="sm" value={remember} onChange={setRemember} isDisabled={busy || !savedCredential.data?.available}
          description={savedCredential.data?.available ? '认证成功后安全保存在本机。' : reading ? '正在读取凭据存储状态…' : '凭据保存目前支持 Windows 桌面端。'} />}
      </VStack>}
      {!ephemeral && (savedCredential.data?.saved || savedCredential.error) && <HStack gap={2} wrap="wrap">
        {savedCredential.data?.saved && <Button label={useSaved ? `更换${credentialLabel}` : '使用已保存凭据'} size="sm" variant="ghost" isDisabled={busy} onClick={() => {
          if (useSaved) { setReplaceSaved(true); setRemember(true); }
          else { setCredential(''); setReplaceSaved(false); setRemember(false); }
        }} />}
        <Button label="清除已保存凭据" size="sm" variant="ghost" isDisabled={busy} onClick={() => { void clearSaved(); }} />
      </HStack>}
      <HStack gap={2} hAlign="end"><Button label="取消" onClick={onClose} isDisabled={busy} /><Button label="连接服务器" icon={<Plug size="1em" />} type="submit" variant="primary" isDisabled={!isDesktop || reading || busy} /></HStack>
    </VStack>
  </Dialog>;
}

export function ConnectionManagerRoute() {
  const query = useConnections();
  const navigate = useNavigate();
  const cache = useQueryClient();
  const [removing, setRemoving] = useState<ConnectionProfile | null>(null);
  const deletion = useMutation({mutationFn: deleteConnection, onSuccess: async (_result, id) => {
    await cache.invalidateQueries({queryKey: connectionKeys.all});
    cache.removeQueries({queryKey: credentialStatusKey(id)});
    setRemoving(null);
  }});
  return <Dialog isOpen purpose={removing ? 'required' : 'info'} width="46rem" onOpenChange={open => { if (!open) void navigate('/'); }}>
    <DialogHeader title={removing ? '删除连接配置' : '管理连接'} onOpenChange={open => { if (!open) void navigate('/'); }} />
    <VStack gap={4}>
      {removing ? <>
        <Text>删除“{removing.name}”的本地连接配置及其已保存凭据？已经打开的会话仍可继续使用。</Text>
        {deletion.error && <Banner status="error" title="删除失败" description={errorMessage(deletion.error)} />}
        <HStack gap={2} hAlign="end"><Button label="取消" onClick={() => setRemoving(null)} /><Button label="删除配置" variant="destructive" isLoading={deletion.isPending} onClick={() => deletion.mutate(removing.id)} /></HStack>
      </> : <>
        <HStack hAlign="between"><Text color="secondary">{query.data?.length ?? 0} 个已保存的连接</Text><Button label="新建连接" href="/connections/new" icon={<Plus size="1em" />} /></HStack>
        {query.isPending && <Text>正在读取连接…</Text>}
        {query.error && <Banner status="error" title="无法读取连接" description={errorMessage(query.error)} endContent={<Button label="重试" onClick={() => { void query.refetch(); }} />} />}
        {query.data?.length === 0 && <Text color="secondary">还没有连接。创建第一个 SSH 连接开始使用。</Text>}
        <List density="compact" hasDividers>
          {query.data?.map(profile => <ListItem key={profile.id} label={profile.name} description={`${profile.group} · ${profile.hostname}:${profile.port}`} startContent={<Server size="1em" />}
            endContent={<HStack gap={1}>
              <IconButton label={`连接 ${profile.name}`} icon={<Plug size="1em" />} variant="ghost" href={`/connections/${profile.id}/connect`} />
              <IconButton label={`编辑 ${profile.name}`} icon={<Pencil size="1em" />} variant="ghost" href={`/connections/${profile.id}/edit`} />
              <IconButton label={`删除 ${profile.name}`} icon={<Trash2 size="1em" />} variant="ghost" onClick={() => { deletion.reset(); setRemoving(profile); }} />
            </HStack>} />)}
        </List>
      </>}
    </VStack>
  </Dialog>;
}
