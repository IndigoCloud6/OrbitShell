import {Table, proportional} from '@astryxdesign/core/Table';
import {Text} from '@astryxdesign/core/Text';

const descriptions: Record<string, string> = {
  list_connections: '列出已保存的连接',
  list_sessions: '列出当前会话',
  open_connection: '打开指定连接',
  execute_command: '通过 SSH exec 通道执行命令',
  send_input: '向交互终端发送输入',
  read_output: '读取终端输出',
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
