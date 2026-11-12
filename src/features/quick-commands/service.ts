import {useQuery} from '@tanstack/react-query';
import {desktopCommand, isDesktop} from '../../shared/desktop';

export interface QuickCommand {id: string; name: string; group: string; command: string; appendNewline: boolean}
export interface QuickCommands {commands: QuickCommand[]; groups: string[]}
const previewKey = 'orbitshell.preview.quick-commands.v1';
export const quickCommandKey = ['quick-commands'] as const;

function validate(value: unknown): QuickCommand {
  if (!value || typeof value !== 'object') throw new Error('快捷指令格式无效');
  const command = value as Record<string, unknown>;
  if (typeof command.id !== 'string' || typeof command.name !== 'string' || typeof command.group !== 'string' ||
    typeof command.command !== 'string' || typeof command.appendNewline !== 'boolean') throw new Error('快捷指令格式无效');
  return {id: command.id, name: command.name.trim(), group: command.group.trim() || '未分组', command: command.command, appendNewline: command.appendNewline};
}
async function list(signal?: AbortSignal): Promise<QuickCommands> {
  signal?.throwIfAborted();
  const value: unknown = isDesktop ? await desktopCommand('list_quick_commands') : JSON.parse(localStorage.getItem(previewKey) ?? '{"commands":[],"groups":[]}');
  signal?.throwIfAborted();
  if (!value || typeof value !== 'object' || !('commands' in value) || !Array.isArray(value.commands) ||
    !('groups' in value) || !Array.isArray(value.groups) || !value.groups.every(group => typeof group === 'string')) throw new Error('无法读取快捷指令');
  return {commands: value.commands.map(validate), groups: value.groups};
}
export function useQuickCommands() { return useQuery({queryKey: quickCommandKey, queryFn: ({signal}) => list(signal), retry: false}); }

function validateGroup(name: string) {
  const result = name.trim();
  if (!result || new TextEncoder().encode(result).length > 120 || /[\x00-\x1f\x7f]/.test(result)) throw new Error('请填写有效的分组名称（不超过 120 字节）');
  return result;
}
export async function saveQuickCommand(value: QuickCommand): Promise<QuickCommand> {
  const command = validate(value);
  command.group = validateGroup(command.group);
  if (!command.name || new TextEncoder().encode(command.name).length > 160 || /[\x00-\x1f\x7f]/.test(command.name)) throw new Error('请填写有效的指令名称（不超过 160 字节）');
  if (!command.command.trim() || new TextEncoder().encode(command.command).length > 64 * 1024 || command.command.includes('\0')) throw new Error('指令内容不能为空，且不能超过 64 KiB 或包含空字符');
  if (isDesktop) return validate(await desktopCommand('save_quick_command', {command}));
  const collection = await list();
  if (collection.commands.length >= 500 && !collection.commands.some(row => row.id === command.id)) throw new Error('最多保存 500 个快捷指令');
  const commands = collection.commands.some(row => row.id === command.id) ? collection.commands.map(row => row.id === command.id ? command : row) : [...collection.commands, command];
  const groups = [...new Set([...collection.groups, ...(command.group === '未分组' ? [] : [command.group])])];
  if (groups.length > 100) throw new Error('最多保存 100 个指令分组');
  localStorage.setItem(previewKey, JSON.stringify({commands, groups}));
  return command;
}
export async function deleteQuickCommand(id: string) {
  if (isDesktop) return desktopCommand<void>('delete_quick_command', {id});
  const collection = await list();
  localStorage.setItem(previewKey, JSON.stringify({...collection, commands: collection.commands.filter(row => row.id !== id)}));
}
export async function saveQuickGroup(name: string, previousName: string | null) {
  name = validateGroup(name);
  if (name === '未分组' || previousName === '未分组') throw new Error('未分组是保留名称');
  if (isDesktop) return desktopCommand<void>('save_quick_command_group', {name, previousName});
  const collection = await list();
  if (collection.groups.includes(name) && previousName !== name) throw new Error('该分组已存在');
  const groups = previousName ? collection.groups.map(group => group === previousName ? name : group) : [...collection.groups, name];
  if (groups.length > 100) throw new Error('最多保存 100 个指令分组');
  localStorage.setItem(previewKey, JSON.stringify({groups, commands: collection.commands.map(row => row.group === previousName ? {...row, group: name} : row)}));
}
export async function deleteQuickGroup(name: string) {
  if (name === '未分组') throw new Error('未分组不能删除');
  if (isDesktop) return desktopCommand<void>('delete_quick_command_group', {name});
  const collection = await list();
  localStorage.setItem(previewKey, JSON.stringify({groups: collection.groups.filter(group => group !== name), commands: collection.commands.map(row => row.group === name ? {...row, group: '未分组'} : row)}));
}
export function commandGroups(collection: QuickCommands | undefined) {
  return [...new Set([...(collection?.groups ?? []), ...(collection?.commands.map(row => row.group) ?? [])])];
}
export function filterCommands(commands: QuickCommand[], group: string, search: string) {
  const words = search.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return commands.filter(row => (!group || row.group === group) && words.every(word => `${row.name} ${row.command} ${row.group}`.toLocaleLowerCase().includes(word)));
}
