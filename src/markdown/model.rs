//! The document as the formatted editor holds it: a flat list of paragraphs, each one knowing
//! what it is and what it sits inside. A text view edits paragraphs, not trees, so this is the
//! shape that survives typing, and it has all that's needed to write the Markdown back.
//!
//! What the editor can't show as formatting is kept word for word (`Block::Raw` lines and
//! `InlineStyle::RAW` runs): HTML blocks, footnote definitions, images inside links… so
//! editing a document in the formatted view never drops any of it.

use std::rc::Rc;

/// Inline styles, combinable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InlineStyle(pub u8);

impl InlineStyle {
    pub const NONE: InlineStyle = InlineStyle(0);
    pub const EMPHASIS: InlineStyle = InlineStyle(1);
    pub const STRONG: InlineStyle = InlineStyle(2);
    pub const CODE: InlineStyle = InlineStyle(4);
    pub const STRIKETHROUGH: InlineStyle = InlineStyle(8);
    /// Markdown has no underline: it is written as `<u>text</u>`, as most editors do.
    pub const UNDERLINE: InlineStyle = InlineStyle(16);
    /// Markdown kept as written, such as inline HTML or a footnote reference.
    pub const RAW: InlineStyle = InlineStyle(32);

    pub fn contains(self, other: InlineStyle) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn union(self, other: InlineStyle) -> InlineStyle {
        InlineStyle(self.0 | other.0)
    }

    pub fn intersection(self, other: InlineStyle) -> InlineStyle {
        InlineStyle(self.0 & other.0)
    }

    pub fn without(self, other: InlineStyle) -> InlineStyle {
        InlineStyle(self.0 & !other.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    pub destination: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InlineRun {
    pub text: String,
    pub style: InlineStyle,
    pub link: Option<Rc<Link>>,
}

impl InlineRun {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: InlineStyle::NONE, link: None }
    }

    pub fn styled(text: impl Into<String>, style: InlineStyle) -> Self {
        Self { text: text.into(), style, link: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Alert {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Alert {
    pub fn tag(self) -> &'static str {
        match self {
            Alert::Note => "NOTE",
            Alert::Tip => "TIP",
            Alert::Important => "IMPORTANT",
            Alert::Warning => "WARNING",
            Alert::Caution => "CAUTION",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Image {
    pub source: String,
    pub alt: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TableCell {
    pub table_id: u32,
    /// Row 0 is the header.
    pub row: usize,
    pub column: usize,
    pub alignments: Rc<[Alignment]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Block {
    Paragraph,
    /// `attributes` is written after the text as is, such as `{#intro}`.
    Heading { level: u8, attributes: Option<String> },
    /// One paragraph per line of code. Lines with the same `id` are one block; `info` is
    /// what follows the opening fence, `None` for indented code.
    Code { id: u32, info: Option<String> },
    ThematicBreak,
    Image(Image),
    TableCell(TableCell),
    /// Markdown the editor keeps as written, one paragraph per line. Lines with the same
    /// `id` are one block.
    Raw { id: u32 },
}

impl Block {
    pub fn is_text(&self) -> bool {
        matches!(self, Block::Paragraph | Block::Heading { .. })
    }

    pub fn is_code(&self) -> bool {
        matches!(self, Block::Code { .. })
    }

    pub fn heading_level(&self) -> u8 {
        match self {
            Block::Heading { level, .. } => *level,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListItem {
    pub list_id: u32,
    /// Paragraphs with the same `item_id` belong to one item; a new one starts a new item.
    pub item_id: u32,
    pub ordered: bool,
    /// The number of the list's first item.
    pub start: u64,
    /// `Some(done)` for a task list item.
    pub task: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    Quote { id: u32, alert: Option<Alert> },
    ListItem(ListItem),
}

impl Container {
    pub fn list_item(&self) -> Option<&ListItem> {
        match self {
            Container::ListItem(item) => Some(item),
            Container::Quote { .. } => None,
        }
    }

    pub fn is_quote(&self) -> bool {
        matches!(self, Container::Quote { .. })
    }
}

/// What a paragraph is and what it sits inside, outermost container first: a list item in a
/// quote is `[Quote, ListItem]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Kind {
    pub block: Block,
    pub containers: Vec<Container>,
}

impl Kind {
    pub fn body() -> Self {
        Self { block: Block::Paragraph, containers: vec![] }
    }

    pub fn with_block(&self, block: Block) -> Self {
        Self { block, containers: self.containers.clone() }
    }

    pub fn with_containers(&self, containers: Vec<Container>) -> Self {
        Self { block: self.block.clone(), containers }
    }

    pub fn is_quoted(&self) -> bool {
        self.containers.iter().any(Container::is_quote)
    }

    pub fn last_item(&self) -> Option<&ListItem> {
        self.containers.last().and_then(Container::list_item)
    }

    pub fn table_cell(&self) -> Option<&TableCell> {
        match &self.block {
            Block::TableCell(cell) => Some(cell),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Paragraph {
    pub kind: Kind,
    /// A hard line break is a `"\n"` inside a run.
    pub runs: Vec<InlineRun>,
}

impl Paragraph {
    pub fn new(block: Block, containers: Vec<Container>, runs: Vec<InlineRun>) -> Self {
        Self { kind: Kind { block, containers }, runs }
    }

    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    /// YAML or TOML front matter, delimiters included, kept as written and not shown.
    pub front_matter: Option<String>,
    pub paragraphs: Vec<Paragraph>,
}
