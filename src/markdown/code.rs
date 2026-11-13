//! Highlighting for code blocks: the language from a fence's info string, and a small lexer
//! per language family, written for coloring, not for parsing. It knows comments, strings,
//! numbers and words, so unusual code is colored approximately and never breaks anything.
//! Offsets count characters, the unit the text views use.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Swift,
    Python,
    JavaScript,
    TypeScript,
    Json,
    Html,
    Css,
    Sql,
    Shell,
}

impl Language {
    pub const ALL: [Language; 9] = [
        Language::Swift,
        Language::Python,
        Language::JavaScript,
        Language::TypeScript,
        Language::Json,
        Language::Html,
        Language::Css,
        Language::Sql,
        Language::Shell,
    ];

    /// The first word of a fence's info string, as in ```` ```swift ````, with common aliases.
    /// Anything else is a language without highlighting.
    pub fn from_info(info: &str) -> Option<Language> {
        Some(match first_word(info).to_lowercase().as_str() {
            "swift" => Language::Swift,
            "python" | "py" | "python3" => Language::Python,
            "javascript" | "js" | "jsx" | "mjs" | "cjs" | "node" => Language::JavaScript,
            "typescript" | "ts" | "tsx" => Language::TypeScript,
            "json" | "jsonc" | "json5" => Language::Json,
            "html" | "htm" | "xml" | "xhtml" | "svg" | "plist" => Language::Html,
            "css" | "scss" | "less" => Language::Css,
            "sql" | "mysql" | "postgresql" | "postgres" | "sqlite" => Language::Sql,
            "shell" | "sh" | "bash" | "zsh" | "console" | "shellscript" => Language::Shell,
            _ => return None,
        })
    }

    /// How the language is named in menus and on the block: proper names, not translated.
    pub fn title(self) -> &'static str {
        match self {
            Language::Swift => "Swift",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Json => "JSON",
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Sql => "SQL",
            Language::Shell => "Shell",
        }
    }

    /// What is written after the fence.
    pub fn fence_name(self) -> &'static str {
        match self {
            Language::Swift => "swift",
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Json => "json",
            Language::Html => "html",
            Language::Css => "css",
            Language::Sql => "sql",
            Language::Shell => "shell",
        }
    }

    /// The known languages by name, as the language menu lists them.
    pub fn by_title() -> Vec<Language> {
        let mut all = Self::ALL.to_vec();
        all.sort_by_key(|l| l.title().to_lowercase());
        all
    }
}

fn first_word(info: &str) -> &str {
    let info = info.trim();
    let end = info.find(|c: char| c.is_whitespace() || c == '{' || c == ',').unwrap_or(info.len());
    &info[..end]
}

