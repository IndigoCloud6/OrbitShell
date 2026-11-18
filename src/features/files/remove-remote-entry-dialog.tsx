import {useEffect, useId, useRef} from 'react';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {HStack, VStack} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {usePreferences} from '../../app/preferences';
import type {RemoteEntry} from './file-utils';

interface Props {
  entry: RemoteEntry;
  server: string;
  busy: boolean;
  error: string;
  onCancel: () => void;
  onConfirm: () => void;
}

export function RemoveRemoteEntryDialog({entry, server, busy, error, onCancel, onConfirm}: Props) {
  const {uiDensity} = usePreferences();
  const description = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (busy) return;
    const frame = requestAnimationFrame(() => cancel.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [busy]);

  const kind = entry.isSymlink ? '符号链接' : entry.isDirectory ? '目录' : '文件';
  const dismiss = (open: boolean) => { if (!open && !busy) onCancel(); };
  return <Dialog isOpen className="orbit-dialog remote-removal-dialog" data-density={uiDensity}
    aria-describedby={description} purpose={busy ? 'required' : 'form'} padding={0}
    width="calc(var(--orbit-unit) * 42)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" onOpenChange={dismiss}>
    <VStack className="orbit-dialog-layout" gap={0}>
      <DialogHeader className="orbit-dialog-header" title={`删除远程${kind}？`} hasDivider {...(!busy ? {onOpenChange: dismiss} : {})} />
      <VStack className="orbit-dialog-body" gap={3} id={description}>
        <Text className="transfer-record-path" color="secondary">{server}</Text>
        <Text className="transfer-record-path mono-text">{entry.path}</Text>
        <Text>{entry.isSymlink ? '只删除链接本身，链接指向的内容会保留。' :
          entry.isDirectory ? '只允许删除空目录。删除后无法撤销。' : '文件将从远程服务器永久删除。'}</Text>
        {error && <Banner status="error" title="删除未完成" description={error} />}
      </VStack>
      <HStack className="orbit-dialog-footer" gap={2} hAlign="end">
        <Button ref={cancel} label={error ? '关闭' : '取消'} variant="secondary" isDisabled={busy} onClick={onCancel} />
        <Button label={`删除${kind}`} variant="destructive" isLoading={busy} isDisabled={busy || Boolean(error)} onClick={onConfirm} />
      </HStack>
    </VStack>
  </Dialog>;
}
