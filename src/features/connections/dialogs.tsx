import {useState} from 'react';
import type {FormEvent} from 'react';
import {useNavigate, useParams, useSearchParams} from 'react-router';
import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {VStack, HStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {CheckboxInput} from '@astryxdesign/core/CheckboxInput';
import {Banner} from '@astryxdesign/core/Banner';
import {List, ListItem} from '@astryxdesign/core/List';
import {Table, TableHeader, TableBody, TableRow, TableHeaderCell, TableCell} from '@astryxdesign/core/Table';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import {Folder, FolderOpen, KeyRound, Layers, LoaderCircle, LockKeyhole, MoreHorizontal, Pencil, Plug, Plus, Search, Server, Trash2} from 'lucide-react';
import {connectionKeys, deleteConnection, listConnections, saveConnection} from './service';
import {credentialStatusKey, forgetCredential, saveCredential, useCredentialStatus} from './credentials';
import type {ConnectionProfile} from './types';
import {desktopCommand, errorMessage, isDesktop} from '../../shared/desktop';
import {useSessions} from '../terminal/session-provider';
import {usePreferences} from '../../app/preferences';

export function useConnections() {
  return useQuery({queryKey: connectionKeys.all, queryFn: ({signal}) => listConnections(signal)});
}

export function ConnectionEditorRoute() {
  const {connectionId} = useParams();
  const query = useConnections();
  const navigate = useNavigate();
  const profile = query.data?.find(item => item.id === connectionId);
  if (connectionId && !profile) {
    return <Dialog isOpen className="orbit-dialog" onOpenChange={open => { if (!open) void navigate('/'); }}>
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
  const {uiDensity} = usePreferences();
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

  return <Dialog isOpen className="orbit-dialog connection-editor" data-density={uiDensity} purpose={busy ? 'required' : 'form'}
    width="calc(var(--orbit-unit) * 54)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" padding={0}
    onOpenChange={open => { if (!open && !busy) void navigate('/'); }}>
    <VStack className="orbit-dialog-layout connection-editor-layout" gap={0} as="form" onSubmit={(event: FormEvent) => { event.preventDefault(); void submit(false); }}>
      <DialogHeader className="orbit-dialog-header" title={profile ? '编辑连接' : '新建 SSH 连接'} hasDivider onOpenChange={open => { if (!open && !busy) void navigate('/'); }} />
      <VStack className="orbit-dialog-body" gap={4}>
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
      <VStack gap={2}>
        <Text className="field-label">认证方式</Text>
        <HStack className="choice-group connection-auth-choice" role="group" aria-label="认证方式" gap={0}>
          <Button label="密码" variant="ghost" isDisabled={busy} aria-pressed={form.authMethod === 'password'} onClick={() => {
            if (form.authMethod === 'password') return;
            update('authMethod', 'password'); setCredential(''); setRemember(false);
          }} />
          <Button label="SSH 私钥" variant="ghost" isDisabled={busy} aria-pressed={form.authMethod === 'privateKey'} onClick={() => {
            if (form.authMethod === 'privateKey') return;
            update('authMethod', 'privateKey'); setCredential(''); setRemember(false);
          }} />
        </HStack>
      </VStack>
      {form.authMethod === 'privateKey' && <VStack gap={2}>
        <HStack className="private-key-picker" gap={2} vAlign="end">
          <StackItem size="fill"><TextInput label="私钥文件" value={form.privateKeyPath ?? ''} onChange={value => update('privateKeyPath', value || null)}
            placeholder="OpenSSH 或 PEM 私钥路径" width="100%" isRequired isDisabled={busy} /></StackItem>
          <IconButton label="选择私钥文件" tooltip="选择私钥文件" icon={<FolderOpen size="1em" />} isDisabled={!isDesktop || busy} onClick={() => {
            void desktopCommand<string | null>('pick_private_key').then(path => { if (path) update('privateKeyPath', path); }).catch(failure => setError(errorMessage(failure)));
          }} />
        </HStack>
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
      </VStack>
      <HStack className="orbit-dialog-footer" gap={2} hAlign="end" wrap="wrap">
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
  if (!profile) return <Dialog isOpen className="orbit-dialog" onOpenChange={open => { if (!open) void navigate('/'); }}><DialogHeader title="连接服务器" />
    <Text>{query.isPending ? '正在读取连接…' : query.error ? errorMessage(query.error) : '找不到这个连接'}</Text><Button label="返回" href="/" /></Dialog>;
  return <CredentialsDialog key={profile.id} profile={profile} onClose={() => { void navigate('/'); }} />;
}

export function CredentialsDialog({profile, onClose, ephemeral = false}: {profile: ConnectionProfile; onClose: () => void; ephemeral?: boolean}) {
  const cache = useQueryClient();
  const {uiDensity} = usePreferences();
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
  const hostAddress = `${profile.hostname.includes(':') && !profile.hostname.startsWith('[') ? `[${profile.hostname}]` : profile.hostname}:${profile.port}`;
  const displayUsername = username.startsWith('b64>>') ? '堡垒机会话' : username;

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

  const credentialActions = !ephemeral && (savedCredential.data?.saved || savedCredential.error) ?
    <HStack className="connection-login-credential-actions" gap={1} vAlign="center">
      {savedCredential.data?.saved && <Button label={useSaved ? profile.authMethod === 'privateKey' ? '更换口令' : '更换密码' : '使用已保存凭据'}
        tooltip={useSaved ? `更换${credentialLabel}` : '使用已保存凭据'} size="sm" variant="ghost" isDisabled={busy} onClick={() => {
          if (useSaved) { setReplaceSaved(true); setRemember(true); }
          else { setCredential(''); setReplaceSaved(false); setRemember(false); }
      }} />}
      <DropdownMenu presentation="popover" hasChevron={false} alignment="end" menuWidth="max-content"
        renderTrigger={triggerProps => <IconButton {...triggerProps} className="connection-login-more" label="更多凭据操作" tooltip="更多凭据操作"
          icon={<MoreHorizontal size="1em" />} size="sm" variant="ghost" isDisabled={busy} />}
        items={[{label: '清除已保存凭据', icon: <Trash2 size="1em" />, variant: 'destructive', isDisabled: busy, onClick: () => { void clearSaved(); }}]} />
    </HStack> : null;

  return <Dialog isOpen className="orbit-dialog connection-login" data-density={uiDensity} purpose={busy ? 'required' : 'form'} padding={0}
    width="calc(var(--orbit-unit) * 41.6)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)"
    onOpenChange={open => { if (!open && !busy) onClose(); }}>
    <VStack className="connection-login-layout" gap={0} as="form" onSubmit={connect}>
      <DialogHeader className="connection-login-header" title="连接服务器" hasDivider onOpenChange={open => { if (!open && !busy) onClose(); }} />
      <VStack className="connection-login-body" gap={0}>
        <HStack className="connection-login-server" gap={3} vAlign="center">
          <HStack className="connection-login-server-mark" vAlign="center"><Server aria-hidden /></HStack>
          <VStack className="connection-login-target" gap={0.5}>
            <Text className="connection-login-name" maxLines={1}>{profile.name}</Text>
            <Text className="connection-login-address" color="secondary" maxLines={1}>
              {displayUsername ? `${displayUsername}@${hostAddress}` : hostAddress}
            </Text>
          </VStack>
          <Text className="connection-login-group" color="secondary" maxLines={1}>{profile.group}</Text>
        </HStack>
        {!profile.username && <VStack className="connection-login-username" gap={0}>
          <TextInput label="用户名" value={username} onChange={setUsername} isRequired isDisabled={busy} hasAutoFocus
            width="100%" controlClassName="connection-login-input" />
        </VStack>}
        <VStack className="connection-login-authentication" gap={2}>
          {error && <Banner status="error" title="暂时无法连接" description={error} />}
          {!isDesktop && <Banner status="info" title="当前为浏览器预览" description="请启动 OrbitShell 桌面端连接服务器。" />}
          {!ephemeral && savedCredential.error && <Banner status="warning" title="无法读取已保存凭据" description="可手动输入本次密码，或清除旧凭据后重试。"
            endContent={<Button label="重试" size="sm" isDisabled={busy} onClick={() => { void savedCredential.refetch(); }} />} />}
          {reading ? <HStack className="connection-login-reading" gap={2} vAlign="center" role="status">
            <LoaderCircle className="spinning" aria-hidden /><Text color="secondary">正在读取已保存凭据状态…</Text>
          </HStack> : useSaved ? <VStack className="connection-login-saved" gap={2}>
            <HStack className="connection-login-credential-strip" gap={2} vAlign="center">
              <KeyRound className="connection-login-credential-mark" aria-hidden />
              <Text className="connection-login-credential-title" maxLines={1}>使用已保存的{profile.authMethod === 'privateKey' ? '私钥口令' : '密码'}</Text>
              {credentialActions}
            </HStack>
            <Text className="connection-login-local-note" color="secondary">凭据仅保存在本机</Text>
          </VStack> : <VStack className="connection-login-manual" gap={2}>
            <TextInput label={credentialLabel} type="password" value={credential} onChange={setCredential}
              autoComplete="off" hasAutoFocus={Boolean(profile.username)} isDisabled={busy} isOptional={profile.authMethod === 'privateKey'}
              width="100%" controlClassName="connection-login-input"
              description={ephemeral ? '临时会话的认证信息仅用于本次连接。' : profile.authMethod === 'privateKey' ? '未加密的私钥可留空。' : '可选择记住密码，下次连接直接使用。'} />
            {!ephemeral && <CheckboxInput label={`记住${credentialLabel}`} size="sm" value={remember} onChange={setRemember} isDisabled={busy || !savedCredential.data?.available}
              description={savedCredential.data?.available ? '认证成功后安全保存在本机。' : '凭据保存目前支持 Windows 桌面端。'} />}
            {credentialActions}
          </VStack>}
        </VStack>
      </VStack>
      <HStack className="connection-login-footer" gap={2} hAlign="end">
        <Button label="取消" size="sm" onClick={onClose} isDisabled={busy} />
        <Button label="连接服务器" size="sm" icon={<Plug size="1em" />} type="submit" variant="primary" isDisabled={!isDesktop || reading || busy} />
      </HStack>
    </VStack>
  </Dialog>;
}

export function ConnectionManagerRoute() {
  const query = useConnections();
  const navigate = useNavigate();
  const cache = useQueryClient();
  const {uiDensity} = usePreferences();
  const [params, setParams] = useSearchParams();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [removing, setRemoving] = useState<ConnectionProfile | null>(null);
  const deletion = useMutation({mutationFn: deleteConnection, onSuccess: async (_result, id) => {
    await cache.invalidateQueries({queryKey: connectionKeys.all});
    cache.removeQueries({queryKey: credentialStatusKey(id)});
    setRemoving(null);
  }});

  if (removing) {
    const cancelRemoval = (open: boolean) => { if (!open && !deletion.isPending) setRemoving(null); };
    return <Dialog isOpen className="orbit-dialog connection-removal" data-density={uiDensity} purpose="required" padding={0}
      width="calc(var(--orbit-unit) * 44)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" onOpenChange={cancelRemoval}>
      <VStack className="orbit-dialog-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title="删除连接配置" hasDivider onOpenChange={cancelRemoval} />
        <VStack className="orbit-dialog-body" gap={4}>
        <Text>删除“{removing.name}”的本地连接配置及其已保存凭据？已经打开的会话仍可继续使用。</Text>
        {deletion.error && <Banner status="error" title="删除失败" description={errorMessage(deletion.error)} />}
        </VStack>
        <HStack className="orbit-dialog-footer" gap={2} hAlign="end">
          <Button label="取消" isDisabled={deletion.isPending} onClick={() => setRemoving(null)} />
          <Button label="删除配置" variant="destructive" isLoading={deletion.isPending} onClick={() => deletion.mutate(removing.id)} />
        </HStack>
      </VStack>
    </Dialog>;
  }

  const profiles = query.data ?? [];
  const groups = new Map<string, number>();
  for (const profile of profiles) groups.set(profile.group, (groups.get(profile.group) ?? 0) + 1);
  const requestedGroup = params.get('group');
  const activeGroup = requestedGroup && groups.has(requestedGroup) ? requestedGroup : null;
  const search = params.get('search') ?? '';
  const searchTerms = search.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const filtered = profiles.filter(profile => {
    if (activeGroup !== null && profile.group !== activeGroup) return false;
    const text = `${profile.name} ${profile.hostname} ${profile.port} ${profile.username}`.toLocaleLowerCase();
    return searchTerms.every(term => text.includes(term));
  });
  const selected = filtered.find(profile => profile.id === selectedId) ?? filtered[0];
  const setFilter = (name: 'search' | 'group', value: string) => {
    const next = new URLSearchParams(params);
    if (value) next.set(name, value); else next.delete(name);
    void setParams(next, {replace: true});
  };

  return <Dialog isOpen className="orbit-dialog connection-manager" data-density={uiDensity} padding={0}
    width="calc(var(--orbit-unit) * 96)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)"
    onOpenChange={open => { if (!open) void navigate('/'); }}>
    <VStack className="connection-manager-layout" gap={0}>
      <DialogHeader className="connection-manager-header" hasDivider
        title={<>管理连接{query.data && <Text className="connection-manager-count" color="secondary">{profiles.length} 个连接</Text>}</>}
        endContent={<Button className="connection-manager-create" label="新建连接" href="/connections/new" icon={<Plus size="1em" />} variant="primary" />}
        onOpenChange={open => { if (!open) void navigate('/'); }} />
      <HStack className="connection-manager-body" gap={0} vAlign="stretch">
        <VStack as="nav" className="connection-manager-sidebar" gap={0} aria-label="连接分组">
          <Text className="connection-manager-group-heading" color="secondary">连接分组</Text>
          <StackItem size="fill" isScrollable aria-label="连接分组列表">
            <List density="compact" edge="flush" className="connection-manager-groups">
              <ListItem className="connection-manager-group" label="全部连接" isSelected={activeGroup === null} data-active={String(activeGroup === null)}
                startContent={<Layers aria-hidden />} endContent={<Text className="connection-manager-group-count" hasTabularNumbers>{profiles.length}</Text>}
                onClick={() => setFilter('group', '')} />
              {[...groups].map(([group, count]) => <ListItem key={group} className="connection-manager-group"
                label={<Text maxLines={1}>{group}</Text>} isSelected={activeGroup === group} data-active={String(activeGroup === group)}
                startContent={<Folder aria-hidden />} endContent={<Text className="connection-manager-group-count" hasTabularNumbers>{count}</Text>}
                onClick={() => setFilter('group', group)} />)}
            </List>
          </StackItem>
        </VStack>
        <VStack className="connection-manager-main" gap={0}>
          <VStack className="connection-manager-search" gap={0}>
            <TextInput label="搜索连接" isLabelHidden type="search" value={search} onChange={value => setFilter('search', value)}
              placeholder="搜索名称、主机或用户名…" startIcon={<Search size="1em" />} hasClear hasSpellCheck={false}
              autoComplete="off" width="100%" controlClassName="connection-manager-search-control" />
          </VStack>
          {query.error && <VStack className="connection-manager-feedback" gap={0}>
            <Banner status="error" title="无法读取连接" description={errorMessage(query.error)}
              endContent={<Button label="重试" size="sm" isLoading={query.isFetching} onClick={() => { void query.refetch(); }} />} />
          </VStack>}
          {query.isPending && <VStack className="connection-manager-empty" gap={2} vAlign="center" hAlign="center" role="status"><Text color="secondary">正在读取连接…</Text></VStack>}
          {query.data && !filtered.length && <VStack className="connection-manager-empty" gap={3} vAlign="center" hAlign="center" role="status">
            <Server className="connection-manager-empty-icon" aria-hidden />
            <Text>{profiles.length ? '没有匹配的连接' : '还没有保存的连接'}</Text>
            {profiles.length > 0 && <Button label="清除筛选" variant="ghost" size="sm" onClick={() => {
              const next = new URLSearchParams(params);
              next.delete('search'); next.delete('group');
              void setParams(next, {replace: true});
            }} />}
          </VStack>}
          {filtered.length > 0 && <StackItem className="connection-manager-table-region" size="fill">
            <Table className="connection-manager-table" density="compact" dividers="rows" textOverflow="truncate" aria-label="已保存的连接">
              <TableHeader>
                <TableRow isHeaderRow>
                  <TableHeaderCell className="connection-manager-name-column" scope="col">连接名称</TableHeaderCell>
                  <TableHeaderCell className="connection-manager-host-column" scope="col">主机 / 端口</TableHeaderCell>
                  <TableHeaderCell className="connection-manager-user-column" scope="col">用户名</TableHeaderCell>
                  <TableHeaderCell className="connection-manager-group-column" scope="col">分组</TableHeaderCell>
                  <TableHeaderCell className="connection-manager-auth-column" scope="col">认证</TableHeaderCell>
                  <TableHeaderCell className="connection-manager-actions-column" scope="col">操作</TableHeaderCell>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filtered.map(profile => <TableRow key={profile.id} className="connection-manager-row" data-active={String(selected?.id === profile.id)}
                  onClick={() => setSelectedId(profile.id)} onFocusCapture={() => setSelectedId(profile.id)}>
                  <TableCell><HStack className="connection-manager-name" gap={2} vAlign="center">
                    <Server aria-hidden /><Text maxLines={1}>{profile.name}</Text>
                  </HStack></TableCell>
                  <TableCell><Text className="connection-manager-host" maxLines={1}>
                    {profile.hostname.includes(':') ? `[${profile.hostname}]:${profile.port}` : `${profile.hostname}:${profile.port}`}
                  </Text></TableCell>
                  <TableCell><Text maxLines={1}>{profile.username}</Text></TableCell>
                  <TableCell><Text maxLines={1}>{profile.group}</Text></TableCell>
                  <TableCell><HStack className="connection-manager-auth" gap={2} vAlign="center">
                    {profile.authMethod === 'privateKey' ? <KeyRound aria-hidden /> : <LockKeyhole aria-hidden />}
                    <Text>{profile.authMethod === 'privateKey' ? '私钥' : '密码'}</Text>
                  </HStack></TableCell>
                  <TableCell><HStack className="connection-manager-actions" gap={1} vAlign="center">
                    <Button className="connection-manager-connect" label="连接" aria-label={`连接 ${profile.name}`} size="sm" variant="ghost" href={`/connections/${profile.id}/connect`} />
                    <IconButton className="connection-manager-action" label={`编辑 ${profile.name}`} tooltip="编辑连接" icon={<Pencil size="1em" />} size="sm" variant="ghost" href={`/connections/${profile.id}/edit`} />
                    <DropdownMenu presentation="popover" hasChevron={false} alignment="end" menuWidth="max-content"
                      renderTrigger={triggerProps => <IconButton {...triggerProps} className="connection-manager-action" label={`${profile.name} 的更多操作`} tooltip="更多操作" icon={<MoreHorizontal size="1em" />} size="sm" variant="ghost" />}
                      items={[{label: '删除连接', icon: <Trash2 size="1em" />, variant: 'destructive', onClick: () => { deletion.reset(); setRemoving(profile); }}]} />
                  </HStack></TableCell>
                </TableRow>)}
              </TableBody>
            </Table>
          </StackItem>}
        </VStack>
      </HStack>
    </VStack>
  </Dialog>;
}
