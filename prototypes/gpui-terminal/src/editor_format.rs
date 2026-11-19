use std::sync::OnceLock;

use orbitshell_ssh::remote_editor::MAX_BYTES;
use regex::Regex;
use serde::Deserialize;

use crate::editor_syntax::{self, Language, Token};

const MAX_DEPTH: usize = 128;
const TOO_LARGE: &str = "格式化结果超过 2 MiB，草稿未修改。";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatMode {
    Auto,
    Nginx,
    Json,
    Indent,
}

impl FormatMode {
    pub(crate) const MENU: [(Self, &'static str); 4] = [
        (Self::Auto, "自动识别并格式化"),
        (Self::Nginx, "按 Nginx 配置格式化"),
        (Self::Json, "按 JSON 格式化"),
        (Self::Indent, "仅整理当前语言缩进"),
    ];
}

pub(crate) fn format(
    source: &str,
    name: &str,
    language: Language,
    mode: FormatMode,
) -> Result<(String, FormatMode), String> {
    if source.len() > MAX_BYTES {
        return Err("内容超过 2 MiB，草稿未修改。".into());
    }
    let mode = if mode == FormatMode::Auto {
        static NGINX_BLOCK: OnceLock<Regex> = OnceLock::new();
        let nginx_block = NGINX_BLOCK.get_or_init(|| {
            Regex::new(r"(?m)^[\t ]*(?:http|events|server|stream|upstream[ \t]+[^{\n]+|location[ \t]+[^{\n]+)\s*\{").unwrap()
        });
        if name
            .rsplit('.')
            .next()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            FormatMode::Json
        } else if language == Language::Nginx || nginx_block.is_match(source) {
            FormatMode::Nginx
        } else {
            FormatMode::Indent
        }
    } else {
        mode
    };
    let formatted = match mode {
        FormatMode::Nginx => format_nginx(source)?,
        FormatMode::Json => format_json(source)?,
        FormatMode::Indent => indent(source, language)?,
        FormatMode::Auto => unreachable!(),
    };
    Ok((formatted, mode))
}

// Formatting can only change whitespace. Keep the caret and both ends of a
// selection near the same token even if spacings and line breaks were rewritten.
pub(crate) fn map_selection(
    before: &str,
    after: &str,
    selection: [usize; 2],
) -> Result<[usize; 2], String> {
    let (before, after) = (before.as_bytes(), after.as_bytes());
    let (mut old, mut new) = (0, 0);
    let mut mapped = [None, None];
    loop {
        let (old_start, new_start) = (old, new);
        while before.get(old).is_some_and(|&byte| whitespace(byte)) {
            old += 1;
        }
        while after.get(new).is_some_and(|&byte| whitespace(byte)) {
            new += 1;
        }
        for (index, &boundary) in selection.iter().enumerate() {
            if mapped[index].is_none() && (old_start..=old).contains(&boundary) {
                mapped[index] = Some(if boundary == old {
                    new
                } else {
                    (new_start + boundary - old_start).min(new)
                });
            }
        }
        match (before.get(old), after.get(new)) {
            (None, None) => return Ok(mapped.map(|position| position.unwrap_or(new))),
            (Some(left), Some(right)) if left == right => {
                old += 1;
                new += 1;
            }
            _ => return Err("格式化结果改变了原有内容，草稿未修改。".into()),
        }
    }
}

fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn quoted_end(source: &[u8], start: usize) -> Result<usize, String> {
    let quote = source[start];
    let mut index = start + 1;
    while let Some(&byte) = source.get(index) {
        if byte == quote {
            return Ok(index + 1);
        }
        index += if byte == b'\\' { 2 } else { 1 };
    }
    Err("引号没有闭合，草稿未修改。".into())
}

fn append(output: &mut String, text: &str) -> Result<(), String> {
    if output.len() + text.len() > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    output.push_str(text);
    Ok(())
}

fn pad(output: &mut String, width: usize, level: usize) -> Result<(), String> {
    if output.len() + width * level > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    output.extend(std::iter::repeat_n(' ', width * level));
    Ok(())
}

