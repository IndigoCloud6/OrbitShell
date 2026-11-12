import {useEffect, useRef, useState} from 'react';
import type {FormEvent} from 'react';
import {useNavigate, useParams, useSearchParams} from 'react-router';
import {useQueryClient} from '@tanstack/react-query';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {TextArea} from '@astryxdesign/core/TextArea';
import {Selector} from '@astryxdesign/core/Selector';
import {RadioList, RadioListItem} from '@astryxdesign/core/RadioList';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {Banner} from '@astryxdesign/core/Banner';
import {List, ListItem} from '@astryxdesign/core/List';
import {Table, proportional} from '@astryxdesign/core/Table';
import type {TableColumn} from '@astryxdesign/core/Table';
import {DropdownMenu} from '@astryxdesign/core/DropdownMenu';
import {Folder, Layers, Plus, Play, Pencil, Copy, MoreHorizontal, Search, Settings2, Terminal, Trash2} from 'lucide-react';
import {usePreferences} from '../../app/preferences';
import {errorMessage, isDesktop} from '../../shared/desktop';
import {useSessions} from '../terminal/session-provider';
import {commandGroups, deleteQuickCommand, deleteQuickGroup, filterCommands, quickCommandKey, saveQuickCommand, saveQuickGroup, useQuickCommands} from './service';
import type {QuickCommand, QuickCommands} from './service';
import {useQuickCommandDraft} from './command-draft';

function Groups({collection, active, onSelect, onCreate, onRename, onRemove}: {
  collection: QuickCommands | undefined; active: string; onSelect: (group: string) => void;
  onCreate?: () => void; onRename?: (group: string) => void; onRemove?: (group: string) => void;
}) {
  return <VStack className="quick-groups" gap={0} as="nav" aria-label="快捷指令分组">
    <StackItem size="fill" isScrollable aria-label="指令分组列表"><List density="compact" edge="flush">
      <ListItem className="quick-group" label="全部指令" startContent={<Layers />} isSelected={!active} data-active={String(!active)}
        endContent={<Text className="count-label">{collection?.commands.length ?? 0}</Text>} onClick={() => onSelect('')} />
      {commandGroups(collection).map(group => <ListItem key={group} className="quick-group" label={<Text maxLines={1}>{group}</Text>}
        startContent={<Folder />} isSelected={group === active} data-active={String(group === active)} onClick={() => onSelect(group)}
        endContent={<HStack gap={1} vAlign="center"><Text className="count-label">{collection?.commands.filter(command => command.group === group).length ?? 0}</Text>
          {onRename && onRemove && group !== '未分组' && <DropdownMenu hasChevron={false} presentation="popover" alignment="end"
            button={{label: `${group}分组操作`, tooltip: '分组操作', icon: <MoreHorizontal />, isIconOnly: true, variant: 'ghost', size: 'sm', className: 'quick-icon-action'}}
            items={[{label: '重命名分组', onClick: () => onRename(group)}, {label: '删除分组', variant: 'destructive', onClick: () => onRemove(group)}]} />}
        </HStack>} />)}
    </List></StackItem>
    {onCreate && <Button className="quick-create-group" label="新建分组" icon={<Plus size="1em" />} variant="ghost" size="sm" onClick={onCreate} />}
  </VStack>;
}

