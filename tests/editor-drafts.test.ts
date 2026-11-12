import {describe, expect, it} from 'vitest';
import {EditorDrafts} from '../src/features/editor/drafts';
import {isDirty} from '../src/features/editor/types';
import type {EditorDocument} from '../src/features/editor/types';
import {canEditFile} from '../src/features/editor/file-types';

function file(id: string): EditorDocument {
  return {id, sessionId: `session-${id}`, sessionName: '演示服务器', address: '192.0.2.20:22', name: 'app.conf', path: '/srv/demo/app.conf',
    document: {content: 'timeout=30\n', revision: 'version-1', bytes: 11, bom: false, lineEnding: 'LF'}, content: 'timeout=30\n',
    saving: false, restoring: false, error: '', feedback: '', searchRequest: 0, historyRequest: 0, conflict: null};
}
describe('remote editing lifecycle', () => {
  it('save/exit handlers see native editor input before the next React render', () => {
    const drafts = new EditorDrafts();
    const rendered = drafts.update(() => [file('a')]);
    drafts.update(files => files.map(item => ({...item, content: 'timeout=60\n'})));
    expect(isDirty(rendered[0]!)).toBe(false);
    expect(isDirty(drafts.current[0]!)).toBe(true);
    expect(drafts.current[0]!.content).toBe('timeout=60\n');
  });
  it('a save acknowledgement preserves additional input and keeps it dirty', () => {
    const drafts = new EditorDrafts(); drafts.update(() => [file('a')]);
    drafts.update(files => files.map(item => ({...item, content: 'timeout=60\n'})));
    const submitted = drafts.current[0]!.content;
    drafts.update(files => files.map(item => ({...item, content: `${submitted}retries=4\n`})));
    drafts.update(files => files.map(item => ({...item, document: {...item.document, content: submitted, revision: 'version-2'}})));
    expect(drafts.current[0]!.content).toBe('timeout=60\nretries=4\n');
    expect(isDirty(drafts.current[0]!)).toBe(true);
  });
  it('closing one tab cannot roll back a different session save', () => {
    const drafts = new EditorDrafts(); drafts.update(() => [file('a'), {...file('b'), content: 'timeout=90\n'}]);
    drafts.update(files => files.map(item => item.id === 'b' ? {...item, document: {...item.document, content: 'timeout=90\n', revision: 'version-3'}} : item));
    drafts.update(files => files.filter(item => item.id !== 'a'));
    expect(drafts.current).toHaveLength(1);
    expect(drafts.current[0]!.sessionId).toBe('session-b');
    expect(drafts.current[0]!.document.revision).toBe('version-3');
    expect(isDirty(drafts.current[0]!)).toBe(false);
  });
  it('restoring a snapshot changes only the draft until an explicit save', () => {
    const drafts = new EditorDrafts(); drafts.update(() => [file('a')]);
    drafts.update(files => files.map(item => ({...item, content: 'timeout=10\n', restoring: true})));
    expect(drafts.current[0]!.document.revision).toBe('version-1');
    expect(drafts.current[0]!.document.content).toBe('timeout=30\n');
    expect(isDirty(drafts.current[0]!)).toBe(true);
  });
});
describe('file edit entry', () => {
  const entry = {name: 'app.conf', path: '/srv/demo/app.conf', isDirectory: false, isSymlink: false, size: 64};
  it('offers text editing while excluding known binary files, directories and oversize content', () => {
    expect(canEditFile(entry)).toBe(true);
    expect(canEditFile({...entry, name: 'archive.tar.gz'})).toBe(false);
    expect(canEditFile({...entry, isDirectory: true})).toBe(false);
    expect(canEditFile({...entry, size: 2 * 1024 * 1024 + 1})).toBe(false);
  });
});