fn format_json(source: &str) -> Result<String, String> {
    // Validate the grammar while retaining the exact spelling, order and
    // multiplicity of tokens, including duplicate keys and large numbers.
    let mut validator = serde_json::Deserializer::from_str(source);
    serde::de::IgnoredAny::deserialize(&mut validator)
        .and_then(|_| validator.end())
        .map_err(|_| "JSON 语法不完整、不合法或嵌套过深，草稿未修改。")?;
    let bytes = source.as_bytes();
    let mut result = String::new();
    let mut cursor = 0;
    let mut depth = 0;
    while let Some(&byte) = bytes.get(cursor) {
        if whitespace(byte) {
            cursor += 1;
            continue;
        }
        let (start, mut end) = (cursor, cursor + 1);
        let mut newline = false;
        match byte {
            b'"' => end = quoted_end(bytes, start)?,
            b'{' | b'[' => {
                if depth >= MAX_DEPTH {
                    return Err("JSON 嵌套过深，草稿未修改。".into());
                }
                let mut next = end;
                while bytes.get(next).is_some_and(|&byte| whitespace(byte)) {
                    next += 1;
                }
                if bytes.get(next) == Some(&(if byte == b'{' { b'}' } else { b']' })) {
                    if result.is_empty() || result.ends_with('\n') {
                        pad(&mut result, 2, depth)?;
                    }
                    append(&mut result, if byte == b'{' { "{}" } else { "[]" })?;
                    cursor = next + 1;
                    continue;
                }
                if result.is_empty() || result.ends_with('\n') {
                    pad(&mut result, 2, depth)?;
                }
                append(&mut result, &source[start..end])?;
                append(&mut result, "\n")?;
                depth += 1;
                cursor = end;
                continue;
            }
            b'}' | b']' => {
                if !result.ends_with('\n') {
                    append(&mut result, "\n")?;
                }
                depth -= 1;
            }
            b',' => newline = true,
            b':' => {}
            _ => {
                while bytes
                    .get(end)
                    .is_some_and(|&b| !whitespace(b) && !matches!(b, b',' | b'}' | b']'))
                {
                    end += 1;
                }
            }
        }
        if result.is_empty() || result.ends_with('\n') {
            pad(&mut result, 2, depth)?;
        }
        append(&mut result, &source[start..end])?;
        if byte == b':' {
            append(&mut result, " ")?;
        }
        if newline {
            append(&mut result, "\n")?;
        }
        cursor = end;
    }
    if !result.ends_with('\n') {
        append(&mut result, "\n")?;
    }
    Ok(result)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Comment,
    Open,
    Close,
    End,
}

struct Part<'a> {
    kind: Kind,
    text: &'a str,
    line: usize,
    end_line: usize,
}

fn nginx_parts(source: &str) -> Result<Vec<Part<'_>>, String> {
    static QUANTIFIER: OnceLock<Regex> = OnceLock::new();
    let quantifier = QUANTIFIER.get_or_init(|| Regex::new(r"\A\{[0-9]+(?:,[0-9]*)?\}").unwrap());
    let bytes = source.as_bytes();
    let mut parts = Vec::new();
    let mut word = None;
    let (mut pos, mut line) = (0, 1);
    while let Some(&byte) = bytes.get(pos) {
        let separator = match byte {
            b'{' => Some(Kind::Open),
            b'}' => Some(Kind::Close),
            b';' => Some(Kind::End),
            _ => None,
        };
        if byte == b'{'
            && let Some((start, _)) = word
        {
            if bytes.get(pos - 1) == Some(&b'$') {
                let end = source[pos + 1..]
                    .find('}')
                    .map(|end| pos + 1 + end)
                    .filter(|&end| {
                        end > pos + 1
                            && bytes[pos + 1..end]
                                .iter()
                                .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    })
                    .ok_or_else(|| format!("第 {line} 行的变量括号不完整，草稿未修改。"))?;
                pos = end + 1;
                continue;
            }
            if start < pos
                && let Some(found) = quantifier.find(&source[pos..])
            {
                pos += found.end();
                continue;
            }
        }
        if whitespace(byte) || separator.is_some() {
            if let Some((start, word_line)) = word.take() {
                parts.push(Part {
                    kind: Kind::Word,
                    text: &source[start..pos],
                    line: word_line,
                    end_line: line,
                });
            }
            if let Some(kind) = separator {
                parts.push(Part {
                    kind,
                    text: &source[pos..pos + 1],
                    line,
                    end_line: line,
                });
            }
            line += usize::from(byte == b'\n');
            pos += 1;
            continue;
        }
        if byte == b'#' && word.is_none() {
            let end = source[pos..]
                .find('\n')
                .map_or(source.len(), |end| pos + end);
            parts.push(Part {
                kind: Kind::Comment,
                text: source[pos..end].trim_end_matches([' ', '\t', '\r']),
                line,
                end_line: line,
            });
            pos = end;
            continue;
        }
        word.get_or_insert((pos, line));
        match byte {
            b'\\' => {
                if matches!(bytes.get(pos + 1), None | Some(b'\n' | b'\r')) {
                    return Err(format!(
                        "第 {line} 行包含未支持的续行或末尾转义，草稿未修改。"
                    ));
                }
                pos += 2;
            }
            b'"' | b'\'' => {
                let end = quoted_end(bytes, pos)?;
                line += bytes[pos..end].iter().filter(|&&b| b == b'\n').count();
                pos = end;
            }
            _ => pos += 1,
        }
    }
    if let Some((start, word_line)) = word {
        parts.push(Part {
            kind: Kind::Word,
            text: &source[start..],
            line: word_line,
            end_line: line,
        });
    }
    Ok(parts)
}