function CommandRows({commands, manager = false, canSend = false, busy = false, onSend, onEdit, onCopy, onRemove}: {
  commands: QuickCommand[]; manager?: boolean; canSend?: boolean; busy?: boolean;
  onSend?: (command: QuickCommand) => void; onEdit: (command: QuickCommand) => void;
  onCopy?: (command: QuickCommand) => void; onRemove?: (command: QuickCommand) => void;
}) {
  // The data API supplies fixed table layout and real column floors; children mode
  // uses auto layout, which can collapse truncating cells to almost zero width.
  const rows = commands.map(command => ({...command}));
  const columns: TableColumn<(typeof rows)[number]>[] = [
    {key: 'name', header: '名称', width: proportional(1.2, {minWidth: 120}), renderCell: command => onSend ?
      <Button className="quick-send-name" variant="ghost" label={command.name} tooltip={`发送到当前终端：${command.command}`}
        isDisabled={!canSend || busy} size="sm" onClick={() => onSend(command)} /> : <Text maxLines={1} hasTruncateTooltip>{command.name}</Text>},
    {key: 'command', header: '指令内容', width: proportional(2.6, {minWidth: 160}), renderCell: command =>
      <Text className="mono-text quick-command-preview" maxLines={1} hasTruncateTooltip>{command.command}</Text>},
  ];
  if (manager) columns.push({key: 'group', header: '分组', width: proportional(.8, {minWidth: 80}), renderCell: command => <Text maxLines={1}>{command.group}</Text>});
  columns.push(
    {key: 'appendNewline', header: '发送方式', width: proportional(1.1, {minWidth: 120}), renderCell: command => <Text color="secondary">{command.appendNewline ? '发送并执行' : '仅发送，不回车'}</Text>},
    {key: 'actions', header: '操作', width: proportional(.8, {minWidth: 88}), renderCell: command => <HStack gap={0} vAlign="center">
      {onSend && <IconButton className="quick-icon-action" label={`发送 ${command.name}`} tooltip="发送到当前终端" icon={<Play />} variant="ghost" size="sm"
        isDisabled={!canSend || busy} onClick={() => onSend(command)} />}
      <IconButton className="quick-icon-action" label={`编辑 ${command.name}`} tooltip="编辑指令" icon={<Pencil />} variant="ghost" size="sm" isDisabled={busy} onClick={() => onEdit(command)} />
      {onCopy && <IconButton className="quick-icon-action" label={`复制 ${command.name}`} tooltip="创建指令副本" icon={<Copy />} variant="ghost" size="sm" isDisabled={busy} onClick={() => onCopy(command)} />}
      {onRemove && <DropdownMenu hasChevron={false} presentation="popover" alignment="end"
        button={{label: `${command.name}更多操作`, tooltip: '更多操作', icon: <MoreHorizontal />, isIconOnly: true, variant: 'ghost', size: 'sm', className: 'quick-icon-action', isDisabled: busy}}
        items={[{label: '删除指令', variant: 'destructive', onClick: () => onRemove(command)}]} />}
    </HStack>},
  );
  return <Table className="quick-table" data={rows} columns={columns} idKey="id" density="compact" dividers="rows" textOverflow="truncate" hasHover aria-label="快捷指令列表" />;
}

export function QuickCommandPanel({onLocateTerminal}: {onLocateTerminal?: (id: string) => void} = {}) {
  const query = useQuickCommands();
  const {sessions, activeId, getRuntime} = useSessions();
  const navigate = useNavigate();
  const [search, setSearch] = useState('');
  const [group, setGroup] = useState('');
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState('');
  const [error, setError] = useState('');
  const sending = useRef(false);
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;
  const active = sessions.find(session => session.id === activeId);
  const canSend = isDesktop && active?.status === 'connected';
  const activeGroup = commandGroups(query.data).includes(group) ? group : '';
  const commands = filterCommands(query.data?.commands ?? [], activeGroup, search);
  const send = async (command: QuickCommand) => {
    if (sending.current) return;
    const target = active;
    const runtime = target ? getRuntime(target.id) : undefined;
    setError(''); setFeedback('');
    if (!target || target.status !== 'connected' || !runtime) { setError('当前没有已连接终端，指令未发送'); return; }
    sending.current = true; setBusy(true);
    try {
      await runtime.sendCommand(command.command, command.appendNewline);
      setFeedback(`已发送到 ${target.profile.name}`);
      if (activeIdRef.current === target.id) { onLocateTerminal?.(target.id); requestAnimationFrame(() => runtime.terminal?.focus()); }
    } catch (failure) { setError(errorMessage(failure)); }
    finally { sending.current = false; setBusy(false); }
  };
  return <VStack className="quick-panel" height="100%" gap={0}>
    <HStack className="feature-toolbar quick-toolbar" vAlign="center" gap={2} wrap="wrap">
      <Terminal size="1em" /><Text className="quick-target" maxLines={1} hasTruncateTooltip>当前终端：{active ? `${active.profile.name} · ${active.profile.username}@${active.profile.hostname.includes(':') ? `[${active.profile.hostname}]` : active.profile.hostname}:${active.profile.port}` : '未连接'}</Text>
      <StackItem size="fill" /><TextInput label="搜索快捷指令" isLabelHidden type="search" value={search} onChange={setSearch} placeholder="搜索指令…" hasClear startIcon={<Search size="1em" />} controlClassName="feature-search-control" />
      <Button label="管理指令" size="sm" icon={<Settings2 size="1em" />} variant="ghost" href="/commands" />
      <Button label="新建" size="sm" icon={<Plus size="1em" />} href="/commands/new?return=workbench" />
    </HStack>
    {!canSend && <Text className="feature-note" color="secondary">{active?.status === 'connecting' || active?.status === 'hostKey' ? '正在认证，暂不可发送。连接成功后即可使用。' : '当前没有已连接终端，连接后即可发送快捷指令。'}</Text>}
    {feedback && <Banner className="compact-feedback" status="success" title={feedback} isDismissable onDismiss={() => setFeedback('')} />}
    {error && <Banner className="compact-feedback" status="error" title="指令未发送" description={error} isDismissable onDismiss={() => setError('')} />}
    {query.error && <Banner className="compact-feedback" status="error" title="无法读取快捷指令" description={errorMessage(query.error)} endContent={<Button label="重试" size="sm" onClick={() => { void query.refetch(); }} />} />}
    <HStack className="quick-content" gap={0} vAlign="stretch">
      <Groups collection={query.data} active={activeGroup} onSelect={setGroup} />
      <StackItem className="quick-list-region" size="fill" isScrollable aria-label="快捷指令">
        {commands.length > 0 && <CommandRows commands={commands} canSend={canSend} busy={busy} onSend={command => { void send(command); }} onEdit={command => { void navigate(`/commands/${command.id}/edit?return=workbench`); }} />}
        {!commands.length && !query.error && <VStack className="feature-empty" gap={2} hAlign="center" vAlign="center"><Terminal className="muted-icon" />
          <Text>{query.isPending ? '正在读取指令…' : query.data?.commands.length ? '没有匹配的指令' : '还没有快捷指令'}</Text>
          {!query.isPending && <Button label={query.data?.commands.length ? '清除筛选' : '新建快捷指令'} variant="ghost" size="sm" onClick={() => {
            if (query.data?.commands.length) { setSearch(''); setGroup(''); } else void navigate('/commands/new?return=workbench');
          }} />}
        </VStack>}
      </StackItem>
    </HStack>
  </VStack>;
}

