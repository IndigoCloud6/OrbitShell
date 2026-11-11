import {createContext, useContext, useEffect, useState} from 'react';
import type {ReactNode} from 'react';
import {Theme} from '@astryxdesign/core/theme';
import {orbitTheme} from './orbit-theme';
import {projectMeta} from './project-meta';

export type ThemePreference = 'system' | 'light' | 'dark';
const storageKey = `${projectMeta.name}.theme.v1`;

function isPreference(value: unknown): value is ThemePreference {
  return value === 'system' || value === 'light' || value === 'dark';
}

function readPreference(): ThemePreference {
  try {
    const saved = localStorage.getItem(storageKey);
    return isPreference(saved) ? saved : 'dark';
  } catch {
    return 'dark';
  }
}

type ThemeContextValue = {
  mode: ThemePreference;
  setMode: (mode: ThemePreference) => void;
  persistenceError: string | null;
};
const ThemePreferenceContext = createContext<ThemeContextValue | null>(null);

export function AppTheme({children}: {children: ReactNode}) {
  const [mode, setPreference] = useState<ThemePreference>(readPreference);
  const [persistenceError, setPersistenceError] = useState<string | null>(null);

  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key === storageKey) setPreference(isPreference(event.newValue) ? event.newValue : 'system');
    };
    window.addEventListener('storage', sync);
    return () => window.removeEventListener('storage', sync);
  }, []);

  function setMode(next: ThemePreference) {
    setPreference(next);
    try {
      localStorage.setItem(storageKey, next);
      setPersistenceError(null);
    } catch {
      setPersistenceError('外观已更新，但浏览器未允许保存此偏好。');
    }
  }

  return (
    <ThemePreferenceContext value={{mode, setMode, persistenceError}}>
      <Theme theme={orbitTheme} mode={mode}>{children}</Theme>
    </ThemePreferenceContext>
  );
}

export function useThemePreference() {
  const value = useContext(ThemePreferenceContext);
  if (!value) throw new Error('useThemePreference requires AppTheme.');
  return value;
}
