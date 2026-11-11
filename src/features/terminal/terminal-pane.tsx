import {useEffect, useRef, useState} from 'react';
import {Terminal} from '@xterm/xterm';
import {FitAddon} from '@xterm/addon-fit';
import {SearchAddon} from '@xterm/addon-search';
import type {ISearchOptions} from '@xterm/addon-search';
import {HStack, StackItem} from '@astryxdesign/core/Stack';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Text} from '@astryxdesign/core/Text';
import {IconButton} from '@astryxdesign/core/IconButton';
import {ToggleButton} from '@astryxdesign/core/ToggleButton';
import {ArrowDown, ArrowUp, CaseSensitive, Search, X} from 'lucide-react';
import {useThemePreference} from '../../app/theme';
import {usePreferences} from '../../app/preferences';
import {terminalPalette} from './palette';
import type {ShellSession, TerminalRuntime} from './runtime';

interface Props {
  session: ShellSession;
  runtime: TerminalRuntime;
  active: boolean;
  fontSize: number;
  searchOpen: boolean;
  searchRequest: number;
  onCloseSearch: () => void;
}

export function TerminalPane({session, runtime, active, fontSize, searchOpen, searchRequest, onCloseSearch}: Props) {
  const host = useRef<HTMLElement | null>(null);
  const searchInput = useRef<HTMLInputElement | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const search = useRef<SearchAddon | null>(null);
  const decorations = useRef<ISearchOptions['decorations']>(undefined);
  const [term, setTerm] = useState('');
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [results, setResults] = useState({resultIndex: -1, resultCount: 0});
  const [paletteVersion, setPaletteVersion] = useState(0);
  const {mode} = useThemePreference();
  const {setFontSize} = usePreferences();

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const palette = terminalPalette(element);
    decorations.current = palette.decorations;
    const terminal = new Terminal({
      fontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, "Microsoft YaHei", monospace',
      fontSize, lineHeight: 1.15, fontWeight: 'normal', fontWeightBold: '600', cursorBlink: true, cursorStyle: 'block', cursorInactiveStyle: 'outline',
      scrollback: 10000, scrollOnUserInput: true,
      allowProposedApi: true, disableStdin: true, theme: palette.theme,
    });
    const addon = new FitAddon();
    const searchAddon = new SearchAddon({highlightLimit: 1000});
    fit.current = addon;
    search.current = searchAddon;
    terminal.loadAddon(addon);
    terminal.loadAddon(searchAddon);
    terminal.open(element);
    runtime.terminal = terminal;
    const detach = runtime.attach((bytes, done) => terminal.write(bytes, done));
    const input = terminal.onData(data => runtime.input(data));
    const binary = terminal.onBinary(data => runtime.input(Uint8Array.from(data, character => character.charCodeAt(0))));
    const resize = terminal.onResize(size => { void runtime.resize(size.cols, size.rows); });
    const resultListener = searchAddon.onDidChangeResults(setResults);
    const observer = new ResizeObserver(() => {
      const bounds = element.getBoundingClientRect();
      if (bounds.width > 0 && bounds.height > 0) addon.fit();
    });
    observer.observe(element);
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey || event.deltaY === 0) return;
      event.preventDefault();
      event.stopPropagation();
      setFontSize(value => value + (event.deltaY < 0 ? 1 : -1));
    };
    element.addEventListener('wheel', wheel, {passive: false, capture: true});
    terminal.attachCustomKeyEventHandler(event => {
      if (event.type !== 'keydown' || !event.ctrlKey || !event.shiftKey) return true;
      if (event.code === 'KeyC' && terminal.hasSelection()) {
        void navigator.clipboard.writeText(terminal.getSelection()).catch(() => {});
        return false;
      }
      if (event.code === 'KeyV') {
        void navigator.clipboard.readText().then(text => terminal.paste(text)).catch(() => {});
        return false;
      }
      return true;
    });
    return () => {
      observer.disconnect();
      element.removeEventListener('wheel', wheel, true);
      detach(); input.dispose(); binary.dispose(); resize.dispose(); resultListener.dispose();
      runtime.terminal = null; fit.current = null; search.current = null; terminal.dispose();
    };
    // Font and theme changes update the existing terminal below, preserving its buffer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime, setFontSize]);

  useEffect(() => {
    if (runtime.terminal) runtime.terminal.options.disableStdin = session.status !== 'connected';
  }, [runtime, session.status]);

  useEffect(() => {
    if (!active) return;
    const frame = requestAnimationFrame(() => {
      fit.current?.fit();
      if (searchOpen) { searchInput.current?.focus(); searchInput.current?.select(); }
      else runtime.terminal?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [active, runtime, searchOpen, searchRequest]);

  useEffect(() => {
    if (!runtime.terminal) return;
    runtime.terminal.options.fontSize = fontSize;
    if (active) fit.current?.fit();
  }, [fontSize, runtime, active]);

  useEffect(() => {
    const refresh = () => {
      if (!runtime.terminal || !host.current) return;
      const palette = terminalPalette(host.current);
      runtime.terminal.options.theme = palette.theme;
      decorations.current = palette.decorations;
      setPaletteVersion(value => value + 1);
    };
    const frame = requestAnimationFrame(refresh);
    const media = matchMedia('(prefers-color-scheme: dark)');
    media.addEventListener('change', refresh);
    return () => { cancelAnimationFrame(frame); media.removeEventListener('change', refresh); };
  }, [mode, runtime]);

  useEffect(() => {
    if (!searchOpen || !active || !term) {
      search.current?.clearDecorations();
      search.current?.clearActiveDecoration();
      setResults({resultIndex: -1, resultCount: 0});
      return;
    }
    const timer = setTimeout(() => {
      search.current?.findNext(term, {
        caseSensitive, incremental: true,
        ...(decorations.current ? {decorations: decorations.current} : {}),
      });
    }, 80);
    return () => clearTimeout(timer);
  }, [term, caseSensitive, searchOpen, active, paletteVersion]);

  function find(previous = false) {
    if (!term || !search.current) return;
    const options: ISearchOptions = {caseSensitive, ...(decorations.current ? {decorations: decorations.current} : {})};
    if (previous) search.current.findPrevious(term, options); else search.current.findNext(term, options);
  }
  const countLabel = !term ? '输入以查找' : results.resultCount < 0 ? '1000+ 个匹配' :
    results.resultCount ? `${Math.max(0, results.resultIndex + 1)} / ${results.resultCount}` : '无匹配';

  return <StackItem id={`terminal-${session.id}`} role="region" aria-label={`${session.profile.name}终端`}
    hidden={!active} className="terminal-pane" size="fill">
    {searchOpen && <HStack className="terminal-search" gap={1} vAlign="center" paddingBlock={1} paddingInline={2}
      onKeyDown={event => {
        if (event.nativeEvent.isComposing) return;
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onCloseSearch(); }
      }}>
      <StackItem size="fill"><TextInput ref={searchInput} label="查找终端内容" isLabelHidden placeholder="查找终端内容" size="sm" width="100%"
        startIcon={<Search size="1em" />} value={term} onChange={setTerm}
        onKeyDown={event => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === 'Enter') { event.preventDefault(); find(event.shiftKey); }
        }} /></StackItem>
      <Text className="search-results" type="supporting" color="secondary" role="status" hasTabularNumbers>{countLabel}</Text>
      <ToggleButton label="区分大小写" isIconOnly tooltip="区分大小写" icon={<CaseSensitive size="1em" />} size="sm" isPressed={caseSensitive} onPressedChange={setCaseSensitive} />
      <IconButton label="上一个匹配" tooltip="上一个 · Shift + Enter" icon={<ArrowUp size="1em" />} variant="ghost" size="sm" isDisabled={!term} onClick={() => find(true)} />
      <IconButton label="下一个匹配" tooltip="下一个 · Enter" icon={<ArrowDown size="1em" />} variant="ghost" size="sm" isDisabled={!term} onClick={() => find()} />
      <IconButton label="关闭查找" tooltip="关闭 · Esc" icon={<X size="1em" />} variant="ghost" size="sm" onClick={onCloseSearch} />
    </HStack>}
    <StackItem ref={host} className="terminal-host" size="fill" />
  </StackItem>;
}
