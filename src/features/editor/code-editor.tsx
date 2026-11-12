import {useEffect, useRef, useState} from 'react';
import {basicSetup} from 'codemirror';
import {Compartment, EditorState} from '@codemirror/state';
import type {Extension} from '@codemirror/state';
import {EditorView, keymap} from '@codemirror/view';
import {defaultKeymap, indentWithTab, isolateHistory, redo, undo} from '@codemirror/commands';
import {HighlightStyle, StreamLanguage, syntaxHighlighting} from '@codemirror/language';
import {openSearchPanel} from '@codemirror/search';
import {MergeView} from '@codemirror/merge';
import {tags} from '@lezer/highlight';
import {markdown} from '@codemirror/lang-markdown';
import {javascript, json, typescript} from '@codemirror/legacy-modes/mode/javascript';
import {shell} from '@codemirror/legacy-modes/mode/shell';
import {yaml} from '@codemirror/legacy-modes/mode/yaml';
import {properties} from '@codemirror/legacy-modes/mode/properties';
import {nginx} from '@codemirror/legacy-modes/mode/nginx';
import {css} from '@codemirror/legacy-modes/mode/css';
import {xml} from '@codemirror/legacy-modes/mode/xml';
import {python} from '@codemirror/legacy-modes/mode/python';
import {rust} from '@codemirror/legacy-modes/mode/rust';
import {toml} from '@codemirror/legacy-modes/mode/toml';
import {standardSQL} from '@codemirror/legacy-modes/mode/sql';
import {StackItem} from '@astryxdesign/core/Stack';
import {ContextMenu} from '@astryxdesign/core/ContextMenu';
import {useToast} from '@astryxdesign/core/Toast';
import {ClipboardPaste, Copy, Scissors, Search, TextSelect, Undo2, Redo2} from 'lucide-react';
import {MenuShortcut, openContextMenuFromKeyboard, useCopyText} from '../../shared/context-menu';
import {errorMessage} from '../../shared/desktop';

export function languageName(name: string) {
  const extension = name.split('.').pop()?.toLowerCase();
  if (name.toLowerCase().includes('nginx')) return 'Nginx';
  const names: Record<string, string> = {json: 'JSON', js: 'JavaScript', jsx: 'JavaScript', ts: 'TypeScript', tsx: 'TypeScript', sh: 'Shell', bash: 'Shell', zsh: 'Shell', yml: 'YAML', yaml: 'YAML', ini: 'INI', conf: '配置', properties: '配置', env: '配置', css: 'CSS', html: 'HTML', xml: 'XML', svg: 'XML', py: 'Python', rs: 'Rust', sql: 'SQL', toml: 'TOML', md: 'Markdown'};
  return names[extension ?? ''] ?? '文本';
}
function language(name: string): Extension {
  switch (languageName(name)) {
    case 'JSON': return StreamLanguage.define(json);
    case 'JavaScript': return StreamLanguage.define(javascript);
    case 'TypeScript': return StreamLanguage.define(typescript);
    case 'Shell': return StreamLanguage.define(shell);
    case 'YAML': return StreamLanguage.define(yaml);
    case 'INI': case '配置': return StreamLanguage.define(properties);
    case 'Nginx': return StreamLanguage.define(nginx);
    case 'CSS': return StreamLanguage.define(css);
    case 'HTML': case 'XML': return StreamLanguage.define(xml);
    case 'Python': return StreamLanguage.define(python);
    case 'Rust': return StreamLanguage.define(rust);
    case 'SQL': return StreamLanguage.define(standardSQL);
    case 'TOML': return StreamLanguage.define(toml);
    case 'Markdown': return markdown();
    default: return [];
  }
}
const highlight = HighlightStyle.define([
  {tag: tags.comment, color: 'var(--orbit-editor-comment)'},
  {tag: [tags.keyword, tags.typeName, tags.bool, tags.null], color: 'var(--orbit-editor-keyword)'},
  {tag: [tags.string, tags.regexp], color: 'var(--orbit-editor-string)'},
  {tag: [tags.number, tags.integer, tags.float], color: 'var(--orbit-editor-number)'},
  {tag: [tags.propertyName, tags.attributeName, tags.tagName], color: 'var(--orbit-editor-property)'},
]);
const editorTheme = EditorView.theme({
  '&': {height: '100%', backgroundColor: 'var(--color-background-body)', color: 'var(--color-text-primary)', fontSize: 'var(--orbit-editor-font-size)'},
  '&.cm-focused': {outline: 'none'},
  '.cm-scroller': {fontFamily: '"Cascadia Mono", Consolas, "Microsoft YaHei", monospace', lineHeight: '1.6', overflow: 'auto'},
  '.cm-content': {caretColor: 'var(--color-accent)', paddingBlock: 'calc(var(--orbit-unit) * .75)'},
  '.cm-cursor, .cm-dropCursor': {borderLeftColor: 'var(--color-accent)'},
  '.cm-gutters': {backgroundColor: 'var(--color-background-surface)', color: 'var(--color-text-secondary)', borderRight: 'thin solid var(--color-border)'},
  '.cm-activeLine, .cm-activeLineGutter': {backgroundColor: 'var(--orbit-editor-active-line)'},
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {backgroundColor: 'var(--orbit-editor-selection)'},
  '.cm-panels, .cm-tooltip': {backgroundColor: 'var(--color-background-surface)', color: 'var(--color-text-primary)', borderColor: 'var(--color-border)', fontSize: 'var(--orbit-text-body)'},
  '.cm-textfield, .cm-button': {background: 'var(--color-background-body)', color: 'var(--color-text-primary)', border: 'thin solid var(--color-border)', fontSize: 'inherit'},
});
function extensions(name: string): Extension[] {
  return [basicSetup, keymap.of([...defaultKeymap, indentWithTab]), language(name), syntaxHighlighting(highlight), editorTheme, EditorState.tabSize.of(2),
    EditorState.phrases.of({
      'Control character': '控制字符', 'Selection deleted': '选区已删除', 'Folded lines': '已折叠行', 'Unfolded lines': '已展开行',
      'to': '到', 'folded code': '已折叠代码', 'unfold': '展开', 'Fold line': '折叠行', 'Unfold line': '展开行',
      'Go to line': '跳转到行', 'go': '跳转', 'Find': '查找', 'Replace': '替换', 'next': '下一个', 'previous': '上一个',
      'all': '全部', 'match case': '区分大小写', 'regexp': '正则表达式', 'by word': '全词匹配', 'replace': '替换',
      'replace all': '全部替换', 'close': '关闭', 'current match': '当前匹配', 'no matches': '无匹配',
      'replaced $ occurrences': '已替换 $ 处', 'replaced $ occurrence': '已替换 $ 处',
    }),
  ];
}