export function QuickCommandManagerRoute() {
  const query = useQuickCommands();
  const cache = useQueryClient();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const {uiDensity} = usePreferences();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [removing, setRemoving] = useState<QuickCommand | null>(null);
  const [groupForm, setGroupForm] = useState<{previous: string | null; name: string} | null>(null);
  const [removingGroup, setRemovingGroup] = useState<string | null>(null);
  const groups = commandGroups(query.data);
  const group = params.get('group') ?? '';
  const active = groups.includes(group) ? group : '';
  const search = params.get('search') ?? '';
  const commands = filterCommands(query.data?.commands ?? [], active, search);
  const {sessions, activeId} = useSessions();
  const target = sessions.find(session => session.id === activeId);
  const setFilter = (key: string, value: string) => { const next = new URLSearchParams(params); if (value) next.set(key, value); else next.delete(key); void setParams(next, {replace: true}); };
  const mutate = async (action: () => Promise<unknown>, completed?: () => void) => {
    if (busy) return;
    setBusy(true); setError('');
    try { await action(); completed?.(); await cache.invalidateQueries({queryKey: quickCommandKey}); }
    catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  };
  return <>
    <Dialog isOpen className="orbit-dialog quick-manager" data-density={uiDensity} padding={0} width="calc(var(--orbit-unit) * 88)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)"
      onOpenChange={open => { if (!open && !busy) void navigate('/'); }}>
      <VStack className="orbit-dialog-layout quick-manager-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title={<>快捷指令管理<Text className="dialog-inline-count" color="secondary">{query.data?.commands.length ?? 0} 个指令</Text></>}
          hasDivider endContent={<Button label="新建指令" icon={<Plus size="1em" />} variant="primary" href={`/commands/new${active ? `?group=${encodeURIComponent(active)}` : ''}`} />}
          onOpenChange={open => { if (!open && !busy) void navigate('/'); }} />
        {error && !groupForm && !removing && !removingGroup && <Banner className="compact-feedback" status="error" title="操作未完成" description={error} />}
        <HStack className="quick-manager-body" gap={0} vAlign="stretch">
          <Groups collection={query.data} active={active} onSelect={value => setFilter('group', value)} onCreate={() => { setError(''); setGroupForm({previous: null, name: ''}); }}
            onRename={value => { setError(''); setGroupForm({previous: value, name: value}); }} onRemove={value => { setError(''); setRemovingGroup(value); }} />
          <VStack className="quick-manager-main" gap={0}>
            <VStack className="feature-toolbar" gap={0}><TextInput label="搜索快捷指令" isLabelHidden type="search" value={search} onChange={value => setFilter('search', value)}
              placeholder="搜索名称或指令内容…" hasClear startIcon={<Search size="1em" />} width="100%" /></VStack>
            {query.error && <Banner status="error" title="无法读取快捷指令" description={errorMessage(query.error)} endContent={<Button label="重试" onClick={() => { void query.refetch(); }} />} />}
            <StackItem className="quick-list-region" size="fill" isScrollable aria-label="已保存的快捷指令">
              {commands.length > 0 && <CommandRows commands={commands} manager busy={busy} onEdit={command => { void navigate(`/commands/${command.id}/edit`); }}
                onCopy={command => { void mutate(() => saveQuickCommand({...command, id: crypto.randomUUID(), name: `${Array.from(command.name).slice(0, 35).join('')}（副本）`})); }}
                onRemove={command => { setError(''); setRemoving(command); }} />}
              {!commands.length && !query.error && <VStack className="feature-empty" hAlign="center" vAlign="center" gap={3}><Terminal className="muted-icon" />
                <Text>{query.isPending ? '正在读取快捷指令…' : query.data?.commands.length ? '没有匹配的指令' : '还没有快捷指令'}</Text>
                {query.data?.commands.length ? <Button label="清除筛选" variant="ghost" onClick={() => { void setParams({}, {replace: true}); }} /> : !query.isPending && <Button label="新建指令" href="/commands/new" variant="primary" />}
              </VStack>}
            </StackItem>
          </VStack>
        </HStack>
        <HStack className="orbit-dialog-footer" hAlign="between" gap={3}><Text color="secondary">当前终端：{target?.profile.name ?? '未连接'}</Text><Text color="secondary">指令保存在本机</Text></HStack>
      </VStack>
    </Dialog>
    {groupForm && <Dialog isOpen className="orbit-dialog" purpose={busy ? 'required' : 'form'} padding={0} width="calc(var(--orbit-unit) * 36)" onOpenChange={open => { if (!open && !busy) setGroupForm(null); }}>
      <VStack className="orbit-dialog-layout" gap={0} as="form" onSubmit={(event: FormEvent) => { event.preventDefault(); void mutate(() => saveQuickGroup(groupForm.name, groupForm.previous), () => { setFilter('group', groupForm.name.trim()); setGroupForm(null); }); }}>
        <DialogHeader className="orbit-dialog-header" title={groupForm.previous ? '重命名分组' : '新建分组'} hasDivider />
        <VStack className="orbit-dialog-body" gap={3}>{error && <Banner status="error" title="分组未保存" description={error} />}
          <TextInput label="分组名称" value={groupForm.name} onChange={name => setGroupForm({...groupForm, name})} isRequired hasAutoFocus isDisabled={busy} />
        </VStack>
        <HStack className="orbit-dialog-footer" hAlign="end" gap={2}><Button label="取消" isDisabled={busy} onClick={() => setGroupForm(null)} /><Button label="保存分组" type="submit" variant="primary" isLoading={busy} /></HStack>
      </VStack>
    </Dialog>}
    {(removing || removingGroup) && <Dialog isOpen className="orbit-dialog" purpose={busy ? 'required' : 'form'} padding={0} width="calc(var(--orbit-unit) * 38)" onOpenChange={open => { if (!open && !busy) { setRemoving(null); setRemovingGroup(null); } }}>
      <VStack className="orbit-dialog-layout" gap={0}><DialogHeader className="orbit-dialog-header" title={removing ? '删除快捷指令' : '删除指令分组'} hasDivider />
        <VStack className="orbit-dialog-body" gap={3}><Text>{removing ? `确定删除“${removing.name}”？` : `确定删除分组“${removingGroup}”？其中的指令会移到“未分组”，不会删除。`}</Text>
          {error && <Banner status="error" title="删除未完成" description={error} />}</VStack>
        <HStack className="orbit-dialog-footer" hAlign="end" gap={2}><Button label="取消" isDisabled={busy} onClick={() => { setRemoving(null); setRemovingGroup(null); }} />
          <Button label="删除" variant="destructive" icon={<Trash2 size="1em" />} isLoading={busy} onClick={() => {
            void mutate(() => removing ? deleteQuickCommand(removing.id) : deleteQuickGroup(removingGroup!), () => { setRemoving(null); setRemovingGroup(null); });
          }} /></HStack>
      </VStack>
    </Dialog>}
  </>;
}

