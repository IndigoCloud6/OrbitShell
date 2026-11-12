import {HStack} from '@astryxdesign/core/Stack';
import {Link2} from 'lucide-react';
import {useThemePreference} from '../../app/theme';
import associations from '../../assets/file-icons/material/associations.json';
import defaultFile from '../../assets/file-icons/material/file.svg';
import defaultFolder from '../../assets/file-icons/material/folder.svg';
import type {RemoteEntry} from './file-utils';

type Associations = Readonly<Record<string, string>>;
interface Rules {fileNames: Associations; fileExtensions: Associations; folderNames: Associations}
interface Manifest extends Rules {icons: Associations; light: Rules}
const manifest: Manifest = associations;
const assets = import.meta.glob<string>('../../assets/file-icons/material/*.svg', {eager: true, query: '?url', import: 'default'});

function own(map: Associations, key: string) { return Object.hasOwn(map, key) ? map[key] : undefined; }
function iconFor(entry: RemoteEntry, light: boolean) {
  const name = entry.name.toLocaleLowerCase();
  const lookup = (kind: keyof Rules, key: string) => (light ? own(manifest.light[kind], key) : undefined) ?? own(manifest[kind], key);
  let id: string | undefined;
  if (entry.isDirectory) id = lookup('folderNames', name) ?? 'folder';
  else {
    id = lookup('fileNames', name);
    if (!id && name.startsWith('.env.')) id = 'settings';
    // Try compound suffixes before shorter ones: d.ts, tar.gz, then ts or gz.
    for (let dot = name.indexOf('.'); !id && dot >= 0; dot = name.indexOf('.', dot + 1)) {
      id = lookup('fileExtensions', name.slice(dot + 1));
    }
    id ??= 'file';
  }
  const filename = own(manifest.icons, id);
  return (filename ? assets[`../../assets/file-icons/material/${filename}`] : undefined) ?? (entry.isDirectory ? defaultFolder : defaultFile);
}

export function RemoteFileIcon({entry}: {entry: RemoteEntry}) {
  const {mode} = useThemePreference();
  const dark = iconFor(entry, false);
  const light = iconFor(entry, true);
  // Let the browser track system color changes without one media listener per row.
  const lightMedia = mode === 'system' ? '(prefers-color-scheme: light)' : mode === 'light' ? 'all' : 'not all';
  return <HStack className="remote-entry-icon" gap={0} hAlign="center" vAlign="center" aria-hidden="true">
    {light !== dark ? <picture className="remote-file-art">
      <source srcSet={light} media={lightMedia} />
      <img className="remote-file-image" src={dark} alt="" draggable={false} />
    </picture> : <img className="remote-file-image" src={dark} alt="" draggable={false} />}
    {entry.isSymlink && <Link2 className="remote-file-symlink" />}
  </HStack>;
}
