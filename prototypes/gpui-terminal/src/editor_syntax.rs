use std::{ops::Range, sync::OnceLock};

use syntect::parsing::{
    ParseState, Scope, ScopeStack, SyntaxDefinition, SyntaxReference, SyntaxSet,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Language {
    #[default]
    Plain,
    Json,
    JavaScript,
    TypeScript,
    Shell,
    Yaml,
    Ini,
    Properties,
    Nginx,
    Css,
    Html,
    Xml,
    Python,
    Rust,
    Sql,
    Toml,
    Markdown,
}

impl Language {
    pub(crate) fn for_file(name: &str, path: &str) -> Self {
        let name = name.to_ascii_lowercase();
        if name.contains("nginx")
            || path
                .split('/')
                .any(|part| part.eq_ignore_ascii_case("nginx"))
        {
            return Self::Nginx;
        }
        match name.rsplit('.').next().unwrap_or_default() {
            "json" => Self::Json,
            "js" | "jsx" => Self::JavaScript,
            "ts" | "tsx" => Self::TypeScript,
            "sh" | "bash" | "zsh" => Self::Shell,
            "yml" | "yaml" => Self::Yaml,
            "ini" => Self::Ini,
            "conf" | "properties" | "env" => Self::Properties,
            "css" => Self::Css,
            "html" => Self::Html,
            "xml" | "svg" => Self::Xml,
            "py" => Self::Python,
            "rs" => Self::Rust,
            "sql" => Self::Sql,
            "toml" => Self::Toml,
            "md" => Self::Markdown,
            _ => Self::Plain,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Plain => "文本",
            Self::Json => "JSON",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Shell => "Shell",
            Self::Yaml => "YAML",
            Self::Ini => "INI",
            Self::Properties => "配置",
            Self::Nginx => "Nginx",
            Self::Css => "CSS",
            Self::Html => "HTML",
            Self::Xml => "XML",
            Self::Python => "Python",
            Self::Rust => "Rust",
            Self::Sql => "SQL",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
        }
    }

    pub(crate) fn tab_width(self) -> usize {
        if self == Self::Nginx { 4 } else { 2 }
    }

    fn syntax(self, set: &SyntaxSet) -> Option<&SyntaxReference> {
        let extension = match self {
            Self::Plain => return None,
            Self::Ini | Self::Properties => {
                return set.find_syntax_by_name("OrbitShell Properties");
            }
            Self::Nginx => return set.find_syntax_by_name("OrbitShell Nginx"),
            Self::Toml => return set.find_syntax_by_name("OrbitShell TOML"),
            Self::TypeScript => {
                return set
                    .find_syntax_by_name("OrbitShell TypeScript")
                    .or_else(|| set.find_syntax_by_extension("js"));
            }
            Self::Json => "json",
            Self::JavaScript => "js",
            Self::Shell => "sh",
            Self::Yaml => "yaml",
            Self::Css => "css",
            Self::Html => "html",
            Self::Xml => "xml",
            Self::Python => "py",
            Self::Rust => "rs",
            Self::Sql => "sql",
            Self::Markdown => "md",
        };
        set.find_syntax_by_extension(extension)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    Comment,
    Keyword,
    String,
    Number,
    Property,
}

#[derive(Clone)]
pub(crate) struct Span {
    pub(crate) range: Range<usize>,
    pub(crate) token: Token,
}

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
        for source in [
            include_str!("editor_syntax/nginx.sublime-syntax"),
            include_str!("editor_syntax/properties.sublime-syntax"),
            include_str!("editor_syntax/toml.sublime-syntax"),
            include_str!("editor_syntax/typescript.sublime-syntax"),
        ] {
            if let Ok(syntax) = SyntaxDefinition::load_from_str(source, true, None) {
                builder.add(syntax);
            }
        }
        builder.build()
    })
}

pub(crate) fn highlight(
    language: Language,
    source: &str,
    cancelled: impl Fn() -> bool,
) -> Vec<Span> {
    let set = syntax_set();
    let Some(syntax) = language.syntax(set) else {
        return Vec::new();
    };
    let mut parser = ParseState::new(syntax);
    let mut scopes = ScopeStack::new();
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if cancelled() {
            return Vec::new();
        }
        // Avoid spending unbounded time inside a grammar on minified input.
        if line.len() > 32 * 1024 {
            parser = ParseState::new(syntax);
            scopes = ScopeStack::new();
        } else if let Ok(ops) = parser.parse_line(line, set) {
            let mut start = 0;
            let end = line.trim_end_matches('\n').len();
            for (pos, operation) in ops {
                push_span(&mut spans, offset, start..pos.min(end), &scopes);
                if scopes.apply(&operation).is_err() {
                    scopes = ScopeStack::new();
                }
                start = pos;
            }
            push_span(&mut spans, offset, start..end, &scopes);
        } else {
            parser = ParseState::new(syntax);
            scopes = ScopeStack::new();
        }
        offset += line.len();
    }
    spans
}

fn push_span(spans: &mut Vec<Span>, offset: usize, range: Range<usize>, scopes: &ScopeStack) {
    if range.is_empty() {
        return;
    }
    let Some(token) = tokenize_scope(scopes.as_slice()) else {
        return;
    };
    let start = offset + range.start;
    let end = offset + range.end;
    if let Some(last) = spans.last_mut()
        && last.token == token
        && last.range.end == start
    {
        last.range.end = end;
    } else {
        spans.push(Span {
            range: start..end,
            token,
        });
    }
}

fn tokenize_scope(scopes: &[Scope]) -> Option<Token> {
    static PREFIXES: OnceLock<Vec<(Scope, Token)>> = OnceLock::new();
    let prefixes = PREFIXES.get_or_init(|| {
        [
            ("comment", Token::Comment),
            ("constant.numeric", Token::Number),
            ("string", Token::String),
            ("keyword", Token::Keyword),
            ("storage", Token::Keyword),
            ("constant.language", Token::Keyword),
            ("support.type.property-name", Token::Property),
            ("support.type", Token::Keyword),
            ("entity.name.type", Token::Keyword),
            ("entity.name.tag", Token::Property),
            ("entity.other.attribute-name", Token::Property),
            ("variable.other.property", Token::Property),
            ("meta.mapping.key", Token::Property),
            ("markup.heading", Token::Keyword),
            ("markup.raw", Token::String),
        ]
        .into_iter()
        .filter_map(|(name, kind)| Scope::new(name).ok().map(|scope| (scope, kind)))
        .collect()
    });
    scopes.iter().rev().find_map(|scope| {
        prefixes
            .iter()
            .find_map(|(prefix, token)| prefix.is_prefix_of(*scope).then_some(*token))
    })
}