/// The name shown on a code block: the language's proper name when it is known, else the
/// info string's first word as written; `None` without one.
pub fn label(info: Option<&str>) -> Option<String> {
    let info = info?;
    if let Some(language) = Language::from_info(info) {
        return Some(language.title().to_owned());
    }
    let word = first_word(info);
    (!word.is_empty()).then(|| word.to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    Type,
    String,
    Number,
    Comment,
    /// Object keys, CSS properties and shell variables.
    Property,
    /// HTML and XML tags.
    Tag,
    /// HTML attributes, Python decorators.
    Attribute,
}

impl TokenKind {
    pub const ALL: [TokenKind; 8] = [
        TokenKind::Keyword,
        TokenKind::Type,
        TokenKind::String,
        TokenKind::Number,
        TokenKind::Comment,
        TokenKind::Property,
        TokenKind::Tag,
        TokenKind::Attribute,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TokenKind::Keyword => "tok-keyword",
            TokenKind::Type => "tok-type",
            TokenKind::String => "tok-string",
            TokenKind::Number => "tok-number",
            TokenKind::Comment => "tok-comment",
            TokenKind::Property => "tok-property",
            TokenKind::Tag => "tok-tag",
            TokenKind::Attribute => "tok-attribute",
        }
    }

    /// Light and dark colors, close to Xcode's default theme. The system's saturated colors
    /// looked alike or didn't read on white. `None` for comments: the dimmed text color.
    pub fn color(self, dark: bool) -> Option<&'static str> {
        Some(match (self, dark) {
            (TokenKind::Keyword, false) => "#9B2393",
            (TokenKind::Keyword, true) => "#FF7AB2",
            (TokenKind::Type, false) => "#3900A0",
            (TokenKind::Type, true) => "#DABAFF",
            (TokenKind::String, false) => "#C41A16",
            (TokenKind::String, true) => "#FF8170",
            (TokenKind::Number, false) => "#1C00CF",
            (TokenKind::Number, true) => "#D9C97C",
            (TokenKind::Comment, _) => return None,
            (TokenKind::Property, false) => "#326D74",
            (TokenKind::Property, true) => "#67B7A4",
            (TokenKind::Tag, false) => "#0B4F79",
            (TokenKind::Tag, true) => "#5DD8FF",
            (TokenKind::Attribute, false) => "#815F03",
            (TokenKind::Attribute, true) => "#CC9768",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

pub fn tokens(code: &str, language: Language) -> Vec<Token> {
    let chars: Vec<char> = code.chars().collect();
    match language {
        Language::Html => Markup { c: &chars, pos: 0, out: vec![] }.tokens(),
        _ => Lexer { c: &chars, syntax: syntax(language), pos: 0, out: vec![], braces: 0 }.tokens(),
    }
}

// ------------------------------------------------------------------ languages

struct Syntax {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Longest first, so `"""` wins over `"`: delimiter, multiline, escapes.
    strings: &'static [(&'static str, bool, bool)],
    keywords: HashSet<&'static str>,
    types: HashSet<&'static str>,
    case_insensitive: bool,
    capitalized_are_types: bool,
    /// The prefix of a word that is a keyword in full, as `@` for Swift attributes.
    keyword_prefixes: &'static [char],
    decorator_prefix: Option<char>,
    /// Characters a word may also contain, as `$` in JavaScript.
    extra_word: &'static [char],
    string_prefixes: &'static [&'static str],
    shell_variables: bool,
    comment_needs_boundary: bool,
    keys_are_properties: bool,
    css: bool,
}

impl Default for Syntax {
    fn default() -> Self {
        Self {
            line_comments: &[],
            block_comment: None,
            strings: &[("\"", false, true)],
            keywords: HashSet::new(),
            types: HashSet::new(),
            case_insensitive: false,
            capitalized_are_types: false,
            keyword_prefixes: &[],
            decorator_prefix: None,
            extra_word: &[],
            string_prefixes: &[],
            shell_variables: false,
            comment_needs_boundary: false,
            keys_are_properties: false,
            css: false,
        }
    }
}

const JS_KEYWORDS: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do",
    "else", "export", "extends", "false", "finally", "for", "from", "function", "get", "if", "import", "in",
    "instanceof", "let", "new", "null", "of", "return", "set", "static", "super", "switch", "this", "throw", "true",
    "try", "typeof", "undefined", "var", "void", "while", "with", "yield",
];

