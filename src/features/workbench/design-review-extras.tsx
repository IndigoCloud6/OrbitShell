// Only imported by the development-only design-review route. No IPC or persistence.
import {useState} from 'react';
import {HStack, VStack, StackItem} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Button} from '@astryxdesign/core/Button';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {Plus, Search, Settings2, Terminal} from 'lucide-react';
import {QuickCommandGroups, QuickCommandRows} from '../quick-commands/quick-commands';
import {filterCommands} from '../quick-commands/service';
import type {QuickCommand, QuickCommands} from '../quick-commands/service';
import {RemoteEditorPane} from '../editor/remote-editor-pane';
import type {EditorHistorySource} from '../editor/remote-editor-pane';
import {CodeComparison} from '../editor/code-editor';
import type {Comparison, EditorDocument, RemoteText} from '../editor/types';
import {AiExecutionPanelView} from '../ai/execution-panel';
import type {ExecutionOutput, ExecutionRecord} from '../ai/execution-feed';

const executions: ExecutionRecord[] = ['df -h /srv', 'systemctl status demo-api --no-pager'].map((command, index) => ({
  id: `review-execution-${index}`, sessionId: 'design-profile-0', sessionName: 'api-prod-01', group: '生产环境', hostname: '192.0.2.24', port: 22,
  username: 'deploy', tool: 'execute_command', command, status: 'completed', startedAt: Date.UTC(2026, 10, 12, 8, 30 + index),
  elapsedMs: 245 + index * 120, exitCode: 0, message: '隔离演示记录，并未执行远程命令。',
}));
const readReviewOutput = async (id: string, cursor: number): Promise<ExecutionOutput> => ({cursor: 1, hasMore: false, chunks: cursor ? [] : [
  {stream: 'stdout', data: Array.from(new TextEncoder().encode(id.endsWith('-0') ? 'Filesystem   Size  Used  Avail Use% Mounted on\n/dev/demo    100G   42G    58G  42% /srv\n' : 'demo-api.service — Example service\nActive: active (running)\n'))},
]});
const subscribeReviewOutput = () => () => {};
const refreshReviewExecutions = async () => {};
export function DesignReviewExecutions() {
  const [records, setRecords] = useState(executions);
  return <AiExecutionPanelView activeId="design-profile-0" availableSessionIds={['design-profile-0']} onLocateSession={() => {}}
    mcpEnabled={false} mcpPending={false} mcpError="" mcpTools={[]} onRefreshMcp={() => {}}
    snapshot={{revision: 0, records, loading: false, error: ''}} readOutput={readReviewOutput}
    feed={{subscribeOutput: subscribeReviewOutput, refresh: refreshReviewExecutions, clear: async () => setRecords([])}} />;
}

const commands: QuickCommands = {groups: ['日常巡检', '服务管理'], commands: [
  {id: 'review-command-1', name: '查看磁盘空间', group: '日常巡检', command: 'df -h', appendNewline: true},
  {id: 'review-command-2', name: '查看服务状态', group: '服务管理', command: 'systemctl status demo-api --no-pager', appendNewline: true},
  {id: 'review-command-3', name: '查看最新日志', group: '日常巡检', command: 'journalctl -u demo-api -n 50 --no-pager', appendNewline: false},
]};

export function DesignReviewCommands() {
  const [search, setSearch] = useState('');
  const [group, setGroup] = useState('');
  const [detail, setDetail] = useState<QuickCommand | null>(null);
  return <VStack className="quick-panel" height="100%" gap={0}>
    <HStack className="feature-toolbar quick-toolbar" gap={2} vAlign="center" wrap="wrap">
      <Terminal size="1em" /><Text className="quick-target" maxLines={1}>隔离设计比对 · 不会发送指令</Text>
      <StackItem size="fill" /><TextInput label="搜索快捷指令" isLabelHidden type="search" value={search} onChange={setSearch} hasClear
        placeholder="搜索指令…" startIcon={<Search size="1em" />} controlClassName="feature-search-control" />
      <Button label="管理指令" icon={<Settings2 size="1em" />} size="sm" variant="ghost" isDisabled />
      <Button label="新建" icon={<Plus size="1em" />} size="sm" isDisabled />
    </HStack>
    <HStack className="quick-content" gap={0} vAlign="stretch">
      <QuickCommandGroups collection={commands} active={group} onSelect={setGroup} />
      <StackItem className="quick-list-region" size="fill" isScrollable>
        <QuickCommandRows commands={filterCommands(commands.commands, group, search)} onEdit={setDetail} onSend={() => {}} />
      </StackItem>
    </HStack>
    {detail && <Dialog isOpen className="orbit-dialog" padding={0} width="calc(var(--orbit-unit) * 40)" onOpenChange={open => { if (!open) setDetail(null); }}>
      <VStack className="orbit-dialog-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title={detail.name} hasDivider onOpenChange={open => { if (!open) setDetail(null); }} />
        <VStack className="orbit-dialog-body" gap={3}><Text className="mono-text">{detail.command}</Text><Text color="secondary">隔离演示内容，不读取或修改已保存指令。</Text></VStack>
      </VStack>
    </Dialog>}
  </VStack>;
}

