import {createContext, useCallback, useContext, useRef} from 'react';
import type {ReactNode} from 'react';

interface Draft {id: string; command: string}
interface DraftContext {stage: (command: string) => void; peek: () => Draft | null; clear: (id: string) => void}
const Context = createContext<DraftContext | null>(null);

/** Explicitly selected terminal text stays in memory, never in URLs/history/storage. */
export function QuickCommandDraftProvider({children}: {children: ReactNode}) {
  const draft = useRef<Draft | null>(null);
  const stage = useCallback((command: string) => { draft.current = {id: crypto.randomUUID(), command}; }, []);
  const peek = useCallback(() => draft.current, []);
  const clear = useCallback((id: string) => { if (draft.current?.id === id) draft.current = null; }, []);
  return <Context value={{stage, peek, clear}}>{children}</Context>;
}
export function useQuickCommandDraft() {
  const context = useContext(Context);
  if (!context) throw new Error('QuickCommandDraftProvider is missing');
  return context;
}