static SYNTAXES: LazyLock<Vec<(Language, Syntax)>> = LazyLock::new(|| {
    let set = |words: &[&'static str]| words.iter().copied().collect::<HashSet<_>>();
    vec![
        (Language::Swift, Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &[("\"\"\"", true, true), ("\"", false, true)],
            keywords: set(&[
                "actor", "any", "as", "associatedtype", "async", "await", "borrowing", "break", "case", "catch", "class",
                "consuming", "continue", "convenience", "default", "defer", "deinit", "didSet", "do", "dynamic", "else",
                "enum", "extension", "fallthrough", "false", "fileprivate", "final", "for", "func", "get", "guard", "if",
                "import", "in", "indirect", "init", "inout", "internal", "is", "lazy", "let", "macro", "mutating", "nil",
                "nonisolated", "nonmutating", "open", "operator", "optional", "override", "package", "private",
                "protocol", "public", "repeat", "required", "rethrows", "return", "self", "Self", "sending", "set",
                "some", "static", "struct", "subscript", "super", "switch", "throw", "throws", "true", "try",
                "typealias", "unowned", "var", "weak", "where", "while", "willSet",
            ]),
            capitalized_are_types: true,
            keyword_prefixes: &['@', '#'],
            ..Default::default()
        }),
        (Language::Python, Syntax {
            line_comments: &["#"],
            strings: &[("\"\"\"", true, true), ("'''", true, true), ("\"", false, true), ("'", false, true)],
            keywords: set(&[
                "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del", "elif",
                "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
                "match", "None", "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try", "while",
                "with", "yield",
            ]),
            types: set(&["int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "object"]),
            capitalized_are_types: true,
            decorator_prefix: Some('@'),
            string_prefixes: &["r", "b", "f", "u", "rb", "br", "fr", "rf"],
            ..Default::default()
        }),
        (Language::JavaScript, Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &[("`", true, true), ("\"", false, true), ("'", false, true)],
            keywords: set(JS_KEYWORDS),
            capitalized_are_types: true,
            extra_word: &['$'],
            ..Default::default()
        }),
        (Language::TypeScript, Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &[("`", true, true), ("\"", false, true), ("'", false, true)],
            keywords: {
                let mut k = set(JS_KEYWORDS);
                k.extend([
                    "abstract", "as", "asserts", "declare", "enum", "implements", "infer", "interface", "is", "keyof",
                    "module", "namespace", "override", "private", "protected", "public", "readonly", "satisfies",
                    "type", "unique",
                ]);
                k
            },
            types: set(&["any", "boolean", "never", "number", "object", "string", "symbol", "unknown", "bigint"]),
            capitalized_are_types: true,
            decorator_prefix: Some('@'),
            extra_word: &['$'],
            ..Default::default()
        }),
        (Language::Json, Syntax {
            line_comments: &["//"],
            block_comment: Some(("/*", "*/")),
            keywords: set(&["true", "false", "null"]),
            keys_are_properties: true,
            ..Default::default()
        }),
        (Language::Css, Syntax {
            block_comment: Some(("/*", "*/")),
            strings: &[("\"", false, true), ("'", false, true)],
            keywords: set(&["important"]),
            keyword_prefixes: &['@'],
            extra_word: &['-'],
            css: true,
            ..Default::default()
        }),
        (Language::Sql, Syntax {
            line_comments: &["--"],
            block_comment: Some(("/*", "*/")),
            strings: &[("'", false, false)],
            keywords: set(&[
                "add", "all", "alter", "and", "as", "asc", "begin", "between", "by", "case", "check", "commit",
                "constraint", "create", "default", "delete", "desc", "distinct", "drop", "else", "end", "exists",
                "foreign", "from", "full", "group", "having", "if", "in", "index", "inner", "insert", "into", "is",
                "join", "key", "left", "like", "limit", "not", "null", "offset", "on", "or", "order", "outer",
                "primary", "references", "returning", "right", "rollback", "select", "set", "table", "then",
                "transaction", "union", "unique", "update", "values", "view", "when", "where", "with",
            ]),
            types: set(&[
                "bigint", "blob", "boolean", "char", "date", "decimal", "double", "float", "int", "integer", "json",
                "numeric", "real", "serial", "text", "time", "timestamp", "uuid", "varchar",
            ]),
            case_insensitive: true,
            ..Default::default()
        }),
        (Language::Shell, Syntax {
            line_comments: &["#"],
            strings: &[("\"", true, true), ("'", true, false)],
            keywords: set(&[
                "break", "case", "continue", "do", "done", "echo", "elif", "else", "esac", "exit", "export", "fi", "for",
                "function", "if", "in", "local", "readonly", "return", "select", "set", "shift", "source", "then",
                "unset", "until", "while",
            ]),
            extra_word: &['-'],
            shell_variables: true,
            comment_needs_boundary: true,
            ..Default::default()
        }),
    ]
});

