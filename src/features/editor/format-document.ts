export type FormatMode = 'auto' | 'nginx' | 'json' | 'indent';
type NginxToken = {kind: 'word' | 'comment' | '{' | '}' | ';'; value: string; line: number; endLine: number};
const MAX_FORMAT_BYTES = 2 * 1024 * 1024;
const MAX_DEPTH = 128;

export function assertFormatSize(value: string) {
  if (value.length > MAX_FORMAT_BYTES || new TextEncoder().encode(value).byteLength > MAX_FORMAT_BYTES) {
    throw new Error('内容超过 2 MiB，暂不自动格式化，草稿未修改。');
  }
}

export function formatterForFile(name: string, path: string, content: string): Exclude<FormatMode, 'auto'> {
  if (/\.json$/i.test(name)) return 'json';
  if (/nginx/i.test(name) || /(?:^|\/)nginx(?:\/|$)/i.test(path)
    || /(?:^|\n)\s*(?:http|events|server|stream|upstream\s+[^\n{]+|location\s+[^\n{]+)\s*\{/.test(content)) return 'nginx';
  return 'indent';
}

function quotedEnd(source: string, start: number) {
  const quote = source[start];
  for (let index = start + 1; index < source.length; index++) {
    if (source[index] === '\\') { index++; continue; }
    if (source[index] === quote) return index + 1;
  }
  throw new Error('引号没有闭合，未修改草稿。');
}

function nginxTokens(source: string): NginxToken[] {
  const tokens: NginxToken[] = [];
  let value = '';
  let line = 1;
  let wordLine = 1;
  const flush = () => { if (value) { tokens.push({kind: 'word', value, line: wordLine, endLine: line}); value = ''; } };
  for (let index = 0; index < source.length;) {
    const ch = source[index]!;
    if (/[ \t\r\n]/.test(ch)) { flush(); if (ch === '\n') line++; index++; continue; }
    if (!value) wordLine = line;
    if (ch === '\\') {
      const next = source[index + 1];
      if (next === undefined || next === '\n' || next === '\r') throw new Error(`第 ${line} 行包含未支持的续行或末尾转义，未修改草稿。`);
      value += source.slice(index, index + 2); index += 2; continue;
    }
    if (ch === '"' || ch === "'") {
      const end = quotedEnd(source, index);
      const quoted = source.slice(index, end);
      value += quoted; line += quoted.split('\n').length - 1; index = end; continue;
    }
    if (ch === '#' && !value) {
      let end = source.indexOf('\n', index);
      if (end < 0) end = source.length;
      tokens.push({kind: 'comment', value: source.slice(index, end).replace(/[ \t\r]+$/, ''), line, endLine: line}); index = end; continue;
    }
    if (ch === '{' && value.endsWith('$')) {
      const end = source.indexOf('}', index + 1);
      if (end < 0 || !/^[A-Za-z0-9_]+$/.test(source.slice(index + 1, end))) throw new Error(`第 ${line} 行的变量括号不完整，未修改草稿。`);
      value += source.slice(index, end + 1); index = end + 1; continue;
    }
    // Regex quantifiers belong to an argument rather than a configuration block.
    const quantifier = ch === '{' && value ? /^\{\d+(?:,\d*)?\}/.exec(source.slice(index))?.[0] : undefined;
    if (quantifier) { value += quantifier; index += quantifier.length; continue; }
    if (ch === '{' || ch === '}' || ch === ';') {
      flush(); tokens.push({kind: ch, value: ch, line, endLine: line}); index++; continue;
    }
    value += ch; index++;
  }
  flush(); return tokens;
}

export function formatNginx(source: string) {
  assertFormatSize(source);
  const tokens = nginxTokens(source);
  const lines: string[] = [];
  let depth = 0;
  let lastEndLine = 0;
  let pending: NginxToken[] = [];
  let outputLength = 0;
  const emit = (value: string) => {
    outputLength += value.length + 1;
    if (outputLength > MAX_FORMAT_BYTES) throw new Error('格式化结果超过 2 MiB，草稿未修改。');
    lines.push(value);
  };
  const indent = (extra = 0) => '    '.repeat(depth + extra);
  const statement = (suffix: ';' | ' {', endLine: number) => {
    if (!pending.some(token => token.kind === 'word')) throw new Error(`第 ${endLine} 行缺少指令名称，未修改草稿。`);
    let words: string[] = [];
    let continuation = 0;
    for (const token of pending) {
      if (token.kind === 'comment') {
        emit(indent(continuation) + words.join(' ') + (words.length ? ' ' : '') + token.value);
        words = []; continuation = 1;
      } else { words.push(token.value); }
    }
    emit(indent(continuation) + words.join(' ') + suffix);
    pending = []; lastEndLine = endLine;
  };
  for (const token of tokens) {
    if (token.kind === 'word') { pending.push(token); continue; }
    if (token.kind === 'comment') {
      if (pending.length) pending.push(token);
      else if (lines.length && token.line === lastEndLine) {
        outputLength += token.value.length + 1;
        if (outputLength > MAX_FORMAT_BYTES) throw new Error('格式化结果超过 2 MiB，草稿未修改。');
        lines[lines.length - 1] += ` ${token.value}`; lastEndLine = token.endLine;
      }
      else { emit(indent() + token.value); lastEndLine = token.endLine; }
      continue;
    }
    if (token.kind === '{') {
      if (depth >= MAX_DEPTH) throw new Error('配置嵌套过深，暂不自动格式化，草稿未修改。');
      const directive = pending.find(part => part.kind === 'word')?.value ?? '';
      if (/lua.*_block$|^perl$/i.test(directive)) throw new Error(`第 ${token.line} 行包含嵌入脚本块，暂不自动格式化，草稿未修改。`);
      statement(' {', token.endLine); depth++;
    } else if (token.kind === ';') { statement(';', token.endLine); }
    else {
      if (pending.length) throw new Error(`第 ${token.line} 行前的指令缺少分号，未修改草稿。`);
      if (!depth) throw new Error(`第 ${token.line} 行存在多余的右花括号，未修改草稿。`);
      depth--; emit(indent() + '}'); lastEndLine = token.endLine;
    }
  }
  if (pending.length || depth) throw new Error('配置存在未结束的指令或花括号，未修改草稿。');
  const result = lines.length ? `${lines.join('\n')}\n` : source;
  assertFormatSize(result); return result;
}

export function formatJson(source: string) {
  assertFormatSize(source);
  // Validate syntax, but retain the original tokens. Stringifying parsed values
  // would round large integers and discard duplicate keys.
  try { JSON.parse(source); } catch { throw new Error('JSON 语法不完整或不合法，未修改草稿。'); }
  const lines: string[] = [];
  let line = '';
  let depth = 0;
  let outputLength = 0;
  const write = (value: string) => {
    if (!line) line = '  '.repeat(depth);
    if (outputLength + line.length + value.length > MAX_FORMAT_BYTES) throw new Error('格式化结果超过 2 MiB，草稿未修改。');
    line += value;
  };
  const flush = () => { if (line) { outputLength += line.length + 1; lines.push(line); } line = ''; };
  for (let index = 0; index < source.length;) {
    const ch = source[index]!;
    if (/[ \t\r\n]/.test(ch)) { index++; continue; }
    if (ch === '"') { const end = quotedEnd(source, index); write(source.slice(index, end)); index = end; continue; }
    if (ch === '{' || ch === '[') {
      if (depth >= MAX_DEPTH) throw new Error('JSON 嵌套过深，暂不自动格式化，草稿未修改。');
      let next = index + 1; while (next < source.length && /[ \t\r\n]/.test(source[next]!)) next++;
      const close = ch === '{' ? '}' : ']';
      if (source[next] === close) { write(ch + close); index = next + 1; continue; }
      write(ch); flush(); depth++; index++; continue;
    }
    if (ch === '}' || ch === ']') { flush(); depth--; write(ch); index++; continue; }
    if (ch === ',') { write(','); flush(); index++; continue; }
    if (ch === ':') { write(': '); index++; continue; }
    let end = index + 1;
    while (end < source.length && !/[ \t\r\n,}\]]/.test(source[end]!)) end++;
    write(source.slice(index, end)); index = end;
  }
  flush(); const result = `${lines.join('\n')}\n`; assertFormatSize(result); return result;
}

/** Only whitespace becomes an editor change, keeping tokens, selections and undo intact. */
export function formattingChanges(before: string, after: string) {
  const changes: {from: number; to: number; insert: string}[] = [];
  let left = 0; let right = 0;
  while (left < before.length || right < after.length) {
    const start = left; const next = right;
    while (left < before.length && /[ \t\r\n]/.test(before[left]!)) left++;
    while (right < after.length && /[ \t\r\n]/.test(after[right]!)) right++;
    const insert = after.slice(next, right);
    if (before.slice(start, left) !== insert) changes.push({from: start, to: left, insert});
    if (left === before.length && right === after.length) break;
    if (before[left] !== after[right]) throw new Error('格式化结果改变了原有内容，已停止，草稿未修改。');
    left++; right++;
  }
  return changes;
}
