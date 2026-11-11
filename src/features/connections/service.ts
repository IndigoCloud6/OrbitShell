import {desktopCommand, isDesktop} from '../../shared/desktop';
import {validateProfile} from './types';
import type {ConnectionProfile} from './types';

const previewKey = 'orbitshell.preview.connections.v1';

export async function listConnections(signal?: AbortSignal): Promise<ConnectionProfile[]> {
  signal?.throwIfAborted();
  const value: unknown = isDesktop
    ? await desktopCommand('list_connections')
    : JSON.parse(localStorage.getItem(previewKey) ?? '[]');
  signal?.throwIfAborted();
  if (!Array.isArray(value)) throw new Error('无法读取连接配置');
  return value.map(validateProfile);
}

export async function saveConnection(profile: ConnectionProfile): Promise<ConnectionProfile> {
  const next = validateProfile(profile);
  if (!next.name || !next.hostname || /\s/.test(next.hostname) || !next.username) throw new Error('请填写连接名称、有效的服务器地址和用户名');
  if (next.authMethod === 'privateKey' && !next.privateKeyPath) throw new Error('请选择私钥文件');
  if (isDesktop) return validateProfile(await desktopCommand('save_connection', {profile: next}));
  const profiles = await listConnections();
  localStorage.setItem(previewKey, JSON.stringify([...profiles.filter(item => item.id !== next.id), next]));
  return next;
}

export async function deleteConnection(id: string): Promise<void> {
  if (isDesktop) return desktopCommand('delete_connection', {id});
  const profiles = await listConnections();
  localStorage.setItem(previewKey, JSON.stringify(profiles.filter(item => item.id !== id)));
}

export const connectionKeys = {all: ['connections'] as const};
