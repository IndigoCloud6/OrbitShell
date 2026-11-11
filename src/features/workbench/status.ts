import type {ShellSession} from '../terminal/runtime';

export function statusLabel(session: ShellSession) {
  return {connecting: '正在连接', hostKey: '等待确认指纹', connected: '已连接', closed: '已断开', error: '连接失败'}[session.status];
}
