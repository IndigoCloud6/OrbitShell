import type {EditorDocument} from './types';

/** Drafts update synchronously so native editor events and exit/save handlers see
 * the latest input even before React has committed its next render. */
export class EditorDrafts {
  private files: EditorDocument[] = [];
  get current(): EditorDocument[] { return this.files; }
  update(change: (files: EditorDocument[]) => EditorDocument[]): EditorDocument[] {
    this.files = change(this.files);
    return this.files;
  }
}
