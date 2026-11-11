import {useMemo, useState} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {TextInput} from '@astryxdesign/core/TextInput';
import {List, ListItem} from '@astryxdesign/core/List';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Banner} from '@astryxdesign/core/Banner';
import {ChevronDown, ChevronRight, Plus, Search, Server} from 'lucide-react';
import type {ConnectionProfile} from './types';
import type {ShellSession} from '../terminal/runtime';

interface Props {
  profiles: ConnectionProfile[]; sessions: ShellSession[]; activeId: string | null;
  search: string; onSearch: (value: string) => void; onOpen: (profile: ConnectionProfile) => void;
  onCreate: () => void; onManage: () => void; pending?: boolean; error?: string; onRetry?: () => void;
}

export function ConnectionExplorer(props: Props) {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const groups = useMemo(() => {
    const result = new Map<string, ConnectionProfile[]>();
    for (const profile of props.profiles) {
      if (props.search && !`${profile.name} ${profile.hostname} ${profile.group}`.toLowerCase().includes(props.search.toLowerCase())) continue;
      const profiles = result.get(profile.group) ?? [];
      profiles.push(profile); result.set(profile.group, profiles);
    }
    return [...result];
  }, [props.profiles, props.search]);
  const active = props.sessions.find(session => session.id === props.activeId);
  return <VStack height="100%" gap={0} className="connection-explorer">
    <HStack className="panel-heading connection-heading" hAlign="between" vAlign="center">
      <HStack gap={2} vAlign="center"><Text className="panel-title">连接</Text><Text className="count-label">{props.profiles.length}</Text></HStack>
      <Button label="新建" className="new-connection" variant="secondary" size="sm" icon={<Plus size="1em" />} tooltip="新建连接 · Ctrl + Shift + T" onClick={props.onCreate} />
    </HStack>
    <VStack className="server-controls" gap={0}>
      <TextInput controlClassName="ide-field-control" label="搜索连接" isLabelHidden placeholder="搜索连接…" size="md" width="100%" startIcon={<Search size="1.15em" />}
        value={props.search} onChange={value => { props.onSearch(value); if (value) setCollapsed({}); }} />
    </VStack>
    <StackItem size="fill" isScrollable className="connection-tree" aria-label="连接分组">
      {props.pending && <Text className="panel-message" color="secondary">正在读取连接…</Text>}
      {props.error && <VStack padding={3}><Banner title="读取失败" status="error" description={props.error}
        endContent={<Button label="重试" onClick={() => props.onRetry?.()} size="sm" />} /></VStack>}
      {!props.pending && !props.error && !groups.length && <VStack padding={4} gap={3}><Text color="secondary">{props.search ? '没有匹配的连接' : '还没有连接'}</Text>{!props.search && <Button label="添加连接" onClick={props.onCreate} size="sm" />}</VStack>}
      {groups.map(([group, profiles]) => <VStack className="connection-group" key={group} gap={0}>
        <Button className="group-toggle" label={`${group}  (${profiles.length})`} size="sm" variant="ghost" aria-expanded={!collapsed[group]}
          icon={collapsed[group] ? <ChevronRight size="1em" /> : <ChevronDown size="1em" />} onClick={() => setCollapsed(previous => ({...previous, [group]: !previous[group]}))}><Text className="group-name">{group} <Text className="group-count">({profiles.length})</Text></Text></Button>
        {!collapsed[group] && <List className="server-list" density="compact" edge="flush">
          {profiles.map(profile => {
            const selected = active?.profile.id === profile.id;
            const connected = props.sessions.some(session => session.profile.id === profile.id && session.status === 'connected');
            return <ListItem className="server-row" data-active={String(selected)} key={profile.id} isSelected={selected}
              label={<Text className="server-name" maxLines={1}>{profile.name}</Text>}
              description={<Text className="server-address" color="secondary" maxLines={1}>{profile.hostname}:{profile.port}</Text>}
              startContent={<Server className={selected ? 'accent-icon' : 'muted-icon'} />}
              endContent={<StatusDot label={connected ? '已连接' : '未连接'} variant={connected ? 'success' : 'neutral'} />}
              onClick={() => props.onOpen(profile)} />;
          })}
        </List>}
      </VStack>)}
    </StackItem>
    <HStack className="connection-footer" vAlign="center"><Button label={`本地连接：${props.profiles.length}`} variant="ghost" size="sm" onClick={props.onManage} /></HStack>
  </VStack>;
}
