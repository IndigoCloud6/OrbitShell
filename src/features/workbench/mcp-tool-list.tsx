import {Table, proportional} from '@astryxdesign/core/Table';
import {Text} from '@astryxdesign/core/Text';

const descriptions: Record<string, string> = {
  list_connections: '列出已保存的连接',
  list_sessions: '列出当前会话',
  open_connection: '打开指定连接',
  execute_command: '按明确的终端绑定执行：独立 Shell 或已接管的原标签',
  send_input: '透传输入；兼容 action=takeover / release 控制原终端',
  read_output: '读取绑定终端输出，接管模式读取原标签保留的缓冲',
  take_over_session: '接管已登录的原标签，复用同一 PTY 并锁定人工输入',
  release_session: '交回人工，停止接管权限，不自动终止远端命令',
  disconnect: '断开指定会话',
};

export function McpToolList({tools}: {tools: string[]}) {
  const rows = tools.map(name => ({name, description: descriptions[name] ?? '—'}));
  return <Table className="mcp-tool-table" aria-label="可用 MCP 工具" data={rows} idKey="name" density="compact" dividers="rows" textOverflow="truncate"
    columns={[
      {key: 'name', header: '工具名称', width: proportional(1), renderCell: row => <Text className="mono-text" maxLines={1}>{row.name}</Text>},
      {key: 'description', header: '用途', width: proportional(1.3)},
    ]} />;
}
