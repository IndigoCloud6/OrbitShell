export interface RemoteText {content: string; revision: string; bytes: number; bom: boolean; lineEnding: 'LF' | 'CRLF'}
export interface OpenedFile {id: string; path: string; document: RemoteText; warning: string | null}
export interface SnapshotInfo {id: string; createdAt: number; bytes: number; reason: 'open' | 'save' | 'restore'}
export type SaveResult = {status: 'saved'; document: RemoteText; snapshot: SnapshotInfo} | {status: 'conflict'; current: RemoteText};
export interface EditorDocument {
  id: string; sessionId: string; sessionName: string; address: string; name: string; path: string;
  document: RemoteText; content: string; saving: boolean; restoring: boolean; error: string; feedback: string; searchRequest: number; historyRequest: number;
  conflict: RemoteText | null;
}
export interface EditorTab {id: string; name: string; path: string; sessionName: string; dirty: boolean; saving: boolean; canSave: boolean}
export interface Comparison {documentId: string; original: RemoteText; title: string; kind: 'snapshot' | 'remote'}
export type CloseRequest = {kind: 'file'; id: string} | {kind: 'window'};
export const isDirty = (document: EditorDocument) => document.content !== document.document.content;
export const snapshotLabel = (reason: SnapshotInfo['reason']) => ({open: '首次读取', save: '保存前快照', restore: '回退前快照'})[reason];
