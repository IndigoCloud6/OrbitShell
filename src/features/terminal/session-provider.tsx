import {createContext, useCallback, useContext, useEffect, useRef, useState} from 'react';
import type {ReactNode} from 'react';
import {listen} from '@tauri-apps/api/event';
import {desktopCommand, isDesktop} from '../../shared/desktop';
import {TerminalRuntime} from './runtime';
import type {AuthenticationOptions, SessionEvent, ShellSession} from './runtime';
import type {ConnectionProfile} from '../connections/types';

interface SessionContext {
  sessions: ShellSession[];
  activeId: string | null;
  setActiveId: (id: string | null) => void;
  start: (profile: ConnectionProfile, credential: string, ephemeral?: boolean, options?: AuthenticationOptions) => string;
  close: (id: string) => void;
  getRuntime: (id: string) => TerminalRuntime | undefined;
  trustHost: (id: string, accept: boolean) => Promise<void>;
  pendingProfile: {profile: ConnectionProfile; ephemeral: boolean} | null;
  dismissPending: () => void;
  launchError: string;
  clearLaunchError: () => void;
}
const Context = createContext<SessionContext | null>(null);

export function SessionProvider({children}: {children: ReactNode}) {
  const [sessions, setSessions] = useState<ShellSession[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const runtimes = useRef(new Map<string, TerminalRuntime>());
  const [credentialRequests, setCredentialRequests] = useState<{profile: ConnectionProfile; ephemeral: boolean}[]>([]);
  const [launchError, setLaunchError] = useState('');

  const adopt = useCallback((id: string, profile: ConnectionProfile, ephemeral = false, focus = true) => {
    if (runtimes.current.has(id)) return runtimes.current.get(id)!;
    const runtime = new TerminalRuntime(id, patch => setSessions(previous => previous.map(session => session.id === id ? {...session, ...patch} : session)));
    runtimes.current.set(id, runtime);
    setSessions(previous => [...previous, {id, profile, ephemeral, status: 'connecting', message: '正在连接…', hostKey: null, columns: 120, rows: 36}]);
    if (focus) setActiveId(id);
    return runtime;
  }, []);

  const start = useCallback((profile: ConnectionProfile, credential: string, ephemeral = false, options: AuthenticationOptions = {}) => {
    if (!isDesktop) throw new Error('SSH 连接需要在 OrbitShell 桌面端打开。');
    const id = crypto.randomUUID();
    const runtime = adopt(id, profile, ephemeral);
    void runtime.connect(profile, credential, ephemeral, options);
    return id;
  }, [adopt]);

  const close = useCallback((id: string) => {
    const index = [...runtimes.current.keys()].indexOf(id);
    runtimes.current.get(id)?.dispose();
    runtimes.current.delete(id);
    const remainingIds = [...runtimes.current.keys()];
    setActiveId(current => current === id ? remainingIds[Math.min(Math.max(index, 0), remainingIds.length - 1)] ?? null : current);
    setSessions(previous => previous.filter(session => session.id !== id));
  }, []);

  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    const cleanup: (() => void)[] = [];
    interface LaunchRequest {profile: ConnectionProfile; credential: string | null}
    let draining = false;
    let drainAgain = false;
    const drain = async () => {
      if (draining) { drainAgain = true; return; }
      draining = true;
      try {
        do {
          drainAgain = false;
          const requests = await desktopCommand<LaunchRequest[]>('take_launch_requests');
          for (const request of requests) {
            if (request.credential !== null && request.profile.username) {
              start(request.profile, request.credential, true);
              request.credential = null;
            } else {
              setCredentialRequests(previous => [...previous, {profile: request.profile, ephemeral: true}]);
            }
          }
        } while (drainAgain);
      } catch (error) { setLaunchError(error instanceof Error ? error.message : String(error)); }
      finally { draining = false; }
    };
    void Promise.all([
      listen<{id: string; profile: ConnectionProfile}>('orbitshell:session-opened', event => adopt(event.payload.id, event.payload.profile, false, false)),
      listen<{sessionId: string; event: SessionEvent}>('orbitshell:session-event', event => runtimes.current.get(event.payload.sessionId)?.receive(event.payload.event)),
      listen('orbitshell:launch-available', () => { void drain(); }),
      listen<string>('orbitshell:launch-error', event => setLaunchError(event.payload)),
      listen<ConnectionProfile>('orbitshell:request-credentials', event => setCredentialRequests(previous => [...previous, {profile: event.payload, ephemeral: false}])),
    ]).then(stops => {
      if (disposed) stops.forEach(stop => stop());
      else { cleanup.push(...stops); void drain(); }
    }).catch(error => setLaunchError(error instanceof Error ? error.message : String(error)));
    return () => { disposed = true; cleanup.forEach(stop => stop()); };
  }, [adopt, start]);

  // SessionProvider deliberately lives above route dialogs so opening settings never remounts terminals.
  return <Context value={{sessions, activeId, setActiveId, start, close, getRuntime: id => runtimes.current.get(id),
    pendingProfile: credentialRequests[0] ?? null, dismissPending: () => setCredentialRequests(previous => previous.slice(1)),
    launchError, clearLaunchError: () => setLaunchError(''),
    trustHost: async (sessionId, accept) => { await desktopCommand('ssh_trust_host', {sessionId, accept}); },
  }}>{children}</Context>;
}

export function useSessions() {
  const context = useContext(Context);
  if (!context) throw new Error('SessionProvider is missing');
  return context;
}
