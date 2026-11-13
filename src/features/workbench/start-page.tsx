import {useEffect, useRef, useState} from 'react';
import type {FormEvent, RefObject} from 'react';
import {useQueryClient} from '@tanstack/react-query';
import {VStack, HStack, StackItem} from '@astryxdesign/core/Stack';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {TextInput} from '@astryxdesign/core/TextInput';
import {InputGroup, InputGroupText} from '@astryxdesign/core/InputGroup';
import {List, ListItem} from '@astryxdesign/core/List';
import {Banner} from '@astryxdesign/core/Banner';
import {ArrowRight, Plus, Server, Settings, SlidersHorizontal} from 'lucide-react';
import {connectionKeys} from '../connections/service';
import {quickConnect, recentAddress, recentQuickTarget} from '../connections/recent';
import type {RecentConnection, RecentSnapshot} from '../connections/recent';
import {errorMessage, isDesktop} from '../../shared/desktop';

function recentTime(time: number, now: number) {
  const age = Math.max(0, now - time);
  if (age < 60_000) return '刚刚';
  if (age < 60 * 60_000) return `${Math.floor(age / 60_000)} 分钟前`;
  const date = new Date(time);
  if (date.toDateString() === new Date(now).toDateString()) return '今天';
  const yesterday = new Date(now); yesterday.setDate(yesterday.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return '昨天';
  return date.toLocaleDateString('zh-CN');
}

interface StartPageProps {
  records: RecentConnection[]; pending: boolean; historyError: string; busy: boolean; input: string; error: string;
  canConnect: boolean; inputRef: RefObject<HTMLInputElement | null>;
  onInput: (value: string) => void; onConnect: () => void; onRecent: (record: RecentConnection) => void;
  onRetry: () => void; onDismissError: () => void;
}

// The isolated design route can use this view without mounting native services.
export function StartPageView(props: StartPageProps) {
  const [expanded, setExpanded] = useState(false);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, []);
  const records = expanded ? props.records : props.records.slice(0, 4);
  return <VStack className="start-page" height="100%" gap={0} role="region" aria-label="工作区启动页">
    <VStack className="start-page-content" gap={0}>
      <VStack className="start-heading" gap={1}><Heading level={1}>开始工作</Heading><Text color="secondary">输入新的服务器，或继续最近使用的会话</Text></VStack>
      <HStack as="form" className="start-connect" gap={2} vAlign="center" aria-label="快速连接" onSubmit={(event: FormEvent) => {
        event.preventDefault(); props.onConnect();
      }}>
        <StackItem size="fill" className="start-connect-field"><InputGroup label="快速连接地址" isLabelHidden isDisabled={props.busy} className="start-connect-group">
          <InputGroupText className="start-protocol">SSH</InputGroupText>
          <TextInput ref={props.inputRef} label="主机或 SSH 链接" isLabelHidden value={props.input} onChange={props.onInput}
            placeholder="主机地址、user@host 或 SSH 链接" width="100%" autoComplete="off" hasAutoFocus hasSpellCheck={false} hasAutoCorrect={false}
            autoCapitalize="none" controlClassName="start-connect-control" />
        </InputGroup></StackItem>
        <Button label="连接" type="submit" variant="primary" endContent={<ArrowRight size="1em" />} className="start-connect-button"
          isLoading={props.busy} isDisabled={!props.canConnect || !props.input.trim()} />
      </HStack>
      {props.error && <Banner className="compact-feedback start-feedback" status="error" title="暂时无法打开会话" description={props.error} isDismissable onDismiss={props.onDismissError} />}
      {!props.canConnect && <Text className="start-preview-note" color="secondary">浏览器界面预览 · 远程连接请使用桌面端</Text>}
      <HStack className="start-sections" gap={0} vAlign="stretch">
        <VStack className="start-recents" gap={0}>
          <HStack className="start-section-heading" hAlign="between" vAlign="center" gap={2}><Heading level={2}>最近会话</Heading>
            {props.records.length > 4 && <Button label={expanded ? '收起' : '查看全部'} variant="ghost" size="sm" onClick={() => setExpanded(value => !value)} />}
          </HStack>
          {props.historyError && <Banner className="compact-feedback start-feedback" status="warning" title="最近会话暂不可用" description={props.historyError}
            endContent={<Button label="重试" variant="ghost" size="sm" onClick={props.onRetry} />} />}
          {props.pending ? <Text className="start-empty" color="secondary">正在读取最近会话…</Text> : records.length ?
            <List className="start-recent-list" density="compact" edge="flush">
              {records.map(record => <ListItem key={record.id} className="start-recent-row" isDisabled={props.busy || !props.canConnect}
                startContent={<Server className="start-recent-icon" aria-hidden />} onClick={() => props.onRecent(record)}
                label={<HStack className="start-recent-label" gap={3} vAlign="center">
                  <VStack className="start-recent-name" gap={0}><Text className="start-recent-title" maxLines={1}>{record.name}</Text><Text className="start-recent-group" color="secondary" maxLines={1}>{record.group}</Text></VStack>
                  <Text className="start-recent-address mono-text" color="secondary" maxLines={1}>{recentAddress(record)}</Text>
                </HStack>}
                endContent={<HStack className="start-recent-end" gap={3} vAlign="center"><Text className="start-recent-time" color="secondary" hasTabularNumbers>{record.requiresNewLink ? '本次运行' : recentTime(record.lastUsedAt, now)}</Text><ArrowRight aria-hidden /></HStack>} />)}
            </List> : !props.historyError && <VStack className="start-empty" gap={1}><Text>还没有最近会话</Text><Text color="secondary">成功建立连接后，会话会显示在这里。</Text></VStack>}
        </VStack>
        <VStack className="start-shortcuts" gap={0} as="nav" aria-label="启动页快捷操作">
          <HStack className="start-section-heading" vAlign="center"><Heading level={2}>快捷操作</Heading></HStack>
          <Button label="新建连接" icon={<Plus size="1em" />} href="/connections/new" variant="ghost" className="start-shortcut" />
          <Button label="管理连接" icon={<SlidersHorizontal size="1em" />} href="/connections" variant="ghost" className="start-shortcut" />
          <Button label="工作区设置" icon={<Settings size="1em" />} href="/settings" variant="ghost" className="start-shortcut" />
          <Text className="start-keyboard-hint" color="secondary">Ctrl + Shift + T 新建连接</Text>
        </VStack>
      </HStack>
    </VStack>
  </VStack>;
}