fn syntax(language: Language) -> &'static Syntax {
    &SYNTAXES.iter().find(|(l, _)| *l == language).expect("syntax for every language but HTML").1
}

// ------------------------------------------------------------------ code

struct Lexer<'a> {
    c: &'a [char],
    syntax: &'static Syntax,
    pos: usize,
    out: Vec<Token>,
    braces: usize,
}

fn is_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || c as u32 > 0x7f
}

impl Lexer<'_> {
    fn tokens(mut self) -> Vec<Token> {
        while self.pos < self.c.len() {
            let start = self.pos;
            if self.comment() || self.string(None) || self.number() || self.word() || self.variable() {
                continue;
            }
            match self.c[start] {
                '{' => self.braces += 1,
                '}' => self.braces = self.braces.saturating_sub(1),
                _ => {}
            }
            self.pos += 1;
        }
        self.out
    }

    fn add(&mut self, kind: TokenKind, start: usize) {
        if self.pos > start {
            self.out.push(Token { range: start..self.pos, kind });
        }
    }

    fn matches(&self, text: &str, at: usize) -> bool {
        let mut i = at;
        for ch in text.chars() {
            if self.c.get(i) != Some(&ch) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn is_word(&self, c: char) -> bool {
        is_letter(c) || c.is_ascii_digit() || c == '_' || self.syntax.extra_word.contains(&c)
    }

    fn space_or_start(&self, i: usize) -> bool {
        i == 0 || [' ', '\t', '\n', '\r', ';', '|', '&'].contains(&self.c[i - 1])
    }

    fn comment(&mut self) -> bool {
        let start = self.pos;
        if let Some((open, close)) = self.syntax.block_comment {
            if self.matches(open, self.pos) {
                self.pos += open.chars().count();
                while self.pos < self.c.len() && !self.matches(close, self.pos) {
                    self.pos += 1;
                }
                self.pos = (self.pos + close.chars().count()).min(self.c.len());
                self.add(TokenKind::Comment, start);
                return true;
            }
        }
        for prefix in self.syntax.line_comments {
            if !self.matches(prefix, self.pos) {
                continue;
            }
            // In a shell, `#` only starts a comment at the start of a word: `a#b` is a word.
            if self.syntax.comment_needs_boundary && !self.space_or_start(self.pos) {
                continue;
            }
            while self.pos < self.c.len() && self.c[self.pos] != '\n' {
                self.pos += 1;
            }
            self.add(TokenKind::Comment, start);
            return true;
        }
        false
    }

    fn string(&mut self, from: Option<usize>) -> bool {
        let start = from.unwrap_or(self.pos);
        let Some(&(delimiter, multiline, escapes)) = self.syntax.strings.iter().find(|(d, _, _)| self.matches(d, self.pos)) else {
            return false;
        };
        self.pos += delimiter.chars().count();
        while self.pos < self.c.len() {
            if escapes && self.c[self.pos] == '\\' {
                self.pos = (self.pos + 2).min(self.c.len());
                continue;
            }
            if self.c[self.pos] == '\n' && !multiline {
                break;
            }
            if self.matches(delimiter, self.pos) {
                self.pos += delimiter.chars().count();
                break;
            }
            self.pos += 1;
        }
        let kind = if self.syntax.keys_are_properties && self.colon_after(self.pos) { TokenKind::Property } else { TokenKind::String };
        self.add(kind, start);
        true
    }

    /// A JSON or object key is followed by a colon.
    fn colon_after(&self, at: usize) -> bool {
        let mut i = at;
        while i < self.c.len() && [' ', '\t'].contains(&self.c[i]) {
            i += 1;
        }
        self.c.get(i) == Some(&':')
    }

    fn number(&mut self) -> bool {
        let start = self.pos;
        let c = self.c[start];
        let next = self.c.get(start + 1).copied();
        let starts_with_point = c == '.' && next.is_some_and(|n| n.is_ascii_digit());
        let hex_color = self.syntax.css && c == '#' && next.is_some_and(|n| n.is_ascii_hexdigit());
        if !(c.is_ascii_digit() || starts_with_point || hex_color) {
            return false;
        }
        // A digit inside a word is part of it, but `-` before a number is a sign.
        if start > 0 {
            let previous = self.c[start - 1];
            if is_letter(previous) || previous.is_ascii_digit() || previous == '_' {
                return false;
            }
        }
        self.pos += 1;
        while self.pos < self.c.len() {
            let n = self.c[self.pos];
            let exponent_sign = (n == '+' || n == '-') && matches!(self.c[self.pos - 1], 'e' | 'E') && !hex_color;
            if !(self.is_word(n) || n == '.' || n == '%' || exponent_sign) {
                break;
            }
            self.pos += 1;
        }
        self.add(TokenKind::Number, start);
        true
    }

    fn word(&mut self) -> bool {
        let start = self.pos;
        let c = self.c[start];
        let keyword_prefix = self.syntax.keyword_prefixes.contains(&c);
        let decorator_prefix = self.syntax.decorator_prefix == Some(c);
        // `-` starts words such as `--color` or `-n`, but `-4px` is a number.
        let starts_number = self.c.get(start + 1).is_some_and(|n| n.is_ascii_digit());
        let extra_start = self.syntax.extra_word.contains(&c) && !c.is_ascii_digit() && !starts_number;
        if !(is_letter(c) || c == '_' || extra_start || keyword_prefix || decorator_prefix) {
            return false;
        }
        if start > 0 && self.is_word(self.c[start - 1]) {
            self.pos += 1;
            return true;
        }
        self.pos += 1;
        while self.pos < self.c.len() && self.is_word(self.c[self.pos]) {
            self.pos += 1;
        }
        if self.pos - start == 1 && (keyword_prefix || decorator_prefix) {
            return true;
        }
        let word: String = self.c[start..self.pos].iter().collect();
        // Python's `f"…"` and `r'…'`: the prefix belongs to the string.
        if self.syntax.string_prefixes.contains(&word.to_lowercase().as_str())
            && self.syntax.strings.iter().any(|(d, _, _)| self.matches(d, self.pos))
        {
            return self.string(Some(start));
        }
        if let Some(kind) = self.kind_of(&word, keyword_prefix, decorator_prefix) {
            self.add(kind, start);
        }
        true
    }

    fn kind_of(&self, word: &str, keyword_prefix: bool, decorator_prefix: bool) -> Option<TokenKind> {
        if decorator_prefix {
            return Some(TokenKind::Attribute);
        }
        if keyword_prefix {
            return Some(TokenKind::Keyword);
        }
        let lookup = if self.syntax.case_insensitive { word.to_lowercase() } else { word.to_owned() };
        if self.syntax.css {
            return if self.braces > 0 && self.colon_after(self.pos) {
                Some(TokenKind::Property)
            } else {
                self.syntax.keywords.contains(lookup.as_str()).then_some(TokenKind::Keyword)
            };
        }
        if self.syntax.keywords.contains(lookup.as_str()) {
            return Some(TokenKind::Keyword);
        }
        if self.syntax.types.contains(lookup.as_str()) {
            return Some(TokenKind::Type);
        }
        if self.syntax.capitalized_are_types && word.chars().next().is_some_and(char::is_uppercase) {
            return Some(TokenKind::Type);
        }
        None
    }

    /// Shell variables: `$HOME`, `${name}`, `$1`, `$@`.
    fn variable(&mut self) -> bool {
        if !self.syntax.shell_variables || self.c[self.pos] != '$' || self.pos + 1 >= self.c.len() {
            return false;
        }
        let start = self.pos;
        self.pos += 1;
        let c = self.c[self.pos];
        if c == '{' {
            while self.pos < self.c.len() && self.c[self.pos] != '}' && self.c[self.pos] != '\n' {
                self.pos += 1;
            }
            self.pos = (self.pos + 1).min(self.c.len());
        } else if self.is_word(c) {
            while self.pos < self.c.len() && self.is_word(self.c[self.pos]) && self.c[self.pos] != '-' {
                self.pos += 1;
            }
        } else if ['@', '*', '#', '?', '$', '!'].contains(&c) {
            self.pos += 1;
        } else {
            return true;
        }
        self.add(TokenKind::Property, start);
        true
    }
}

// ------------------------------------------------------------------ markup

/// HTML and XML: comments, tags, attribute names and their values.
struct Markup<'a> {
    c: &'a [char],
    pos: usize,
    out: Vec<Token>,
}