interface Props {name: string; value: string; active: boolean; readOnly: boolean; findRequest: number; onChange: (content: string) => void; onCursor: (line: number, column: number) => void}
type EditorContext = {view: EditorView; state: EditorState} | {field: HTMLInputElement | HTMLTextAreaElement; start: number; end: number; value: string};
export function CodeEditor({name, value, active, readOnly, findRequest, onChange, onCursor}: Props) {
  const host = useRef<HTMLElement | null>(null);
  const view = useRef<EditorView | null>(null);
  const editability = useRef(new Compartment());
  const callbacks = useRef({onChange, onCursor}); callbacks.current = {onChange, onCursor};
  const context = useRef<EditorContext | null>(null);
  const [selection, setSelection] = useState(false);
  const [fieldMenu, setFieldMenu] = useState(false);
  const [contextReadOnly, setContextReadOnly] = useState(readOnly);
  const copy = useCopyText(); const toast = useToast();
  useEffect(() => {
    if (!host.current) return;
    const editor = new EditorView({parent: host.current, state: EditorState.create({doc: value, extensions: [
      ...extensions(name), editability.current.of(EditorState.readOnly.of(readOnly)),
      EditorView.contentAttributes.of({'aria-label': `编辑 ${name}`, spellcheck: 'false', 'data-orbit-code-input': 'true'}),
      EditorView.updateListener.of(update => {
        if (update.docChanged) callbacks.current.onChange(update.state.doc.toString());
        if (update.selectionSet || update.docChanged) {
          const head = update.state.selection.main.head; const line = update.state.doc.lineAt(head);
          callbacks.current.onCursor(line.number, head - line.from + 1);
        }
      }),
    ]})});
    view.current = editor;
    return () => { view.current = null; editor.destroy(); };
    // The mounted view owns its undo history; draft, read-only and visibility update below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name]);
  useEffect(() => {
    const editor = view.current;
    if (editor && editor.state.doc.toString() !== value) editor.dispatch({changes: {from: 0, to: editor.state.doc.length, insert: value}, annotations: isolateHistory.of('full')});
  }, [value]);
  useEffect(() => { view.current?.dispatch({effects: editability.current.reconfigure(EditorState.readOnly.of(readOnly))}); }, [readOnly]);
  useEffect(() => { if (active) { view.current?.requestMeasure(); view.current?.focus(); } }, [active]);
  useEffect(() => { if (findRequest && active && view.current) openSearchPanel(view.current); }, [findRequest, active]);
  const edit = async (action: 'copy' | 'cut' | 'paste') => {
    const saved = context.current;
    if (!saved) return;
    try {
      // Search/replace fields belong to the editor view but retain ordinary input editing.
      if ('field' in saved) {
        const {field, start, end, value: original} = saved;
        if (action !== 'paste') await navigator.clipboard.writeText(original.slice(start, end));
        if (action === 'copy') return;
        const text = action === 'paste' ? await navigator.clipboard.readText() : '';
        if (!field.isConnected || field.disabled || field.readOnly || field.value !== original) throw new Error('输入目标已变化，请重新右击后操作。');
        const insertion = field instanceof HTMLTextAreaElement ? text.replace(/\r\n?/g, '\n') : text.replace(/[\r\n]/g, '');
        const prototype = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
        const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
        if (!setter) throw new Error('当前输入框不支持此操作。');
        setter.call(field, `${original.slice(0, start)}${insertion}${original.slice(end)}`);
        field.dispatchEvent(new InputEvent('input', {bubbles: true, inputType: action === 'paste' ? 'insertFromPaste' : 'deleteByCut', data: insertion || null}));
        field.setSelectionRange(start + insertion.length, start + insertion.length);
        return;
      }
      const {view: editor, state} = saved;
      const range = state.selection.main;
      if (action === 'copy') { await copy(state.sliceDoc(range.from, range.to)); return; }
      if (state.readOnly) return;
      if (action === 'cut') await navigator.clipboard.writeText(state.sliceDoc(range.from, range.to));
      const insertion = action === 'paste' ? await navigator.clipboard.readText() : '';
      if (view.current !== editor || editor.state !== state || editor.state.readOnly) throw new Error('编辑目标已变化，请重新右击后操作。');
      editor.dispatch(editor.state.replaceSelection(insertion)); editor.focus();
    } catch (failure) { toast({type: 'error', body: errorMessage(failure), uniqueID: 'editor-clipboard'}); }
  };
  return <ContextMenu data-orbit-context="editor" className="orbit-context-menu" label="文件编辑" size="sm" menuWidth="calc(var(--orbit-unit) * 20)"
    onContextMenuCapture={event => {
      const field = event.target instanceof Element ? event.target.closest('input, textarea') : null;
      if ((field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) && field.selectionStart !== null) {
        const start = field.selectionStart; const end = field.selectionEnd ?? start;
        context.current = {field, start, end, value: field.value}; setSelection(end > start); setFieldMenu(true); setContextReadOnly(field.disabled || field.readOnly);
      } else {
        const editor = view.current; context.current = editor ? {view: editor, state: editor.state} : null;
        setSelection(Boolean(editor && !editor.state.selection.main.empty)); setFieldMenu(false); setContextReadOnly(Boolean(editor?.state.readOnly));
      }
    }}
    onContextMenu={event => event.stopPropagation()} onKeyDownCapture={openContextMenuFromKeyboard}
    onOpenChange={open => { if (!open) context.current = null; }} items={[
      {label: '撤销', icon: <Undo2 />, endContent: <MenuShortcut>Ctrl + Z</MenuShortcut>, isDisabled: readOnly || fieldMenu, onClick: () => { if (view.current) undo(view.current); }},
      {label: '重做', icon: <Redo2 />, endContent: <MenuShortcut>Ctrl + Y</MenuShortcut>, isDisabled: readOnly || fieldMenu, onClick: () => { if (view.current) redo(view.current); }},
      {type: 'divider'},
      {label: '复制', icon: <Copy />, isDisabled: !selection, onClick: () => { void edit('copy'); }},
      {label: '剪切', icon: <Scissors />, isDisabled: !selection || contextReadOnly, onClick: () => { void edit('cut'); }},
      {label: '粘贴', icon: <ClipboardPaste />, isDisabled: contextReadOnly, onClick: () => { void edit('paste'); }},
      {label: '全选', icon: <TextSelect />, onClick: () => {
        const saved = context.current;
        if (saved && 'field' in saved) { saved.field.focus(); saved.field.select(); return; }
        const editor = view.current; if (editor) { editor.dispatch({selection: {anchor: 0, head: editor.state.doc.length}}); editor.focus(); }
      }},
      {type: 'divider'},
      {label: '查找 / 替换', icon: <Search />, endContent: <MenuShortcut>Ctrl + F</MenuShortcut>, isDisabled: fieldMenu, onClick: () => { if (view.current) openSearchPanel(view.current); }},
    ]}><StackItem ref={host} size="fill" className="editor-code-host" /></ContextMenu>;
}

export function CodeComparison({name, original, current}: {name: string; original: string; current: string}) {
  const host = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (!host.current) return;
    const readOnly = [...extensions(name), EditorState.readOnly.of(true)];
    const merge = new MergeView({parent: host.current, a: {doc: original, extensions: readOnly}, b: {doc: current, extensions: readOnly},
      gutter: true, highlightChanges: true, diffConfig: {scanLimit: 500, timeout: 1000}});
    return () => merge.destroy();
  }, [name, original, current]);
  return <StackItem ref={host} size="fill" className="editor-diff-host" aria-label="文件版本差异对比" />;
}
