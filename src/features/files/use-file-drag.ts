import {useEffect, useRef, useState} from 'react';
import type {PointerEvent, RefObject} from 'react';
import {getCurrentWebview} from '@tauri-apps/api/webview';
import {errorMessage, isDesktop} from '../../shared/desktop';
import type {RemoteEntry} from './file-utils';

interface Options {
  root: RefObject<HTMLElement | null>; contextKey: string; enabled: boolean;
  directory: string; entries: RemoteEntry[];
  upload: (paths: string[], directory: string) => Promise<void>;
  download: (entry: RemoteEntry) => Promise<unknown>;
}

export function useFileDrag(options: Options) {
  const latest = useRef(options); latest.current = options;
  const incoming = useRef<string | null>(null);
  const outgoing = useRef(false);
  const pointerCleanup = useRef<(() => void) | null>(null);
  const [targetDirectory, setTargetDirectory] = useState<string | null>(null);
  const [error, setError] = useState('');
  useEffect(() => { setTargetDirectory(null); setError(''); pointerCleanup.current?.(); }, [options.contextKey]);
  useEffect(() => {
    if (!isDesktop) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent(event => {
      if (disposed || outgoing.current) return;
      const payload = event.payload;
      if (payload.type === 'leave') { incoming.current = null; setTargetDirectory(null); return; }
      const current = latest.current;
      if (payload.type === 'enter' || incoming.current === null) incoming.current = current.contextKey;
      const root = current.root.current;
      const hit = document.elementFromPoint(payload.position.x / window.devicePixelRatio, payload.position.y / window.devicePixelRatio);
      const accepted = incoming.current === current.contextKey && current.enabled && root && hit && root.contains(hit) && !document.querySelector('dialog[open]');
      if (!accepted || !hit) { setTargetDirectory(null); if (payload.type === 'drop') incoming.current = null; return; }
      const row = hit.closest<HTMLElement>('[data-remote-path]');
      const directory = current.entries.find(entry => entry.path === row?.dataset.remotePath && entry.isDirectory)?.path ?? current.directory;
      if (payload.type !== 'drop') { setTargetDirectory(directory); return; }
      incoming.current = null; setTargetDirectory(null); setError('');
      const paths = [...new Set(payload.paths)];
      if (paths.length) void current.upload(paths, directory).catch(failure => { if (!disposed && latest.current.contextKey === current.contextKey) setError(errorMessage(failure)); });
    }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; })
      .catch(failure => { if (!disposed) setError(errorMessage(failure)); });
    return () => { disposed = true; stop?.(); pointerCleanup.current?.(); };
  }, []);

  const beginDownload = (entry: RemoteEntry, event: PointerEvent<HTMLElement>) => {
    if (!isDesktop || event.button !== 0 || event.pointerType !== 'mouse' || entry.isDirectory || !latest.current.enabled || outgoing.current) return;
    pointerCleanup.current?.();
    const current = latest.current;
    const start = {x: event.clientX, y: event.clientY, id: event.pointerId};
    const cleanup = () => {
      window.removeEventListener('pointermove', move, true);
      window.removeEventListener('pointerup', cleanup, true);
      window.removeEventListener('pointercancel', cleanup, true);
      if (pointerCleanup.current === cleanup) pointerCleanup.current = null;
    };
    const move = (pointer: globalThis.PointerEvent) => {
      if (pointer.pointerId !== start.id) return;
      if (!(pointer.buttons & 1) || latest.current.contextKey !== current.contextKey) { cleanup(); return; }
      if (Math.hypot(pointer.clientX - start.x, pointer.clientY - start.y) < 8) return;
      cleanup(); pointer.preventDefault(); outgoing.current = true; setError('');
      void current.download(entry).catch(failure => { if (latest.current.contextKey === current.contextKey) setError(errorMessage(failure)); })
        .finally(() => { outgoing.current = false; });
    };
    pointerCleanup.current = cleanup;
    window.addEventListener('pointermove', move, true);
    window.addEventListener('pointerup', cleanup, true);
    window.addEventListener('pointercancel', cleanup, true);
  };
  return {targetDirectory, error, dismissError: () => setError(''), beginDownload};
}