export function QuickCommandEditorRoute() {
  const {commandId} = useParams();
  const {peek, clear} = useQuickCommandDraft();
  const [draft] = useState(() => commandId ? null : peek());
  useEffect(() => { if (draft) clear(draft.id); }, [clear, draft]);
  const [newId] = useState(() => crypto.randomUUID());
  const [params] = useSearchParams();
  const query = useQuickCommands();
  const navigate = useNavigate();
  const back = params.get('return') === 'workbench' ? '/' : '/commands';
  const command = query.data?.commands.find(row => row.id === commandId);
  if (query.isPending || query.error || (commandId && !command)) return <Dialog isOpen className="orbit-dialog" onOpenChange={open => { if (!open) void navigate(back); }}>
    <DialogHeader title="快捷指令" /><Text>{query.isPending ? '正在读取指令…' : query.error ? errorMessage(query.error) : '这条指令已被删除'}</Text>
    {query.error && <Button label="重试" onClick={() => { void query.refetch(); }} />}<Button label="返回" href={back} />
  </Dialog>;
  return <QuickCommandEditor key={commandId ?? 'new'} initial={command ?? {id: newId, name: '', group: params.get('group') ?? '未分组', command: draft?.command ?? '', appendNewline: true}}
    groups={commandGroups(query.data)} editing={Boolean(commandId)} back={back} />;
}