impl Markup<'_> {
    fn tokens(mut self) -> Vec<Token> {
        while self.pos < self.c.len() {
            if self.matches("<!--") {
                let start = self.pos;
                while self.pos < self.c.len() && !self.matches("-->") {
                    self.pos += 1;
                }
                self.pos = (self.pos + 3).min(self.c.len());
                self.add(TokenKind::Comment, start);
            } else if self.c[self.pos] == '<'
                && self.c.get(self.pos + 1).is_some_and(|&n| Self::name_start(n) || ['/', '!', '?'].contains(&n))
            {
                self.tag();
            } else {
                self.pos += 1;
            }
        }
        self.out
    }

    fn add(&mut self, kind: TokenKind, start: usize) {
        if self.pos > start {
            self.out.push(Token { range: start..self.pos, kind });
        }
    }

    fn matches(&self, text: &str) -> bool {
        text.chars().enumerate().all(|(i, ch)| self.c.get(self.pos + i) == Some(&ch))
    }

    fn name_start(c: char) -> bool {
        is_letter(c)
    }

    fn name_char(c: char) -> bool {
        Self::name_start(c) || c.is_ascii_digit() || ['-', '_', ':', '.'].contains(&c)
    }

    /// `<name` and the closing `>` are the tag; inside are attributes and their values.
    fn tag(&mut self) {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.c.len() && ['/', '!', '?'].contains(&self.c[self.pos]) {
            self.pos += 1;
        }
        while self.pos < self.c.len() && Self::name_char(self.c[self.pos]) {
            self.pos += 1;
        }
        self.add(TokenKind::Tag, start);
        while self.pos < self.c.len() && self.c[self.pos] != '>' {
            let c = self.c[self.pos];
            if Self::name_start(c) {
                let s = self.pos;
                while self.pos < self.c.len() && Self::name_char(self.c[self.pos]) {
                    self.pos += 1;
                }
                self.add(TokenKind::Attribute, s);
            } else if c == '"' || c == '\'' {
                let s = self.pos;
                self.pos += 1;
                while self.pos < self.c.len() && self.c[self.pos] != c {
                    self.pos += 1;
                }
                self.pos = (self.pos + 1).min(self.c.len());
                self.add(TokenKind::String, s);
            } else if (c == '/' || c == '?') && self.c.get(self.pos + 1) == Some(&'>') {
                let s = self.pos;
                self.pos += 2;
                self.add(TokenKind::Tag, s);
                return;
            } else {
                self.pos += 1;
            }
        }
        if self.pos < self.c.len() {
            let s = self.pos;
            self.pos += 1;
            self.add(TokenKind::Tag, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Language::*;
    use super::TokenKind::*;
    use super::*;

    // `String` is a token kind here
    fn all(code: &str, language: Language) -> Vec<(std::string::String, TokenKind)> {
        let chars: Vec<char> = code.chars().collect();
        tokens(code, language).into_iter().map(|t| (chars[t.range].iter().collect(), t.kind)).collect()
    }

    fn texts(code: &str, language: Language, kind: TokenKind) -> Vec<std::string::String> {
        all(code, language).into_iter().filter(|(_, k)| *k == kind).map(|(t, _)| t).collect()
    }

    #[test]
    fn fence_names_and_aliases() {
        assert_eq!(Language::from_info("swift"), Some(Swift));
        assert_eq!(Language::from_info("JS"), Some(JavaScript));
        assert_eq!(Language::from_info("ts"), Some(TypeScript));
        assert_eq!(Language::from_info("py title=\"a.py\""), Some(Python));
        assert_eq!(Language::from_info("bash"), Some(Shell));
        assert_eq!(Language::from_info("zsh"), Some(Shell));
        assert_eq!(Language::from_info("xml"), Some(Html));
        assert_eq!(Language::from_info("json{1,3}"), Some(Json));
        assert_eq!(Language::from_info(""), None);
        assert_eq!(Language::from_info("cobol"), None);
        for l in Language::ALL {
            assert_eq!(Language::from_info(l.fence_name()), Some(l));
        }
        assert_eq!(label(Some("rust ignore")).as_deref(), Some("rust"));
        assert_eq!(label(Some("ts")).as_deref(), Some("TypeScript"));
        assert_eq!(label(None), None);
        let titles: Vec<&str> = Language::by_title().into_iter().map(Language::title).collect();
        assert_eq!(titles, ["CSS", "HTML", "JavaScript", "JSON", "Python", "Shell", "SQL", "Swift", "TypeScript"]);
    }

    #[test]
    fn swift() {
        let expected: Vec<(std::string::String, TokenKind)> =
            [("let", Keyword), ("String", Type), ("\"Ana\"", String), ("42", Number)].into_iter().map(|(t, k)| (t.to_owned(), k)).collect();
        assert_eq!(all(r#"let name: String = "Ana" + 42"#, Swift), expected);
        assert_eq!(texts("@MainActor final class A {}\n#if DEBUG", Swift, Keyword), ["@MainActor", "final", "class", "#if"]);
        let code = "let a = 1 // let b\n/* var\n c */ var d";
        assert_eq!(texts(code, Swift, Comment), ["// let b", "/* var\n c */"]);
        assert_eq!(texts(code, Swift, Keyword), ["let", "var"]);
    }

    #[test]
    fn strings() {
        let code = "let s = \"\"\"\nlet \"x\"\n\"\"\"\nlet t";
        assert_eq!(texts(code, Swift, String), ["\"\"\"\nlet \"x\"\n\"\"\""]);
        assert_eq!(texts(code, Swift, Keyword), ["let", "let"]);
        assert_eq!(texts(r#"print("a \" b") // c"#, Swift, String), [r#""a \" b""#]);
        let open = "let a = \"abierta\nlet b";
        assert_eq!(texts(open, Swift, String), ["\"abierta"]);
        assert_eq!(texts(open, Swift, Keyword), ["let", "let"]);
    }

    #[test]
    fn words_and_numbers() {
        assert!(texts("letter format1 x2", Swift, Keyword).is_empty());
        assert!(texts("letter format1 x2", Swift, Number).is_empty());
        assert_eq!(texts("a = 3.14 + 0xFF + 1_000 + 1e-3", Swift, Number), ["3.14", "0xFF", "1_000", "1e-3"]);
    }

    #[test]
    fn python() {
        let code = "@cache\ndef f(x: int) -> None:\n    return f\"{x}\"  # fin";
        assert_eq!(texts(code, Python, Attribute), ["@cache"]);
        assert_eq!(texts(code, Python, Keyword), ["def", "None", "return"]);
        assert_eq!(texts(code, Python, Type), ["int"]);
        assert_eq!(texts(code, Python, String), ["f\"{x}\""]);
        assert_eq!(texts(code, Python, Comment), ["# fin"]);
        assert_eq!(texts("'''uno\ndos'''\nx", Python, String), ["'''uno\ndos'''"]);
    }

    #[test]
    fn javascript_and_typescript() {
        let code = "const $el = `a\nb`; // x";
        assert_eq!(texts(code, JavaScript, Keyword), ["const"]);
        assert_eq!(texts(code, JavaScript, String), ["`a\nb`"]);
        assert_eq!(texts(code, JavaScript, Comment), ["// x"]);
        let ts = "interface A { b: number }";
        assert_eq!(texts(ts, TypeScript, Keyword), ["interface"]);
        assert_eq!(texts(ts, TypeScript, Type), ["A", "number"]);
        assert!(texts(ts, JavaScript, Keyword).is_empty());
    }

    #[test]
    fn json() {
        let code = r#"{"name": "Ana", "age": 3, "ok": true, "x": null}"#;
        assert_eq!(texts(code, Json, Property), [r#""name""#, r#""age""#, r#""ok""#, r#""x""#]);
        assert_eq!(texts(code, Json, String), [r#""Ana""#]);
        assert_eq!(texts(code, Json, Number), ["3"]);
        assert_eq!(texts(code, Json, Keyword), ["true", "null"]);
    }

    #[test]
    fn css() {
        let code = "/* c */\n.box { color: #fff; margin-top: -4px !important; }\n@media print {}";
        assert_eq!(texts(code, Css, Comment), ["/* c */"]);
        assert_eq!(texts(code, Css, Property), ["color", "margin-top"]);
        assert_eq!(texts(code, Css, Number), ["#fff", "4px"]);
        assert_eq!(texts(code, Css, Keyword), ["important", "@media"]);
    }

    #[test]
    fn sql_is_case_insensitive() {
        let code = "SELECT id FROM users WHERE name = 'O''Neil' -- fin";
        assert_eq!(texts(code, Sql, Keyword), ["SELECT", "FROM", "WHERE"]);
        assert_eq!(texts(code, Sql, Comment), ["-- fin"]);
        assert_eq!(texts(code, Sql, String), ["'O'", "'Neil'"]);
    }

    #[test]
    fn shell() {
        let code = "export PATH=\"$HOME/bin\" # ruta\nif [ -n ${name} ]; then echo $1; fi\nurl#frag";
        assert_eq!(texts(code, Shell, Keyword), ["export", "if", "then", "echo", "fi"]);
        assert_eq!(texts(code, Shell, Comment), ["# ruta"]);
        assert_eq!(texts(code, Shell, Property), ["${name}", "$1"]);
        assert_eq!(texts(code, Shell, String), ["\"$HOME/bin\""]);
    }

    #[test]
    fn html() {
        let code = "<!-- c -->\n<a href=\"/x\" hidden>Hola</a>\n<br/>";
        let expected: Vec<(std::string::String, TokenKind)> = [
            ("<!-- c -->", Comment),
            ("<a", Tag),
            ("href", Attribute),
            ("\"/x\"", String),
            ("hidden", Attribute),
            (">", Tag),
            ("</a", Tag),
            (">", Tag),
            ("<br", Tag),
            ("/>", Tag),
        ]
        .into_iter()
        .map(|(t, k)| (t.to_owned(), k))
        .collect();
        assert_eq!(all(code, Html), expected);
        assert!(all("a < b y c > d", Html).is_empty());
    }

    #[test]
    fn ranges_count_characters() {
        let string = tokens("let 👋 = \"ñ😀\"", Swift).into_iter().find(|t| t.kind == String).unwrap();
        assert_eq!(string.range, 8..12);
    }

    #[test]
    fn tokens_dont_overlap_and_stay_inside() {
        for (code, language) in [
            ("func a() { \"x\" /* y */ 1 }", Swift),
            ("<p class='a'>t</p>", Html),
            ("SELECT 1; -- x", Sql),
            ("a=\"$(b)\" # c", Shell),
            ("\"unclosed", Json),
            ("/* unclosed", Css),
        ] {
            let len = code.chars().count();
            let mut end = 0;
            for t in tokens(code, language) {
                assert!(t.range.start >= end && t.range.end <= len, "{code}");
                end = t.range.end;
            }
        }
    }
}