const original = '# Orbit API\n\n本文件用于隔离的界面比对。\n\n## 本地启动\n\n```sh\nnpm run dev\n```\n\n## 运行要求\n\n- Node.js LTS\n- UTF-8 编码\n- Linux / Windows\n\n## 部署说明\n\n配置保存在 `/srv/orbit-api/config`。\n';
const text: RemoteText = {content: original, revision: 'review-original', bytes: new TextEncoder().encode(original).length, bom: false, lineEnding: 'LF'};
const historySource: EditorHistorySource = {
  list: async () => [
    {id: 'review-snapshot-2', createdAt: Date.UTC(2026, 10, 12, 8, 30), bytes: text.bytes, reason: 'save'},
    {id: 'review-snapshot-1', createdAt: Date.UTC(2026, 10, 12, 8), bytes: text.bytes, reason: 'open'},
  ],
  read: async (_id, snapshotId) => snapshotId === 'review-snapshot-1' ? {...text, content: original.replace('## 部署说明', '## 部署')} : text,
};

export function DesignReviewEditor({active}: {active: boolean}) {
  const [file, setFile] = useState<EditorDocument>({id: 'review-editor', sessionId: 'design-profile-0', sessionName: 'api-prod-01',
    address: 'deploy@192.0.2.24:22', name: 'README.md', path: '/srv/orbit-api/README.md', document: text,
    content: original.replace('## 部署说明', '## 部署说明与回退'), saving: false, restoring: false, error: '', feedback: '', searchRequest: 0, historyRequest: 0, conflict: null});
  const [comparison, setComparison] = useState<Comparison | null>(null);
  const [restoreRequest, setRestoreRequest] = useState<{id: string; text: RemoteText} | null>(null);
  return <>
    <RemoteEditorPane file={file} active={active} connected historySource={historySource} controller={{
      patch: (_id, changes) => setFile(previous => ({...previous, ...changes})),
      save: async () => { setFile(previous => ({...previous, feedback: '隔离设计比对：不会写入远程服务器。'})); return false; },
      setComparison, setRestoreRequest,
    }} />
    {comparison && <Dialog isOpen className="orbit-dialog editor-comparison-dialog" padding={0} width="calc(100vw - var(--orbit-unit) * 6)" maxHeight="calc(100dvh - var(--orbit-unit) * 5)"
      onOpenChange={open => { if (!open) setComparison(null); }}>
      <VStack className="editor-comparison-layout" gap={0}>
        <DialogHeader className="orbit-dialog-header" title="README.md · 版本对比" hasDivider onOpenChange={open => { if (!open) setComparison(null); }} />
        <HStack className="editor-diff-labels" gap={0}><StackItem size="fill"><Text>{comparison.title}</Text></StackItem><StackItem size="fill"><Text>当前编辑内容</Text></StackItem></HStack>
        <CodeComparison name={file.name} original={comparison.original.content} current={file.content} />
        <HStack className="orbit-dialog-footer" hAlign="end"><Button label="关闭对比" onClick={() => setComparison(null)} /></HStack>
      </VStack>
    </Dialog>}
    {restoreRequest && <Dialog isOpen className="orbit-dialog" padding={0} width="calc(var(--orbit-unit) * 38)" onOpenChange={open => { if (!open) setRestoreRequest(null); }}>
      <VStack className="orbit-dialog-layout" gap={0}><DialogHeader className="orbit-dialog-header" title="恢复此快照到编辑器？" hasDivider />
        <VStack className="orbit-dialog-body" gap={3}><Text>隔离比对：只替换演示草稿，不写入服务器。</Text></VStack>
        <HStack className="orbit-dialog-footer" gap={2} hAlign="end"><Button label="取消" onClick={() => setRestoreRequest(null)} /><Button label="恢复到编辑器" variant="primary" onClick={() => {
          setFile(previous => ({...previous, content: restoreRequest.text.content})); setRestoreRequest(null);
        }} /></HStack>
      </VStack>
    </Dialog>}
  </>;
}
