use std::{
    ops::Range,
    time::{Duration, Instant},
};

use fancy_regex::{Captures, Regex, RegexBuilder};
use orbitshell_ssh::remote_editor::MAX_BYTES;

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct SearchQuery {
    pub(crate) text: String,
    pub(crate) case_sensitive: bool,
    pub(crate) regex: bool,
    pub(crate) whole_word: bool,
}

pub(crate) struct SearchResults {
    pattern: Regex,
    interpolate: bool,
    pub(crate) ranges: Vec<Range<usize>>,
}

const COMPLEX: &str = "正则表达式过于复杂，请缩小匹配范围。";
const TOO_LARGE: &str = "替换结果超过 2 MiB，草稿未修改。";

impl SearchQuery {
    pub(crate) fn collect(
        &self,
        source: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<SearchResults, String> {
        let source_pattern = if self.regex {
            format!("(?m){}", self.text)
        } else {
            regex::escape(&unquote(&self.text))
        };
        let pattern = RegexBuilder::new(&source_pattern)
            .case_insensitive(!self.case_sensitive)
            .backtrack_limit(100_000)
            .build()
            .map_err(|_| "正则表达式无效。".to_owned())?;
        let start = Instant::now();
        let mut ranges = Vec::new();
        for (index, result) in pattern.find_iter(source).enumerate() {
            if index % 1024 == 0 && (cancelled() || start.elapsed() > Duration::from_secs(1)) {
                return Err(COMPLEX.into());
            }
            let result = result.map_err(|_| COMPLEX)?;
            let range = result.start()..result.end();
            if !self.whole_word || is_whole_word(source, &range) {
                ranges.push(range);
            }
        }
        Ok(SearchResults {
            pattern,
            interpolate: self.regex,
            ranges,
        })
    }
}

impl SearchResults {
    pub(crate) fn replacement(
        &self,
        source: &str,
        found: &Range<usize>,
        template: &str,
    ) -> Result<String, String> {
        let template = unquote(template).replace("\r\n", "\n").replace('\r', "\n");
        if !self.interpolate {
            return Ok(template);
        }
        let captures = self
            .pattern
            .captures_from_pos(source, found.start)
            .map_err(|_| COMPLEX)?
            .ok_or("匹配位置已变化，请重新查找。")?;
        if captures
            .get(0)
            .is_none_or(|item| item.start() != found.start || item.end() != found.end)
        {
            return Err("匹配位置已变化，请重新查找。".into());
        }
        expand(&template, &captures)
    }

    pub(crate) fn replace_all(
        &self,
        source: &str,
        template: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<String, String> {
        let mut result = String::new();
        let mut written = 0;
        let deadline = Instant::now() + Duration::from_secs(1);
        for (index, found) in self.ranges.iter().enumerate() {
            if index % 1024 == 0 && (cancelled() || Instant::now() > deadline) {
                return Err(COMPLEX.into());
            }
            let replacement = self.replacement(source, found, template)?;
            append(&mut result, &source[written..found.start])?;
            append(&mut result, &replacement)?;
            written = found.end;
        }
        append(&mut result, &source[written..])?;
        Ok(result)
    }
}

fn is_whole_word(source: &str, range: &Range<usize>) -> bool {
    fn word(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }
    let matched = &source[range.clone()];
    !(matched.starts_with(word) && source[..range.start].ends_with(word)
        || matched.ends_with(word) && source[range.end..].starts_with(word))
}

fn unquote(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let unescaped = match chars.peek() {
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('\\') => '\\',
                _ => {
                    output.push(ch);
                    continue;
                }
            };
            chars.next();
            output.push(unescaped);
        } else {
            output.push(ch);
        }
    }
    output
}

fn expand(template: &str, captures: &Captures<'_>) -> Result<String, String> {
    let mut output = String::new();
    let mut remaining = template;
    while let Some(index) = remaining.find('$') {
        append(&mut output, &remaining[..index])?;
        remaining = &remaining[index + 1..];
        match remaining.as_bytes().first() {
            Some(b'$') => {
                append(&mut output, "$")?;
                remaining = &remaining[1..];
            }
            Some(b'&') => {
                append(
                    &mut output,
                    captures.get(0).map_or("", |item| item.as_str()),
                )?;
                remaining = &remaining[1..];
            }
            Some(b'0'..=b'9') => {
                let mut group = 0usize;
                let mut end = 0;
                for (index, &digit) in remaining
                    .as_bytes()
                    .iter()
                    .enumerate()
                    .take_while(|(_, byte)| byte.is_ascii_digit())
                {
                    group = group
                        .saturating_mul(10)
                        .saturating_add((digit - b'0') as usize);
                    if group >= captures.len() {
                        break;
                    }
                    if group > 0 {
                        end = index + 1;
                    }
                }
                if end > 0 {
                    let group = remaining[..end].parse().unwrap_or(0);
                    append(
                        &mut output,
                        captures.get(group).map_or("", |item| item.as_str()),
                    )?;
                    remaining = &remaining[end..];
                } else {
                    append(&mut output, "$")?;
                }
            }
            _ => append(&mut output, "$")?,
        }
    }
    append(&mut output, remaining)?;
    Ok(output)
}

fn append(output: &mut String, value: &str) -> Result<(), String> {
    if output.len() + value.len() > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    output.push_str(value);
    Ok(())
}
