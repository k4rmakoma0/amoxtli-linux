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
    Ruby,
    JavaScript,
    TypeScript,
    Json,
    Html,
    Css,
    Sql,
    Shell,
    Rust,
    Go,
    C,
    Cpp,
    Java,
    Kotlin,
    Php,
    Lua,
    Yaml,
    Toml,
}

impl Language {
    pub const ALL: [Language; 20] = [
        Language::Swift,
        Language::Python,
        Language::Ruby,
        Language::JavaScript,
        Language::TypeScript,
        Language::Json,
        Language::Html,
        Language::Css,
        Language::Sql,
        Language::Shell,
        Language::Rust,
        Language::Go,
        Language::C,
        Language::Cpp,
        Language::Java,
        Language::Kotlin,
        Language::Php,
        Language::Lua,
        Language::Yaml,
        Language::Toml,
    ];

    /// The first word of a fence's info string, as in ```` ```swift ````, with common aliases.
    /// Anything else is a language without highlighting.
    pub fn from_info(info: &str) -> Option<Language> {
        Some(match first_word(info).to_lowercase().as_str() {
            "swift" => Language::Swift,
            "python" | "py" | "python3" => Language::Python,
            "ruby" | "rb" => Language::Ruby,
            "javascript" | "js" | "jsx" | "mjs" | "cjs" | "node" => Language::JavaScript,
            "typescript" | "ts" | "tsx" => Language::TypeScript,
            "json" | "jsonc" | "json5" => Language::Json,
            "html" | "htm" | "xml" | "xhtml" | "svg" | "plist" => Language::Html,
            "css" | "scss" | "less" => Language::Css,
            "sql" | "mysql" | "postgresql" | "postgres" | "sqlite" => Language::Sql,
            "shell" | "sh" | "bash" | "zsh" | "console" | "shellscript" => Language::Shell,
            "rust" | "rs" => Language::Rust,
            "go" | "golang" => Language::Go,
            "c" | "h" => Language::C,
            "cpp" | "c++" | "cxx" | "cc" | "hpp" | "hxx" => Language::Cpp,
            "java" => Language::Java,
            "kotlin" | "kt" | "kts" => Language::Kotlin,
            "php" => Language::Php,
            "lua" => Language::Lua,
            "yaml" | "yml" => Language::Yaml,
            "toml" => Language::Toml,
            _ => return None,
        })
    }