fn format_nginx(source: &str) -> Result<String, String> {
    let parts = nginx_parts(source)?;
    let mut start = None;
    let mut result = String::new();
    let (mut depth, mut last_end_line) = (0, 0);
    for (index, part) in parts.iter().enumerate() {
        match part.kind {
            Kind::Word => {
                start.get_or_insert(index);
                continue;
            }
            Kind::Comment if start.is_some() => continue,
            Kind::Comment if part.line == last_end_line && !result.is_empty() => {
                result.pop();
                append(&mut result, " ")?;
                append(&mut result, part.text)?;
                append(&mut result, "\n")?;
            }
            Kind::Comment => {
                pad(&mut result, 4, depth)?;
                append(&mut result, part.text)?;
                append(&mut result, "\n")?;
            }
            Kind::Close => {
                if start.is_some() {
                    return Err(format!("第 {} 行前的指令缺少分号，草稿未修改。", part.line));
                }
                if depth == 0 {
                    return Err(format!(
                        "第 {} 行存在多余的右花括号，草稿未修改。",
                        part.line
                    ));
                }
                depth -= 1;
                pad(&mut result, 4, depth)?;
                append(&mut result, "}\n")?;
            }
            Kind::Open | Kind::End => {
                let first = start
                    .take()
                    .ok_or_else(|| format!("第 {} 行缺少指令名称，草稿未修改。", part.line))?;
                let pending = &parts[first..index];
                if part.kind == Kind::Open {
                    if depth >= MAX_DEPTH {
                        return Err("配置嵌套过深，草稿未修改。".into());
                    }
                    let directive = pending[0].text.to_ascii_lowercase();
                    if directive == "perl"
                        || (directive.contains("lua") && directive.ends_with("_block"))
                    {
                        return Err(format!("第 {} 行包含嵌入脚本块，草稿未修改。", part.line));
                    }
                }
                let (mut indent, mut needs_space) = (depth, false);
                pad(&mut result, 4, indent)?;
                for item in pending {
                    if needs_space {
                        append(&mut result, " ")?;
                    }
                    append(&mut result, item.text)?;
                    needs_space = item.kind != Kind::Comment;
                    if item.kind == Kind::Comment {
                        append(&mut result, "\n")?;
                        indent = depth + 1;
                        pad(&mut result, 4, indent)?;
                    }
                }
                append(
                    &mut result,
                    if part.kind == Kind::Open {
                        " {\n"
                    } else {
                        ";\n"
                    },
                )?;
                depth += usize::from(part.kind == Kind::Open);
            }
        }
        last_end_line = part.end_line;
    }
    if start.is_some() || depth != 0 {
        return Err("配置存在未结束的指令或花括号，草稿未修改。".into());
    }
    Ok(if result.is_empty() {
        source.to_owned()
    } else {
        result
    })
}

fn indent(source: &str, language: Language) -> Result<String, String> {
    match language {
        Language::Python => indent_python(source),
        Language::Html | Language::Xml => indent_xml(source, language),
        Language::Json
        | Language::JavaScript
        | Language::TypeScript
        | Language::Rust
        | Language::Css
        | Language::Sql
        | Language::Nginx
        | Language::Toml => indent_delimiters(source, language),
        _ => Ok(source.into()),
    }
}