export function WorkspaceStart({history, pending, historyError, onRetry, onOpenProfile, onLocateSession}: {
  history: RecentSnapshot | undefined; pending: boolean; historyError: string; onRetry: () => void;
  onOpenProfile: (id: string) => void; onLocateSession: (id: string) => boolean;
}) {
  const cache = useQueryClient();
  const inputRef = useRef<HTMLInputElement | null>(null);
  const working = useRef(false);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const openTarget = async (value: string) => {
    if (working.current) return;
    if (!isDesktop) { setError('远程连接请使用 OrbitShell 桌面端。'); return; }
    if (!value.trim()) { inputRef.current?.focus(); return; }
    working.current = true; setBusy(true); setError('');
    // A pasted URI may contain short-lived credentials. Never retain it as input history.
    setInput('');
    try {
      const result = await quickConnect(value);
      if (result.profileId) {
        await cache.invalidateQueries({queryKey: connectionKeys.all});
        onOpenProfile(result.profileId);
      }
    } catch (failure) { setError(errorMessage(failure)); inputRef.current?.focus(); }
    finally { working.current = false; setBusy(false); }
  };
  const openRecent = (record: RecentConnection) => {
    if (working.current) return;
    setError('');
    if (record.sessionId && onLocateSession(record.sessionId)) return;
    if (record.profileId) { onOpenProfile(record.profileId); return; }
    if (record.requiresNewLink) {
      setInput(''); setError('该临时会话需要新的有效 SSH 链接，请在快速连接中重新粘贴。'); inputRef.current?.focus(); return;
    }
    void openTarget(recentQuickTarget(record));
  };
  return <StartPageView records={history?.records ?? []} pending={pending} historyError={historyError} busy={busy} input={input} error={error}
    canConnect={isDesktop} inputRef={inputRef} onInput={setInput} onConnect={() => { void openTarget(input); }} onRecent={openRecent}
    onRetry={onRetry} onDismissError={() => setError('')} />;
}