    /// How the language is named in menus and on the block: proper names, not translated.
    pub fn title(self) -> &'static str {
        match self {
            Language::Swift => "Swift",
            Language::Python => "Python",
            Language::Ruby => "Ruby",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Json => "JSON",
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Sql => "SQL",
            Language::Shell => "Shell",
            Language::Rust => "Rust",
            Language::Go => "Go",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Java => "Java",
            Language::Kotlin => "Kotlin",
            Language::Php => "PHP",
            Language::Lua => "Lua",
            Language::Yaml => "YAML",
            Language::Toml => "TOML",
        }
    }

    /// What is written after the fence.
    pub fn fence_name(self) -> &'static str {
        match self {
            Language::Swift => "swift",
            Language::Python => "python",
            Language::Ruby => "ruby",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Json => "json",
            Language::Html => "html",
            Language::Css => "css",
            Language::Sql => "sql",
            Language::Shell => "shell",
            Language::Rust => "rust",
            Language::Go => "go",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::Php => "php",
            Language::Lua => "lua",
            Language::Yaml => "yaml",
            Language::Toml => "toml",
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
    /// Object/configuration keys, CSS properties and variables.
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
    special: Option<Language>,
    nested_comments: bool,
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
            special: None,
            nested_comments: false,
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
    let c_like = || Syntax {
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        strings: &[("\"", false, true), ("'", false, true)],
        ..Default::default()
    };
    vec![
        (Language::Rust, Syntax {
            keywords: set(&[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
                "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
                "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
                "use", "where", "while", "yield",
            ]),
            types: set(&["bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64"]),
            capitalized_are_types: true,
            string_prefixes: &["b", "c"],
            nested_comments: true,
            special: Some(Language::Rust),
            ..c_like()
        }),
        (Language::Go, Syntax {
            strings: &[("`", true, false), ("\"", false, true), ("'", false, true)],
            keywords: set(&[
                "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for",
                "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select",
                "struct", "switch", "type", "var", "true", "false", "nil", "iota",
            ]),
            types: set(&["any", "bool", "byte", "complex64", "complex128", "error", "float32", "float64", "int", "int8", "int16", "int32", "int64", "rune", "string", "uint", "uint8", "uint16", "uint32", "uint64", "uintptr"]),
            ..c_like()
        }),
        (Language::C, Syntax {
            keywords: set(&[
                "auto", "break", "case", "const", "constexpr", "continue", "default", "do", "else", "enum", "extern",
                "false", "for", "goto", "if", "inline", "nullptr", "register", "restrict", "return", "sizeof", "static",
                "static_assert", "struct", "switch", "thread_local", "true", "typedef", "typeof", "union", "volatile", "while",
                "_Alignas", "_Alignof", "_Atomic", "_Generic", "_Noreturn", "_Static_assert", "_Thread_local",
            ]),
            types: set(&["bool", "_Bool", "char", "double", "float", "int", "long", "short", "signed", "unsigned", "void", "size_t"]),
            keyword_prefixes: &['#'],
            ..c_like()
        }),
        (Language::Cpp, Syntax {
            keywords: set(&[
                "alignas", "alignof", "and", "asm", "auto", "bitand", "bitor", "break", "case", "catch", "class",
                "co_await", "co_return", "co_yield", "concept", "const", "consteval", "constexpr", "constinit",
                "const_cast", "continue", "decltype", "default", "delete", "do", "dynamic_cast", "else", "enum",
                "explicit", "export", "extern", "false", "for", "friend", "goto", "if", "inline", "mutable", "namespace",
                "new", "noexcept", "not", "nullptr", "operator", "or", "private", "protected", "public", "register",
                "reinterpret_cast", "requires", "return", "sizeof", "static", "static_assert", "static_cast", "struct",
                "switch", "template", "this", "thread_local", "throw", "true", "try", "typedef", "typeid", "typename",
                "union", "using", "virtual", "volatile", "while", "xor",
            ]),
            types: set(&["bool", "char", "char8_t", "char16_t", "char32_t", "double", "float", "int", "long", "short", "signed", "unsigned", "void", "wchar_t", "size_t", "string", "vector"]),
            keyword_prefixes: &['#'],
            capitalized_are_types: true,
            ..c_like()
        }),
        (Language::Java, Syntax {
            strings: &[("\"\"\"", true, true), ("\"", false, true), ("'", false, true)],
            keywords: set(&[
                "abstract", "assert", "break", "case", "catch", "class", "const", "continue", "default", "do", "else",
                "enum", "exports", "extends", "false", "final", "finally", "for", "if", "implements", "import",
                "instanceof", "interface", "module", "native", "new", "null", "opens", "package", "permits", "private",
                "protected", "public", "record", "requires", "return", "sealed", "static", "strictfp", "super",
                "switch", "synchronized", "this", "throw", "throws", "transient", "true", "try", "var", "volatile", "while", "yield",
            ]),
            types: set(&["boolean", "byte", "char", "double", "float", "int", "long", "short", "void"]),
            capitalized_are_types: true,
            decorator_prefix: Some('@'),
            ..c_like()
        }),
        (Language::Kotlin, Syntax {
            strings: &[("\"\"\"", true, false), ("\"", false, true), ("'", false, true)],
            keywords: set(&[
                "abstract", "actual", "annotation", "as", "break", "by", "catch", "class", "companion", "const",
                "constructor", "continue", "crossinline", "data", "delegate", "do", "dynamic", "else", "enum",
                "expect", "external", "false", "field", "file", "final", "finally", "for", "fun", "get", "if",
                "import", "in", "infix", "init", "inline", "inner", "interface", "internal", "is", "lateinit",
                "noinline", "null", "object", "open", "operator", "out", "override", "package", "param", "private",
                "property", "protected", "public", "receiver", "reified", "return", "sealed", "set", "setparam",
                "super", "suspend", "tailrec", "this", "throw", "true", "try", "typealias", "val", "var", "vararg", "when", "where", "while",
            ]),
            capitalized_are_types: true,
            decorator_prefix: Some('@'),
            nested_comments: true,
            ..c_like()
        }),
        (Language::Php, Syntax {
            line_comments: &["//", "#"],
            strings: &[("\"", true, true), ("'", true, true), ("`", true, true)],
            keywords: set(&[
                "abstract", "and", "array", "as", "break", "callable", "case", "catch", "class", "clone", "const",
                "continue", "declare", "default", "die", "do", "echo", "else", "elseif", "empty", "enddeclare",
                "endfor", "endforeach", "endif", "endswitch", "endwhile", "enum", "eval", "exit", "extends", "false",
                "final", "finally", "fn", "for", "foreach", "function", "global", "goto", "if", "implements", "include",
                "include_once", "instanceof", "insteadof", "interface", "isset", "list", "match", "namespace", "new",
                "null", "or", "print", "private", "protected", "public", "readonly", "require", "require_once", "return",
                "self", "parent", "static", "switch", "throw", "trait", "true", "try", "unset", "use", "var", "while", "xor", "yield",
            ]),
            types: set(&["bool", "float", "int", "iterable", "mixed", "never", "object", "string", "void"]),
            case_insensitive: true,
            capitalized_are_types: true,
            shell_variables: true,
            ..c_like()
        }),
        (Language::Lua, Syntax {
            line_comments: &["--"],
            strings: &[("\"", false, true), ("'", false, true)],
            keywords: set(&["and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"]),
            special: Some(Language::Lua),
            ..Default::default()
        }),
        (Language::Yaml, Syntax {
            line_comments: &["#"],
            strings: &[("\"", true, true), ("'", true, false)],
            keywords: set(&["true", "false", "null", "~"]),
            case_insensitive: true,
            comment_needs_boundary: true,
            special: Some(Language::Yaml),
            ..Default::default()
        }),
        (Language::Toml, Syntax {
            line_comments: &["#"],
            strings: &[("\"\"\"", true, true), ("'''", true, false), ("\"", false, true), ("'", false, false)],
            keywords: set(&["true", "false", "inf", "nan"]),
            special: Some(Language::Toml),
            ..Default::default()
        }),
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
        (Language::Ruby, Syntax {
            line_comments: &["#"],
            strings: &[("\"", true, true), ("'", true, true), ("`", true, true)],
            keywords: set(&[
                "BEGIN", "END", "__ENCODING__", "__FILE__", "__LINE__", "alias", "and", "begin", "break",
                "case", "class", "def", "defined?", "do", "else", "elsif", "end", "ensure", "false", "for",
                "if", "in", "module", "next", "nil", "not", "or", "redo", "rescue", "retry", "return",
                "self", "super", "then", "true", "undef", "unless", "until", "when", "while", "yield",
            ]),
            capitalized_are_types: true,
            special: Some(Language::Ruby),
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
            if self.special_token() || self.comment() || self.string(None) || self.number() || self.word() || self.variable() {
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

    fn special_token(&mut self) -> bool {
        match self.syntax.special {
            Some(Language::Ruby) => self.ruby_token(),
            Some(Language::Rust) => self.rust_token(),
            Some(Language::Lua) => self.lua_token(),
            Some(Language::Yaml | Language::Toml) => self.config_token(),
            _ => false,
        }
    }

    /// Raw Rust strings have a matching number of hashes. Lifetimes are not strings.
    fn rust_token(&mut self) -> bool {
        let start = self.pos;
        let mut at = start;
        if matches!(self.c.get(at), Some('b' | 'c')) {
            at += 1;
        }
        if self.c.get(at) == Some(&'r') {
            at += 1;
            let hashes = at;
            while self.c.get(at) == Some(&'#') {
                at += 1;
            }
            if self.c.get(at) == Some(&'"') {
                let close = format!("\"{}", "#".repeat(at - hashes));
                self.pos = at + 1;
                while self.pos < self.c.len() && !self.matches(&close, self.pos) {
                    self.pos += 1;
                }
                self.pos = (self.pos + close.len()).min(self.c.len());
                self.add(TokenKind::String, start);
                return true;
            }
        }
        if self.c[start] == '\'' && self.c.get(start + 1).is_some_and(|c| is_letter(*c) || *c == '_') {
            let mut end = start + 2;
            while self.c.get(end).is_some_and(|c| self.is_word(*c)) {
                end += 1;
            }
            if self.c.get(end) != Some(&'\'') {
                self.pos = end;
                self.add(TokenKind::Attribute, start);
                return true;
            }
        }
        false
    }

    /// Lua long strings and comments can use any number of equals signs: [=[...]=].
    fn lua_token(&mut self) -> bool {
        let start = self.pos;
        let comment = self.matches("--", start);
        let open = start + if comment { 2 } else { 0 };
        if self.c.get(open) != Some(&'[') {
            return false;
        }
        let mut at = open + 1;
        while self.c.get(at) == Some(&'=') {
            at += 1;
        }
        if self.c.get(at) != Some(&'[') {
            return false;
        }
        let close = format!("]{}]", "=".repeat(at - open - 1));
        self.pos = at + 1;
        while self.pos < self.c.len() && !self.matches(&close, self.pos) {
            self.pos += 1;
        }
        self.pos = (self.pos + close.len()).min(self.c.len());
        self.add(if comment { TokenKind::Comment } else { TokenKind::String }, start);
        true
    }

    /// Configuration keys are properties, including quoted and dotted TOML keys.
    fn config_token(&mut self) -> bool {
        let start = self.pos;
        let toml = self.syntax.special == Some(Language::Toml);
        let separator = if toml { '=' } else { ':' };
        if self.c[start].is_whitespace()
            || (!toml && self.c[start] == '-' && self.c.get(start + 1).is_some_and(|c| c.is_whitespace()))
        {
            return false;
        }
        let line_start = self.c[..start].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
        let at_line_start = self.c[line_start..start].iter().all(|c| matches!(c, ' ' | '\t'));
        let previous = self.c[..start].iter().rfind(|c| !matches!(c, ' ' | '\t')).copied();
        if !toml && matches!(self.c[start], '|' | '>') && matches!(previous, Some(':' | '-')) {
            let indent = self.c[line_start..].iter().take_while(|c| matches!(c, ' ' | '\t')).count();
            let mut end = start;
            while end < self.c.len() && self.c[end] != '\n' {
                end += 1;
            }
            while end < self.c.len() {
                let next_line = end + 1;
                let mut next_end = next_line;
                while next_end < self.c.len() && self.c[next_end] != '\n' {
                    next_end += 1;
                }
                let spaces = self.c[next_line..next_end].iter().take_while(|c| matches!(c, ' ' | '\t')).count();
                let blank = self.c[next_line..next_end].iter().all(|c| c.is_whitespace());
                if !blank && spaces <= indent {
                    break;
                }
                end = next_end;
            }
            self.pos = end;
            self.add(TokenKind::String, start);
            return true;
        }
        if toml && at_line_start && self.c[start] == '[' {
            let close = if self.matches("[[", start) { "]]" } else { "]" };
            let mut at = start + close.len();
            let mut quote = None;
            while at < self.c.len() && self.c[at] != '\n' {
                let c = self.c[at];
                if quote == Some('"') && c == '\\' {
                    at += 2;
                    continue;
                }
                if quote == Some(c) {
                    quote = None;
                } else if quote.is_none() {
                    if matches!(c, '\'' | '"') {
                        quote = Some(c);
                    } else if self.matches(close, at) {
                        self.pos = at + close.len();
                        self.add(TokenKind::Tag, start);
                        return true;
                    }
                }
                at += 1;
            }
        }
        if !toml && self.c[start] == '~' {
            self.pos += 1;
            self.add(TokenKind::Keyword, start);
            return true;
        }
        // Only look for keys where a mapping entry can begin, never inside a value.
        if !(at_line_start || matches!(previous, Some('{' | ',')) || (!toml && previous == Some('-'))) {
            return false;
        }
        let mut at = start;
        let mut quote = None;
        let mut key_end = start;
        while at < self.c.len() && self.c[at] != '\n' {
            let c = self.c[at];
            if let Some(q) = quote {
                if q == '"' && c == '\\' {
                    at = (at + 2).min(self.c.len());
                    key_end = at;
                    continue;
                }
                if c == q {
                    quote = None;
                }
            } else if matches!(c, '\'' | '"') {
                quote = Some(c);
            } else if c == separator {
                let yaml_boundary = self.c.get(at + 1).is_none_or(|c| c.is_whitespace() || [',', '}', ']'].contains(c));
                if key_end > start && (toml || yaml_boundary) {
                    self.pos = key_end;
                    self.add(TokenKind::Property, start);
                    return true;
                }
                return false;
            } else if !(self.is_word(c) || ['-', '.', ' ', '\t'].contains(&c)) {
                return false;
            }
            at += 1;
            if !matches!(c, ' ' | '\t') {
                key_end = at;
            }
        }
        false
    }

    /// Ruby symbols, instance/class/global variables, and line-delimited block comments.
    fn ruby_token(&mut self) -> bool {
        if self.syntax.special != Some(Language::Ruby) {
            return false;
        }
        let start = self.pos;
        let chars = self.c;
        let line_marker = |at: usize, marker: &str| {
            (at == 0 || chars[at - 1] == '\n')
                && marker.chars().enumerate().all(|(i, ch)| chars.get(at + i) == Some(&ch))
                && chars.get(at + marker.len()).is_none_or(|c| c.is_whitespace())
        };
        if line_marker(start, "=begin") {
            self.pos += 6;
            while self.pos < self.c.len() && !line_marker(self.pos, "=end") {
                self.pos += 1;
            }
            if self.pos < self.c.len() {
                while self.pos < self.c.len() && self.c[self.pos] != '\n' {
                    self.pos += 1;
                }
            }
            self.add(TokenKind::Comment, start);
            return true;
        }
        let prefix = self.c[start];
        if ![':', '@', '$'].contains(&prefix)
            || (prefix == ':' && start > 0 && self.c[start - 1] == ':')
        {
            return false;
        }
        let mut end = start + 1;
        if prefix == '@' && self.c.get(end) == Some(&'@') {
            end += 1;
        }
        if prefix == ':' && matches!(self.c.get(end), Some('\"' | '\'')) {
            self.pos = end;
            return self.string(Some(start));
        }
        if !self.c.get(end).is_some_and(|c| is_letter(*c) || *c == '_') {
            return false;
        }
        self.pos = end + 1;
        while self.pos < self.c.len() && self.is_word(self.c[self.pos]) {
            self.pos += 1;
        }
        if prefix == ':' && matches!(self.c.get(self.pos), Some('?' | '!')) {
            self.pos += 1;
        }
        self.add(if prefix == ':' { TokenKind::String } else { TokenKind::Property }, start);
        true
    }

    fn comment(&mut self) -> bool {
        let start = self.pos;
        if let Some((open, close)) = self.syntax.block_comment {
            if self.matches(open, self.pos) {
                self.pos += open.chars().count();
                let mut depth = 1;
                while self.pos < self.c.len() {
                    if self.matches(close, self.pos) {
                        self.pos += close.chars().count();
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if self.syntax.nested_comments && self.matches(open, self.pos) {
                        depth += 1;
                        self.pos += open.chars().count();
                    } else {
                        self.pos += 1;
                    }
                }
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
        if self.syntax.special == Some(Language::Ruby) && matches!(self.c.get(self.pos), Some('?' | '!')) {
            self.pos += 1;
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
        assert_eq!(Language::from_info("RB title=\"a.rb\""), Some(Ruby));
        assert_eq!(Language::from_info("bash"), Some(Shell));
        assert_eq!(Language::from_info("zsh"), Some(Shell));
        assert_eq!(Language::from_info("xml"), Some(Html));
        assert_eq!(Language::from_info("json{1,3}"), Some(Json));
        assert_eq!(Language::from_info(""), None);
        assert_eq!(Language::from_info("cobol"), None);
        for l in Language::ALL {
            assert_eq!(Language::from_info(l.fence_name()), Some(l));
        }
        assert_eq!(label(Some("rust ignore")).as_deref(), Some("Rust"));
        assert_eq!(label(Some("ts")).as_deref(), Some("TypeScript"));
        assert_eq!(label(None), None);
        let titles: Vec<&str> = Language::by_title().into_iter().map(Language::title).collect();
        assert_eq!(titles, ["C", "C++", "CSS", "Go", "HTML", "Java", "JavaScript", "JSON", "Kotlin", "Lua", "PHP", "Python", "Ruby", "Rust", "Shell", "SQL", "Swift", "TOML", "TypeScript", "YAML"]);
    }

    #[test]
    fn new_language_aliases() {
        for (info, language) in [
            ("RS", Rust), ("golang", Go), ("h", C), ("c++", Cpp), ("cxx", Cpp), ("cc", Cpp),
            ("hpp", Cpp), ("hxx", Cpp), ("kt", Kotlin), ("kts", Kotlin), ("yml", Yaml),
        ] {
            assert_eq!(Language::from_info(info), Some(language));
            assert_eq!(label(Some(info)).as_deref(), Some(language.title()));
        }
    }

    #[test]
    fn rust_raw_strings_lifetimes_and_nested_comments() {
        let code = "pub fn name<'a>(s: &'a str) -> char { let x = r##\"a \"# b\"##; 'ñ' }";
        assert_eq!(texts(code, Rust, Keyword), ["pub", "fn", "let"]);
        assert_eq!(texts(code, Rust, Type), ["str", "char"]);
        assert_eq!(texts(code, Rust, Attribute), ["'a", "'a"]);
        assert_eq!(texts(code, Rust, String), ["r##\"a \"# b\"##", "'ñ'"]);
        assert_eq!(texts("br#\"raw\"# b\"bytes\" c\"cstr\"", Rust, String), ["br#\"raw\"#", "b\"bytes\"", "c\"cstr\""]);
        assert_eq!(texts("/* outer /* inner */ end */ let n = 42;", Rust, Comment), ["/* outer /* inner */ end */"]);
        assert_eq!(texts("/* outer /* inner */ end */ let n = 42;", Rust, Keyword), ["let"]);
    }

    #[test]
    fn go() {
        let code = "package main\nfunc main() { var n int = 42; s := `raw\\\n\"text\"`; r := 'ñ' } // fin";
        assert_eq!(texts(code, Go, Keyword), ["package", "func", "var"]);
        assert_eq!(texts(code, Go, Type), ["int"]);
        assert_eq!(texts(code, Go, Number), ["42"]);
        assert_eq!(texts(code, Go, String), ["`raw\\\n\"text\"`", "'ñ'"]);
        assert_eq!(texts(code, Go, Comment), ["// fin"]);
    }

    #[test]
    fn c_and_cpp() {
        let code = "#include <stdio.h>\nint main(void) { const char *s = \"hola\"; return 0; } /* fin */";
        for language in [C, Cpp] {
            assert_eq!(texts(code, language, Keyword), ["#include", "const", "return"]);
            assert_eq!(texts(code, language, Type), ["int", "void", "char"]);
            assert_eq!(texts(code, language, String), ["\"hola\""]);
            assert_eq!(texts(code, language, Number), ["0"]);
            assert_eq!(texts(code, language, Comment), ["/* fin */"]);
        }
        let cpp = "template <typename T> class Box { public: std::vector<T> items; };";
        assert_eq!(texts(cpp, Cpp, Keyword), ["template", "typename", "class", "public"]);
        assert_eq!(texts(cpp, Cpp, Type), ["T", "Box", "vector", "T"]);
        assert!(texts(cpp, C, Keyword).is_empty());
    }

    #[test]
    fn java_and_kotlin() {
        let java = "@Override public String name() { return \"Ana\"; } // fin";
        assert_eq!(texts(java, Java, Attribute), ["@Override"]);
        assert_eq!(texts(java, Java, Keyword), ["public", "return"]);
        assert_eq!(texts(java, Java, Type), ["String"]);
        assert_eq!(texts(java, Java, String), ["\"Ana\""]);
        let kotlin = "@Composable data class User(val name: String) { suspend fun greet() = \"Hola\" }";
        assert_eq!(texts(kotlin, Kotlin, Attribute), ["@Composable"]);
        assert_eq!(texts(kotlin, Kotlin, Keyword), ["data", "class", "val", "suspend", "fun"]);
        assert_eq!(texts(kotlin, Kotlin, Type), ["User", "String"]);
        for language in [Java, Kotlin] {
            assert_eq!(texts("\"\"\"uno\ndos\"\"\"", language, String), ["\"\"\"uno\ndos\"\"\""]);
        }
        assert_eq!(texts("/* a /* b */ c */ val n = 1", Kotlin, Comment), ["/* a /* b */ c */"]);
    }

    #[test]
    fn php() {
        let code = "<?php FUNCTION greet(string $name): void { echo \"Hola\"; $this->name = $name; } # fin";
        assert_eq!(texts(code, Php, Keyword), ["FUNCTION", "echo"]);
        assert_eq!(texts(code, Php, Type), ["string", "void"]);
        assert_eq!(texts(code, Php, Property), ["$name", "$this", "$name"]);
        assert_eq!(texts(code, Php, String), ["\"Hola\""]);
        assert_eq!(texts(code, Php, Comment), ["# fin"]);
    }

    #[test]
    fn lua_long_strings_and_comments() {
        let code = "local s = [=[uno ]]\n'dos']=]\n--[==[ comment ]=]\nend ]==]\nreturn 42 -- fin";
        assert_eq!(texts(code, Lua, Keyword), ["local", "return"]);
        assert_eq!(texts(code, Lua, String), ["[=[uno ]]\n'dos']=]"]);
        assert_eq!(texts(code, Lua, Comment), ["--[==[ comment ]=]\nend ]==]", "-- fin"]);
        assert_eq!(texts(code, Lua, Number), ["42"]);
        assert_eq!(texts("[[hola]] '\\'x'", Lua, String), ["[[hola]]", "'\\'x'"]);
    }

    #[test]
    fn yaml_keys_values_and_block_scalars() {
        let code = "service-name: \"amoxtli\"\nenabled: true # fin\nitems:\n  - name: 'Ana'\n    count: 42\nempty: null\nother: ~";
        assert_eq!(texts(code, Yaml, Property), ["service-name", "enabled", "items", "name", "count", "empty", "other"]);
        assert_eq!(texts(code, Yaml, String), ["\"amoxtli\"", "'Ana'"]);
        assert_eq!(texts(code, Yaml, Keyword), ["true", "null", "~"]);
        assert_eq!(texts(code, Yaml, Number), ["42"]);
        assert_eq!(texts(code, Yaml, Comment), ["# fin"]);
        assert_eq!(texts("message: |-\n  true # text\n  'hello'\nnext: false", Yaml, String), ["|-\n  true # text\n  'hello'"]);
        assert_eq!(texts("message: >\n  true\nnext: false", Yaml, Keyword), ["false"]);
        assert!(texts("url: https://example.com/#fragment", Yaml, Comment).is_empty());
        assert_eq!(texts("{\"name\": Ana, ok: true}", Yaml, Property), ["\"name\"", "ok"]);
    }

    #[test]
    fn toml_keys_tables_and_multiline_strings() {
        let code = "[server.http]\nlisten-port = 8080 # fin\nenabled = true\n'db'.host = 'localhost'\n[[products]]\nname = \"Ana\"";
        assert_eq!(texts(code, Toml, Tag), ["[server.http]", "[[products]]"]);
        assert_eq!(texts(code, Toml, Property), ["listen-port", "enabled", "'db'.host", "name"]);
        assert_eq!(texts(code, Toml, String), ["'localhost'", "\"Ana\""]);
        assert_eq!(texts(code, Toml, Keyword), ["true"]);
        assert_eq!(texts(code, Toml, Number), ["8080"]);
        assert_eq!(texts(code, Toml, Comment), ["# fin"]);
        assert_eq!(texts("text = '''uno\ndos'''", Toml, String), ["'''uno\ndos'''"]);
        assert_eq!(texts("x = { name = \"Ana\", age = 3 }", Toml, Property), ["x", "name", "age"]);
        assert_eq!(texts("[\"a]b\"]\nx = 1", Toml, Tag), ["[\"a]b\"]"]);
    }

    #[test]
    fn every_language_handles_incomplete_and_unicode_input() {
        for language in Language::ALL {
            for code in ["", "ñ 😀 42", "\"unclosed", "/* a /* b", "r###\"raw", "--[==[", "[\"table", "key: |\n  ñ\nnext: true", "x = { name = 'ñ' }", ":", "$", "'a"] {
                let mut end = 0;
                for token in tokens(code, language) {
                    assert!(token.range.start >= end && token.range.end <= code.chars().count(), "{language:?}: {code}");
                    assert!(token.range.start < token.range.end);
                    end = token.range.end;
                }
            }
        }
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
    fn ruby() {
        let code = "class Saludo < Object\n  def initialize(nombre)\n    @nombre = nombre # saludo\n    @@total = 42\n    $saludo = :hola\n    return \"Hola #{@nombre}\" if defined?(@nombre)\n  end\nend";
        assert_eq!(texts(code, Ruby, Keyword), ["class", "def", "return", "if", "defined?", "end", "end"]);
        assert_eq!(texts(code, Ruby, Type), ["Saludo", "Object"]);
        assert_eq!(texts(code, Ruby, Property), ["@nombre", "@@total", "$saludo", "@nombre"]);
        assert_eq!(texts(code, Ruby, String), [":hola", "\"Hola #{@nombre}\""]);
        assert_eq!(texts(code, Ruby, Number), ["42"]);
        assert_eq!(texts(code, Ruby, Comment), ["# saludo"]);
        assert_eq!(texts("'uno\ndos' :\"hola mundo\" File::Path :ready?", Ruby, String),
            ["'uno\ndos'", ":\"hola mundo\"", ":ready?"]);
    }

    #[test]
    fn ruby_block_comments_require_line_markers() {
        let code = "=begin docs\nclass Ignored\n=endings\n=end docs\nclass Visible\nend";
        assert_eq!(texts(code, Ruby, Comment), ["=begin docs\nclass Ignored\n=endings\n=end docs"]);
        assert_eq!(texts(code, Ruby, Keyword), ["class", "end"]);
        assert!(texts("x =begin\n  =begin\nclass Visible", Ruby, Comment).is_empty());
        assert_eq!(texts("=begin\nunclosed", Ruby, Comment), ["=begin\nunclosed"]);
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
            ("class Niño; @nombre = :hola; end # fin", Ruby),
            ("=begin\nSin cerrar", Ruby),
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
