export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 3);
  return `${(bytes / 1024 ** unit).toFixed(1)} ${['B', 'KB', 'MB', 'GB'][unit]}`;
}
export interface RemoteEntry {name: string; path: string; isDirectory: boolean; isSymlink: boolean; size: number}
export interface Directory {path: string; entries: RemoteEntry[]}
