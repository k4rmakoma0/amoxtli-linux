//! Helpers about the document that both editors and the window share: headings, word counts,
//! anchors, alert colors and image decoding.

use crate::i18n::tr;
use crate::markdown::parse;
use gtk::prelude::*;
use gtk::{gdk, glib};
use pulldown_cmark::{BlockQuoteKind, Event, Parser, Tag, TagEnd};

#[derive(Clone, Debug)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub slug: String,
    pub mark: gtk::TextMark,
}

/// Where each heading starts in the source, in characters, in the order the formatted
/// editor lists them, so the source editor can go to the one picked in the Contents panel.
/// Headings inside footnote definitions don't count: the formatted editor keeps those as written.
pub fn heading_offsets(source: &str) -> Vec<i32> {
    let mut offsets = vec![];
    let (mut byte, mut chars) = (0, 0);
    let mut footnote = 0;
    for (event, range) in Parser::new_ext(source, parse::options()).into_offset_iter() {
        match event {
            Event::Start(Tag::FootnoteDefinition(_)) => footnote += 1,
            Event::End(TagEnd::FootnoteDefinition) => footnote -= 1,
            Event::Start(Tag::Heading { .. }) if footnote == 0 => {
                chars += source[byte..range.start].chars().count();
                byte = range.start;
                offsets.push(chars as i32);
            }
            _ => {}
        }
    }
    offsets
}

pub fn alert_color(kind: BlockQuoteKind, dark: bool) -> gdk::RGBA {
    let hex = match (kind, dark) {
        (BlockQuoteKind::Note, false) => "#0969da",
        (BlockQuoteKind::Note, true) => "#4493f8",
        (BlockQuoteKind::Tip, false) => "#1a7f37",
        (BlockQuoteKind::Tip, true) => "#3fb950",
        (BlockQuoteKind::Important, false) => "#8250df",
        (BlockQuoteKind::Important, true) => "#ab7df8",
        (BlockQuoteKind::Warning, false) => "#9a6700",
        (BlockQuoteKind::Warning, true) => "#d29922",
        (BlockQuoteKind::Caution, false) => "#cf222e",
        (BlockQuoteKind::Caution, true) => "#f85149",
    };
    gdk::RGBA::parse(hex).unwrap()
}

pub fn alert_title(kind: BlockQuoteKind) -> String {
    match kind {
        BlockQuoteKind::Note => format!("ⓘ  {}", tr("Note")),
        BlockQuoteKind::Tip => format!("💡  {}", tr("Tip")),
        BlockQuoteKind::Important => format!("❗  {}", tr("Important")),
        BlockQuoteKind::Warning => format!("⚠  {}", tr("Warning")),
        BlockQuoteKind::Caution => format!("⛔  {}", tr("Caution")),
    }
}

/// Count words: runs of letters/digits, allowing inner apostrophes and hyphens.
pub fn count_words(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut count = 0;
    let mut in_word = false;
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphanumeric() || c == '_' {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else if in_word && matches!(c, '\'' | '’' | '-') && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric()) {
            // joiner inside a word: stay in the word
        } else {
            in_word = false;
        }
    }
    count
}

pub fn slugify(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

pub fn percent_decode(s: &str) -> String {
    glib::Uri::unescape_string(s, None::<&str>).map(|g| g.to_string()).unwrap_or_else(|| s.to_owned())
}

pub fn texture_from_bytes(bytes: &glib::Bytes) -> Option<gdk::Paintable> {
    if let Ok(t) = gdk::Texture::from_bytes(bytes) {
        return Some(t.upcast());
    }
    // SVG, GIF, WebP… go through gdk-pixbuf loaders
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write_bytes(bytes).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    #[allow(deprecated)]
    Some(gdk::Texture::for_pixbuf(&pixbuf).upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(count_words("Hello, world! It's a well-known fact — 42 times."), 8);
        assert_eq!(count_words("  "), 0);
        assert_eq!(count_words("año ñandú"), 2);
    }

    #[test]
    fn heading_offsets_skip_code_and_footnotes() {
        let text = "# Uno\n\ntexto ñ\n\n```\n# no\n```\n## Dos\nSetext\n---";
        assert_eq!(heading_offsets(text), vec![0, 29, 36]);
        assert_eq!(heading_offsets("a[^1]\n\n[^1]: note\n\n    # inside\n\n# Out\n").len(), 1);
    }

    #[test]
    fn heading_offsets_match_the_formatted_editor() {
        let text = "# A\n\n> ## B\n\n- ### C\n\n[^1]: # D\n\nE\n=\n";
        let doc = parse::parse(text);
        let headings = doc.paragraphs.iter().filter(|p| p.kind.block.heading_level() > 0).count();
        assert_eq!(heading_offsets(text).len(), headings);
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("Instalación rápida"), "instalación-rápida");
    }
}
