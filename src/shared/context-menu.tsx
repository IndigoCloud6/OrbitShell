import {useCallback, useEffect, useRef, useState} from 'react';
import type {MouseEvent, ReactNode} from 'react';
import {ContextMenu} from '@astryxdesign/core/ContextMenu';
import {VStack} from '@astryxdesign/core/Stack';
import {Text} from '@astryxdesign/core/Text';
import {useToast} from '@astryxdesign/core/Toast';
import {Copy, ClipboardPaste, Scissors, TextSelect} from 'lucide-react';
import {errorMessage} from './desktop';
import type {KeyboardEvent} from 'react';

export function openContextMenuFromKeyboard(event: KeyboardEvent<HTMLElement>) {
  if (event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey ||
    !(event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10'))) return;
  const element = event.target instanceof HTMLElement ? event.target : event.currentTarget;
  if (element.closest('[role="menu"]')) return;
  event.preventDefault();
  event.stopPropagation();
  const rect = element.getBoundingClientRect();
  element.dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true, view: window, button: 2,
    clientX: rect.left, clientY: Math.max(rect.top, rect.bottom - 1)}));
}

export function MenuShortcut({children}: {children: ReactNode}) { return <Text className="menu-shortcut">{children}</Text>; }

export function useCopyText() {
  const toast = useToast();
  return useCallback(async (text: string) => {
    try { await navigator.clipboard.writeText(text); }
    catch { toast({type: 'error', body: '无法写入系统剪贴板，请重试复制。', uniqueID: 'orbit-context-copy'}); }
  }, [toast]);
}

type TextField = HTMLInputElement | HTMLTextAreaElement;
interface InputSnapshot {field: TextField; value: string; start: number; end: number; token: number}
const inputTypes = new Set(['text', 'search', 'url', 'tel', 'email', 'password']);
function textField(target: EventTarget | null): TextField | null {
  const node = target instanceof Element ? target : null;
  const field = node?.closest('input, textarea');
  if (field instanceof HTMLTextAreaElement || (field instanceof HTMLInputElement && inputTypes.has(field.type))) return field;
  return null;
}
function writable(field: TextField) { return !field.disabled && !field.readOnly && !field.closest('[aria-disabled="true"]'); }
function secret(field: TextField) { return field instanceof HTMLInputElement && field.type === 'password'; }

/** Suppress WebView's web-page menu while retaining application and input menus. */
export function AppContextMenus({children}: {children: ReactNode}) {
  const target = useRef<InputSnapshot | null>(null);
  const token = useRef(0);
  const [availability, setAvailability] = useState({copy: false, cut: false, paste: false, select: false});
  const toast = useToast();
  useEffect(() => {
    const preventNative = (event: Event) => event.preventDefault();
    // Prevent the native menu even if a component later stops propagation.
    // Do not stop propagation: regional application menus still receive it.
    document.addEventListener('contextmenu', preventNative, true);
    return () => document.removeEventListener('contextmenu', preventNative, true);
  }, []);
  const capture = (event: MouseEvent<HTMLElement>) => {
    const field = textField(event.target);
    if (!field || field.closest('.terminal-host')) {
      event.preventDefault(); event.stopPropagation(); return;
    }
    const start = field.selectionStart ?? 0;
    const end = field.selectionEnd ?? 0;
    target.current = {field, value: field.value, start, end, token: ++token.current};
    const selected = end > start && !secret(field);
    setAvailability({copy: selected, cut: selected && writable(field), paste: writable(field), select: !field.disabled});
  };
  const replace = (snapshot: InputSnapshot, text: string) => {
    const {field, value, start, end} = snapshot;
    if (!field.isConnected || token.current !== snapshot.token || !writable(field) || field.value !== value) throw new Error('输入内容或目标已发生变化，请重新右击后操作。');
    const insertion = field instanceof HTMLTextAreaElement ? text.replace(/\r\n?/g, '\n') : text.replace(/[\r\n]/g, '');
    const next = `${value.slice(0, start)}${insertion}${value.slice(end)}`;
    const prototype = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
    if (!setter) throw new Error('当前输入框不支持此编辑操作。');
    // Use the native setter so React's controlled input receives a genuine change.
    setter.call(field, next);
    field.dispatchEvent(new InputEvent('input', {bubbles: true, inputType: insertion ? 'insertFromPaste' : 'deleteByCut', data: insertion || null}));
    try { field.setSelectionRange(start + insertion.length, start + insertion.length); } catch { /* email inputs do not expose a text caret */ }
  };
  const edit = async (action: 'copy' | 'cut' | 'paste' | 'select') => {
    const snapshot = target.current;
    if (!snapshot || !snapshot.field.isConnected) return;
    try {
      if (action === 'select') { snapshot.field.focus(); snapshot.field.select(); return; }
      if (action === 'paste') { replace(snapshot, await navigator.clipboard.readText()); return; }
      if (secret(snapshot.field) || snapshot.start === snapshot.end) return;
      await navigator.clipboard.writeText(snapshot.value.slice(snapshot.start, snapshot.end));
      if (action === 'cut') replace(snapshot, '');
    } catch (error) { toast({type: 'error', body: errorMessage(error), uniqueID: 'orbit-context-edit'}); }
  };
  return <ContextMenu data-orbit-context="application" className="orbit-context-menu orbit-text-edit-menu" label="文本编辑" size="sm" menuWidth="calc(var(--orbit-unit) * 18)"
    onOpenChange={open => { if (!open) target.current = null; }} items={[
      {label: '复制', icon: <Copy />, endContent: <MenuShortcut>Ctrl + C</MenuShortcut>, isDisabled: !availability.copy, onClick: () => { void edit('copy'); }},
      {label: '剪切', icon: <Scissors />, endContent: <MenuShortcut>Ctrl + X</MenuShortcut>, isDisabled: !availability.cut, onClick: () => { void edit('cut'); }},
      {label: '粘贴', icon: <ClipboardPaste />, endContent: <MenuShortcut>Ctrl + V</MenuShortcut>, isDisabled: !availability.paste, onClick: () => { void edit('paste'); }},
      {type: 'divider'},
      {label: '全选', icon: <TextSelect />, endContent: <MenuShortcut>Ctrl + A</MenuShortcut>, isDisabled: !availability.select, onClick: () => { void edit('select'); }},
    ]}>
    <VStack className="application-context-content" height="100%" gap={0} onContextMenu={capture}
      onKeyDownCapture={event => {
        const field = textField(event.target);
        if (field && !field.closest('.terminal-host')) openContextMenuFromKeyboard(event);
      }}>{children}</VStack>
  </ContextMenu>;
}
