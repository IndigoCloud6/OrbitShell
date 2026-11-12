import {useEffect, useState} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {Banner} from '@astryxdesign/core/Banner';
import {History, TriangleAlert} from 'lucide-react';
import {CodeComparison} from './code-editor';
import {isDirty} from './types';
import type {EditorController} from './use-remote-editor';

export function EditorDialogs({controller}: {controller: EditorController}) {
  const {documents, comparison, conflictId, closing, closingBusy, restoreRequest} = controller;
  const conflict = documents.find(file => file.id === conflictId);
  const compared = documents.find(file => file.id === comparison?.documentId);
  const closingFiles = closing ? documents.filter(file => closing.kind === 'window' ? isDirty(file) || file.saving : file.id === closing.id) : [];
  const [confirmRemote, setConfirmRemote] = useState<'overwrite' | 'adopt' | null>(null);
  useEffect(() => setConfirmRemote(null), [comparison]);
  const busy = closingBusy || documents.some(file => file.saving);
  return <>
    {conflict?.conflict && <Dialog isOpen className="orbit-dialog" padding={0} width="calc(var(--orbit-unit) * 40)" onOpenChange={open => { if (!open) controller.setConflictId(null); }}>
      <VStack className="orbit-dialog-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title="远程文件已变更" startContent={<TriangleAlert size="1.1em" />} hasDivider />
        <VStack className="orbit-dialog-body" gap={3}><Text>服务器上的文件在你编辑期间被修改。请先对比变更，再决定如何保存。</Text><Text className="mono-text">{conflict.path}</Text><Text color="secondary">你的未保存内容会保留。</Text></VStack>
        <HStack className="orbit-dialog-footer" gap={2} hAlign="end"><Button label="保留当前编辑" onClick={() => controller.setConflictId(null)} /><Button label="查看差异" variant="primary" onClick={() => {
          controller.setConflictId(null); controller.setComparison({documentId: conflict.id, original: conflict.conflict!, title: '当前远程版本', kind: 'remote'});
        }} /></HStack>
      </VStack>
    </Dialog>}
    {comparison && compared && <Dialog isOpen className="orbit-dialog editor-comparison-dialog" padding={0} width="calc(100vw - var(--orbit-unit) * 6)" maxHeight="calc(100dvh - var(--orbit-unit) * 5)"
      onOpenChange={open => { if (!open) { controller.setComparison(null); setConfirmRemote(null); } }}>
      <VStack className="editor-comparison-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title={`${compared.name} · 版本对比`} hasDivider onOpenChange={open => { if (!open) controller.setComparison(null); }} />
        <HStack className="editor-diff-labels" gap={0}><StackItem size="fill"><Text maxLines={1}>{comparison.title}</Text></StackItem><StackItem size="fill"><Text>当前编辑内容</Text></StackItem></HStack>
        <CodeComparison name={compared.name} original={comparison.original.content} current={compared.content} />
        <HStack className="orbit-dialog-footer" gap={2} hAlign="between" wrap="wrap"><Text color="secondary">{comparison.kind === 'snapshot' ? '先恢复为草稿，再确认保存到远程。' : '保存仍会重新核对远程版本，并先记录快照。'}</Text>
          <HStack gap={2} wrap="wrap"><Button label="关闭对比" onClick={() => controller.setComparison(null)} />{comparison.kind === 'snapshot' ? <Button label="恢复此版本到编辑器" variant="primary" isDisabled={busy} onClick={() => controller.setRestoreRequest({id: compared.id, text: comparison.original})} /> : <>
            <Button label="采用远程版本…" isDisabled={busy} onClick={() => setConfirmRemote('adopt')} /><Button label="保存当前编辑…" variant="primary" isDisabled={busy} onClick={() => setConfirmRemote('overwrite')} />
          </>}</HStack>
        </HStack>
      </VStack>
    </Dialog>}
    {confirmRemote && comparison?.kind === 'remote' && compared && <Dialog isOpen className="orbit-dialog" purpose={busy ? 'required' : 'form'} padding={0} width="calc(var(--orbit-unit) * 40)" onOpenChange={open => { if (!open && !busy) setConfirmRemote(null); }}>
      <VStack className="orbit-dialog-layout" gap={0}><DialogHeader className="orbit-dialog-header" hasDivider title={confirmRemote === 'overwrite' ? '以当前编辑更新远程文件？' : '使用远程版本替换当前草稿？'} />
        <VStack className="orbit-dialog-body" gap={3}><Text>{confirmRemote === 'overwrite' ? '将覆盖刚才对比的远程版本，覆盖前会先保存它的本地快照。若远程版本再次变化，将重新提示冲突。' : '当前草稿将替换为刚才读取的远程内容，此操作不会写入服务器。'}</Text><Text className="mono-text">{compared.path}</Text></VStack>
        <HStack className="orbit-dialog-footer" gap={2} hAlign="end"><Button label="取消" isDisabled={busy} onClick={() => setConfirmRemote(null)} /><Button label={confirmRemote === 'overwrite' ? '记录快照并保存' : '采用远程版本'} variant="primary" isLoading={busy} onClick={() => {
          const action = confirmRemote; setConfirmRemote(null);
          if (action === 'overwrite') { controller.setComparison(null); void controller.save(compared.id, comparison.original.revision); }
          else { controller.patch(compared.id, {document: comparison.original, content: comparison.original.content, conflict: null, restoring: false, error: '', feedback: '已采用远程版本。'}); controller.setComparison(null); }
        }} /></HStack>
      </VStack>
    </Dialog>}
    {restoreRequest && <Dialog isOpen className="orbit-dialog" padding={0} width="calc(var(--orbit-unit) * 38)" onOpenChange={open => { if (!open) controller.setRestoreRequest(null); }}>
      <VStack className="orbit-dialog-layout" gap={0}><DialogHeader className="orbit-dialog-header" hasDivider title="恢复此快照到编辑器？" startContent={<History size="1.1em" />} />
        <VStack className="orbit-dialog-body" gap={3}><Text>当前草稿将被替换，远程文件暂不修改。可在编辑器中撤销这次恢复。</Text><Text color="secondary">确认内容后，再点击“保存到远程”。</Text></VStack>
        <HStack className="orbit-dialog-footer" gap={2} hAlign="end"><Button label="取消" onClick={() => controller.setRestoreRequest(null)} /><Button label="恢复到编辑器" variant="primary" isDisabled={busy} onClick={() => {
          controller.restore(restoreRequest.id, restoreRequest.text); controller.setRestoreRequest(null);
        }} /></HStack>
      </VStack>
    </Dialog>}
    {closing && <Dialog isOpen className="orbit-dialog" purpose={busy ? 'required' : 'form'} padding={0} width="calc(var(--orbit-unit) * 42)" onOpenChange={open => { if (!open && !busy) controller.cancelClose(); }}>
      <VStack className="orbit-dialog-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title={closing.kind === 'file' ? `保存对 ${closingFiles[0]?.name ?? '文件'} 的更改？` : closingFiles.length ? '退出前保存远程文件？' : '退出 OrbitShell？'} hasDivider />
        <VStack className="orbit-dialog-body" gap={3}><Text>{busy ? '文件正在保存，请等待保存完成后再退出。' : closingFiles.length ? '文件包含尚未保存的修改。保存后将更新所属服务器上的远程文件。' : '文件已保存，退出会结束当前会话和后台任务。'}</Text>
          {closing.kind === 'window' && closingFiles.map(file => <Text key={file.id} className="mono-text" maxLines={1}>{file.sessionName} · {file.path}</Text>)}
          <HStack gap={2} vAlign="center"><History size="1em" /><Text color="secondary">保存前会先记录本地快照。</Text></HStack>
          {controller.error && <Banner status="error" title="关闭未完成" description={controller.error} />}
        </VStack>
        <HStack className="orbit-dialog-footer" gap={2} hAlign="end">{closingFiles.length > 0 && <Button label={closing.kind === 'window' ? '不保存并退出' : '不保存'} variant="ghost" isDisabled={busy} onClick={() => { void controller.finishClose(false); }} />}<Button label="取消" isDisabled={busy} onClick={controller.cancelClose} /><Button label={closing.kind === 'window' ? closingFiles.length ? '全部保存并退出' : '退出应用' : '保存到远程'} variant="primary" isLoading={busy} onClick={() => { void controller.finishClose(true); }} /></HStack>
      </VStack>
    </Dialog>}
  </>;
}
