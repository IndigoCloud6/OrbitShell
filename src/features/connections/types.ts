export type AuthenticationMethod = 'password' | 'privateKey';

export interface ConnectionProfile {
  id: string;
  name: string;
  group: string;
  hostname: string;
  port: number;
  username: string;
  authMethod: AuthenticationMethod;
  privateKeyPath: string | null;
}

export function validateProfile(value: unknown): ConnectionProfile {
  if (!value || typeof value !== 'object') throw new Error('连接配置格式无效');
  const profile = value as Record<string, unknown>;
  for (const field of ['id', 'name', 'group', 'hostname', 'username']) {
    if (typeof profile[field] !== 'string') throw new Error('连接配置缺少必要字段');
  }
  if (!Number.isInteger(profile.port) || Number(profile.port) < 1 || Number(profile.port) > 65535) {
    throw new Error('端口必须在 1–65535 之间');
  }
  if (profile.authMethod !== 'password' && profile.authMethod !== 'privateKey') throw new Error('认证方式无效');
  if (profile.privateKeyPath !== null && typeof profile.privateKeyPath !== 'string') throw new Error('私钥路径无效');
  return {
    id: String(profile.id), name: String(profile.name).trim(), group: String(profile.group).trim() || '未分组',
    hostname: String(profile.hostname).trim(), port: Number(profile.port), username: String(profile.username).trim(),
    authMethod: profile.authMethod, privateKeyPath: profile.authMethod === 'privateKey' ? profile.privateKeyPath : null,
  };
}
