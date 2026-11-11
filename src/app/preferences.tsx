import {createContext, useCallback, useContext, useEffect, useState} from 'react';
import type {Dispatch, ReactNode, SetStateAction} from 'react';
import {errorMessage} from '../shared/desktop';

interface Preferences {
  uiDensity: 'compact' | 'comfortable';
  fontSize: number;
  compactFiles: boolean;
  showHiddenFiles: boolean;
  showServers: boolean;
  showFiles: boolean;
}
interface ContextValue extends Preferences {
  setUiDensity: (value: Preferences['uiDensity']) => void;
  setFontSize: Dispatch<SetStateAction<number>>;
  setCompactFiles: (value: boolean) => void;
  setShowHiddenFiles: (value: boolean) => void;
  setShowServers: (value: boolean) => void;
  setShowFiles: (value: boolean) => void;
  persistenceError: string | null;
}
const Context = createContext<ContextValue | null>(null);
const key = 'orbitshell.preferences.v1';
const defaults: Preferences = {uiDensity: 'compact', fontSize: 17, compactFiles: true, showHiddenFiles: false, showServers: true, showFiles: true};

function read(): Preferences {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? '{}');
    if (!value || typeof value !== 'object') return defaults;
    const saved = value as Record<string, unknown>;
    return {
      uiDensity: saved.uiDensity === 'comfortable' ? 'comfortable' : 'compact',
      fontSize: typeof saved.fontSize === 'number' && Number.isInteger(saved.fontSize) && saved.fontSize >= 11 && saved.fontSize <= 24 ? saved.fontSize : defaults.fontSize,
      compactFiles: typeof saved.compactFiles === 'boolean' ? saved.compactFiles : defaults.compactFiles,
      showHiddenFiles: typeof saved.showHiddenFiles === 'boolean' ? saved.showHiddenFiles : defaults.showHiddenFiles,
      showServers: typeof saved.showServers === 'boolean' ? saved.showServers : defaults.showServers,
      showFiles: typeof saved.showFiles === 'boolean' ? saved.showFiles : defaults.showFiles,
    };
  } catch { return defaults; }
}

export function PreferencesProvider({children}: {children: ReactNode}) {
  const [preferences, setPreferences] = useState(read);
  const [persistenceError, setError] = useState<string | null>(null);
  useEffect(() => {
    try { localStorage.setItem(key, JSON.stringify(preferences)); setError(null); }
    catch (error) { setError(`外观已更新，但保存偏好失败：${errorMessage(error)}`); }
  }, [preferences]);
  const setFontSize = useCallback<Dispatch<SetStateAction<number>>>(value => {
    setPreferences(previous => {
      const size = typeof value === 'function' ? value(previous.fontSize) : value;
      return {...previous, fontSize: Number.isFinite(size) ? Math.max(11, Math.min(24, Math.round(size))) : previous.fontSize};
    });
  }, []);
  const setUiDensity = useCallback((uiDensity: Preferences['uiDensity']) => setPreferences(previous => ({...previous, uiDensity})), []);
  const setCompactFiles = useCallback((compactFiles: boolean) => setPreferences(previous => ({...previous, compactFiles})), []);
  const setShowHiddenFiles = useCallback((showHiddenFiles: boolean) => setPreferences(previous => ({...previous, showHiddenFiles})), []);
  const setShowServers = useCallback((showServers: boolean) => setPreferences(previous => ({...previous, showServers})), []);
  const setShowFiles = useCallback((showFiles: boolean) => setPreferences(previous => ({...previous, showFiles})), []);
  return <Context value={{...preferences, persistenceError, setUiDensity, setFontSize, setCompactFiles, setShowHiddenFiles, setShowServers, setShowFiles}}>{children}</Context>;
}

export function usePreferences() {
  const context = useContext(Context);
  if (!context) throw new Error('PreferencesProvider is missing');
  return context;
}
