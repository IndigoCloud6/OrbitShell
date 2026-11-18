import {useEffect, useRef, useState} from 'react';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {VStack, HStack} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Banner} from '@astryxdesign/core/Banner';
import {usePreferences} from '../../app/preferences';
import {desktopCommand, errorMessage} from '../../shared/desktop';

export interface UploadConflict {token: string; transferId: string; name: string; remotePath: string; sessionName: string}

export function UploadConflictDialog({conflict, onAnswered}: {conflict: UploadConflict; onAnswered: (token: string) => void}) {
  const {uiDensity} = usePreferences();
  const cancel = useRef<HTMLButtonElement | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    const frame = requestAnimationFrame(() => cancel.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [conflict.token]);
  const answer = async (overwrite: boolean) => {
    if (busy) return;
    setBusy(true); setError('');
    try { await desktopCommand('sftp_confirm_upload', {token: conflict.token, overwrite}); onAnswered(conflict.token); }
    catch (failure) { setError(errorMessage(failure)); }
    finally { setBusy(false); }
  };
  return <Dialog isOpen className="orbit-dialog upload-conflict-dialog" data-density={uiDensity} padding={0} purpose={busy ? 'required' : 'form'}
    width="calc(var(--orbit-unit) * 42)" maxHeight="calc(100dvh - var(--orbit-unit) * 6)" onOpenChange={open => { if (!open && !busy) void answer(false); }}>
    <VStack className="orbit-dialog-layout" gap={0}>
      <DialogHeader className="orbit-dialog-header" title="覆盖同名文件？" hasDivider onOpenChange={open => { if (!open && !busy) void answer(false); }} />
      <VStack className="orbit-dialog-body" gap={3}>
        <Text className="transfer-record-path">{conflict.sessionName} 中已存在“{conflict.name}”。</Text>
        <Text className="transfer-record-path mono-text" color="secondary">{conflict.remotePath}</Text>
        <Text>覆盖会使用本机文件替换此远程文件。选择取消将停止此次上传。</Text>
        {error && <Banner status="error" title="确认未完成" description={error} />}
      </VStack>
      <HStack className="orbit-dialog-footer" gap={2} hAlign="end">
        <Button ref={cancel} label="取消上传" variant="secondary" isDisabled={busy} onClick={() => { void answer(false); }} />
        <Button label="覆盖文件" variant="primary" isLoading={busy} isDisabled={busy} onClick={() => { void answer(true); }} />
      </HStack>
    </VStack>
  </Dialog>;
}