fn indent_delimiters(source: &str, language: Language) -> Result<String, String> {
    let ignored = editor_syntax::highlight(language, source, || false)
        .into_iter()
        .filter(|span| matches!(span.token, Token::String | Token::Comment))
        .collect::<Vec<_>>();
    let mut output = String::new();
    let mut nesting = Vec::new();
    let mut base = 0;
    for line in source.split_inclusive('\n') {
        let content = line.trim_start_matches([' ', '\t']);
        let start = base + line.len() - content.len();
        let already_inside = ignored
            .get(ignored.partition_point(|span| span.range.end <= base))
            .is_some_and(|span| {
                span.range.contains(&base) && (base < start || language != Language::Json)
            });
        let mut symbols = content
            .bytes()
            .enumerate()
            .filter(|(index, b)| {
                (if language == Language::Nginx {
                    matches!(b, b'{' | b'}')
                } else {
                    matches!(b, b'{' | b'}' | b'[' | b']' | b'(' | b')')
                }) && !ignored
                    .get(ignored.partition_point(|span| span.range.end <= start + index))
                    .is_some_and(|span| span.range.contains(&(start + index)))
            })
            .peekable();
        let closes = symbols
            .peek()
            .is_some_and(|&(index, byte)| index == 0 && nesting.last() == Some(&byte));
        if already_inside || content.trim().is_empty() {
            append(&mut output, line)?;
        } else {
            pad(
                &mut output,
                language.tab_width(),
                nesting.len().saturating_sub(usize::from(closes)),
            )?;
            append(&mut output, content)?;
        }
        for (_, byte) in symbols {
            match byte {
                b'{' => nesting.push(b'}'),
                b'(' => nesting.push(b')'),
                b'[' => nesting.push(b']'),
                _ if nesting.last() == Some(&byte) => {
                    nesting.pop();
                }
                _ => {}
            }
            if nesting.len() >= MAX_DEPTH {
                return Err("内容嵌套过深，草稿未修改。".into());
            }
        }
        base += line.len();
    }
    Ok(output)
}

fn indent_python(source: &str) -> Result<String, String> {
    // Python's leading whitespace is syntax. Keep its existing block levels
    // and the indentation inside literals, and normalize only their widths.
    let spans = editor_syntax::highlight(Language::Python, source, || false);
    let mut output = String::new();
    let mut blocks = vec![0];
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.trim_start_matches(' ');
        let leading = line.len() - content.len();
        let inside_literal = spans
            .get(spans.partition_point(|span| span.range.end <= offset))
            .is_some_and(|span| span.token == Token::String && span.range.contains(&offset));
        if inside_literal || content.trim().is_empty() || content.starts_with('#') {
            append(&mut output, line)?;
        } else {
            if content.starts_with('\t') {
                return Ok(source.into());
            }
            if leading < *blocks.last().unwrap() && !blocks.contains(&leading) {
                // Continuations may indent to a delimiter instead of a block.
                return Ok(source.into());
            }
            while *blocks.last().unwrap() > leading {
                blocks.pop();
            }
            if *blocks.last().unwrap() < leading {
                blocks.push(leading);
            }
            pad(&mut output, Language::Python.tab_width(), blocks.len() - 1)?;
            append(&mut output, content)?;
        }
        offset += line.len();
    }
    Ok(output)
}

fn indent_xml(source: &str, language: Language) -> Result<String, String> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| {
        Regex::new(r#"<(/?)([A-Za-z_][\w:.-]*)(?:[^<>"']|"[^"]*"|'[^']*')*>"#).unwrap()
    });
    // Operate on whole tags only. Leave text-bearing and raw-text lines alone.
    let mut stack = Vec::new();
    let mut result = String::new();
    for line in source.split_inclusive('\n') {
        let content = line.trim_start_matches([' ', '\t']);
        let raw_text = stack.last().is_some_and(|name: &String| {
            matches!(name.as_str(), "pre" | "textarea" | "script" | "style")
        });
        if !raw_text && content.starts_with('<') {
            let closing = content.starts_with("</");
            pad(
                &mut result,
                language.tab_width(),
                stack.len().saturating_sub(usize::from(closing)),
            )?;
            append(&mut result, content)?;
        } else {
            append(&mut result, line)?;
        }
        for captures in tag.captures_iter(content) {
            let markup = captures.get(0).unwrap().as_str();
            let name = captures.get(2).unwrap().as_str().to_ascii_lowercase();
            if &captures[1] == "/" {
                if stack.last() == Some(&name) {
                    stack.pop();
                }
            } else if !raw_text
                && !markup.ends_with("/>")
                && !(language == Language::Html
                    && matches!(
                        name.as_str(),
                        "area"
                            | "base"
                            | "br"
                            | "col"
                            | "embed"
                            | "hr"
                            | "img"
                            | "input"
                            | "link"
                            | "meta"
                            | "param"
                            | "source"
                            | "track"
                            | "wbr"
                    ))
            {
                stack.push(name);
                if stack.len() >= MAX_DEPTH {
                    return Err("内容嵌套过深，草稿未修改。".into());
                }
            }
        }
    }
    Ok(result)
}