function QuickCommandEditor({initial, groups, editing, back}: {initial: QuickCommand; groups: string[]; editing: boolean; back: string}) {
  const [form, setForm] = useState(initial);
  const [customGroup, setCustomGroup] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const cache = useQueryClient();
  const navigate = useNavigate();
  const {uiDensity} = usePreferences();
  const submit = async () => {
    if (busy) return;
    setBusy(true); setError('');
    try { await saveQuickCommand(form); await cache.invalidateQueries({queryKey: quickCommandKey}); void navigate(back); }
    catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  };
  return <Dialog isOpen className="orbit-dialog quick-editor" data-density={uiDensity} purpose={busy ? 'required' : 'form'} width="calc(var(--orbit-unit) * 48)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" padding={0}
    onOpenChange={open => { if (!open && !busy) void navigate(back); }}>
    <VStack className="orbit-dialog-layout" gap={0} as="form" onSubmit={(event: FormEvent) => { event.preventDefault(); void submit(); }}>
      <DialogHeader className="orbit-dialog-header" title={editing ? '编辑快捷指令' : '新建快捷指令'} hasDivider onOpenChange={open => { if (!open && !busy) void navigate(back); }} />
      <VStack className="orbit-dialog-body" gap={4}>
        {error && <Banner status="error" title="指令未保存" description={error} />}
        <TextInput label="名称" value={form.name} onChange={name => setForm({...form, name})} isRequired hasAutoFocus isDisabled={busy} placeholder="例如：查看磁盘" />
        <HStack gap={2} vAlign="end"><StackItem size="fill">{customGroup ? <TextInput label="新分组名称" value={form.group} onChange={group => setForm({...form, group})} width="100%" isDisabled={busy} hasAutoFocus /> :
          <Selector label="分组" value={form.group} options={[...new Set(['未分组', ...groups, form.group])]} onChange={group => setForm({...form, group})} width="100%" isDisabled={busy} />}</StackItem>
          <Button label={customGroup ? '选择已有分组' : '新建分组'} size="sm" variant="ghost" isDisabled={busy} onClick={() => { if (!customGroup) setForm({...form, group: ''}); else setForm({...form, group: '未分组'}); setCustomGroup(!customGroup); }} />
        </HStack>
        <TextArea label="指令内容" value={form.command} onChange={command => setForm({...form, command})} rows={6} isRequired isDisabled={busy}
          hasSpellCheck={false} autoComplete="off" controlClassName="quick-command-editor" placeholder="df -h" description="支持多行指令。请勿在指令中保存密码或访问令牌。" />
        <RadioList label="发送方式" value={form.appendNewline ? 'execute' : 'send'} onChange={value => setForm({...form, appendNewline: value === 'execute'})} isDisabled={busy} size="sm">
          <RadioListItem value="execute" label="发送并执行" description="发送指令后，在末尾追加一次回车。" />
          <RadioListItem value="send" label="仅发送，不回车" description="不追加末尾回车；指令中的多行换行仍会发送。" />
        </RadioList>
        <Text type="supporting" color="secondary">执行目标为点击时的当前终端，保存不会执行指令。</Text>
      </VStack>
      <HStack className="orbit-dialog-footer" hAlign="end" gap={2}><Button label="取消" isDisabled={busy} onClick={() => { void navigate(back); }} /><Button label="保存指令" type="submit" variant="primary" isLoading={busy} /></HStack>
    </VStack>
  </Dialog>;
}
