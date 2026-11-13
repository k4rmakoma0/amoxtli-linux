//! Lines up the scroll positions of the two editors of the split view. The source and the
//! formatted document have very different heights (a table, an image or a code block take more
//! or less room on each side), so a percentage would drift. Instead, each top-level block is a
//! pair of offsets, one per side, and positions between two blocks are interpolated.

use super::model::{Block, Container, Kind};
use super::parse;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub source: f64,
    pub formatted: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollMap {
    /// Increasing on both sides, from the top of both documents to the end of both.
    pub points: Vec<Point>,
}

impl ScrollMap {
    /// The anchors are where each block starts on each side, as the scroll offset that puts it
    /// at the top. They pair up only when both sides have the same blocks; otherwise the map
    /// only aligns the tops and the ends. The ends are each side's largest scroll offset.
    pub fn new(source: &[f64], formatted: &[f64], source_end: f64, formatted_end: f64) -> Self {
        let mut points = vec![Point { source: 0.0, formatted: 0.0 }];
        if source.len() == formatted.len() {
            for (&s, &f) in source.iter().zip(formatted) {
                let last = *points.last().unwrap();
                // Near the end a block can't reach the top, so the end takes over there.
                if s > last.source && f > last.formatted && s < source_end && f < formatted_end {
                    points.push(Point { source: s, formatted: f });
                }
            }
        }
        let last = *points.last().unwrap();
        if source_end > last.source && formatted_end > last.formatted {
            points.push(Point { source: source_end, formatted: formatted_end });
        }
        Self { points }
    }

    pub fn formatted_for_source(&self, offset: f64) -> f64 {
        Self::map(offset, &self.points, |p| p.source, |p| p.formatted)
    }

    pub fn source_for_formatted(&self, offset: f64) -> f64 {
        Self::map(offset, &self.points, |p| p.formatted, |p| p.source)
    }

    fn map(offset: f64, points: &[Point], input: impl Fn(&Point) -> f64, output: impl Fn(&Point) -> f64) -> f64 {
        if points.len() < 2 {
            return 0.0;
        }
        let (first, last) = (&points[0], &points[points.len() - 1]);
        if offset <= input(first) {
            return output(first);
        }
        if offset >= input(last) {
            return output(last);
        }
        // the first point past the offset; the one before it is at or before the offset
        let i = points.partition_point(|p| input(p) <= offset).max(1);
        let (start, end) = (&points[i - 1], &points[i]);
        let fraction = (offset - input(start)) / (input(end) - input(start));
        output(start) + fraction * (output(end) - output(start))
    }
}

/// Where each top-level block starts in Markdown source, in characters. A paragraph of only
/// images counts each image, as the formatted editor shows each as a block of its own. Front
/// matter isn't shown there, so it doesn't count.
pub fn source_block_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![];
    let mut depth = 0;
    let mut images: Vec<usize> = vec![];
    let mut only_images = true;
    let mut paragraph_start = 0;
    let mut front_matter_seen = false;
    for (event, range) in Parser::new_ext(source, parse::options()).into_offset_iter() {
        match &event {
            Event::Start(tag) => {
                if depth == 0 {
                    match tag {
                        // front matter opens the document and isn't shown; one further down is
                        // kept as written, a block like any other
                        Tag::MetadataBlock(_) if starts.is_empty() && !front_matter_seen => front_matter_seen = true,
                        Tag::Paragraph => {
                            paragraph_start = range.start;
                            images.clear();
                            only_images = true;
                        }
                        _ => starts.push(range.start),
                    }
                }
                if depth == 1 && matches!(tag, Tag::Image { .. }) {
                    images.push(range.start);
                } else if depth == 1 && !matches!(tag, Tag::Image { .. }) {
                    only_images = false;
                }
                depth += 1;
            }
            Event::End(tag) => {
                depth -= 1;
                if depth == 0 && matches!(tag, TagEnd::Paragraph) {
                    if only_images && !images.is_empty() {
                        starts.extend(&images);
                    } else {
                        starts.push(paragraph_start);
                    }
                }
            }
            Event::Text(t) if depth == 1 => only_images &= t.trim().is_empty(),
            Event::SoftBreak if depth == 1 => {}
            Event::Rule if depth == 0 => starts.push(range.start),
            Event::Html(_) | Event::InlineHtml(_) | Event::Code(_) | Event::HardBreak | Event::FootnoteReference(_)
                if depth == 1 =>
            {
                only_images = false
            }
            _ => {}
        }
    }
    // byte offsets to characters, in one pass
    let mut chars = vec![];
    let (mut byte, mut count) = (0, 0);
    for start in starts {
        count += source[byte..start].chars().count();
        byte = start;
        chars.push(count);
    }
    chars
}

/// Which lines of the formatted editor start a top-level block: the lines of one list, quote,
/// code block, raw block or table are one block; any other line is a block of its own. An
/// empty line outside everything writes no Markdown, so it starts nothing.
pub fn formatted_block_starts(lines: &[(Kind, bool)]) -> Vec<usize> {
    #[derive(PartialEq)]
    enum Group {
        Quote(u32),
        List(u32),
        Code(u32),
        Raw(u32),
        Table(u32),
    }
    let group = |kind: &Kind| match kind.containers.first() {
        Some(Container::Quote { id, .. }) => Some(Group::Quote(*id)),
        Some(Container::ListItem(item)) => Some(Group::List(item.list_id)),
        None => match &kind.block {
            Block::Code { id, .. } => Some(Group::Code(*id)),
            Block::Raw { id } => Some(Group::Raw(*id)),
            Block::TableCell(cell) => Some(Group::Table(cell.table_id)),
            _ => None,
        },
    };
    let mut starts = vec![];
    let mut previous: Option<Group> = None;
    for (i, (kind, empty)) in lines.iter().enumerate() {
        if *empty && kind.block == Block::Paragraph && kind.containers.is_empty() {
            previous = None;
            continue;
        }
        let g = group(kind);
        if g.is_none() || g != previous {
            starts.push(i);
        }
        previous = g;
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> ScrollMap {
        ScrollMap::new(&[100.0, 200.0], &[300.0, 400.0], 1000.0, 2000.0)
    }

    #[test]
    fn interpolates_between_blocks() {
        let m = map();
        for (s, f) in [(0.0, 0.0), (50.0, 150.0), (100.0, 300.0), (150.0, 350.0), (600.0, 1200.0), (1000.0, 2000.0)] {
            assert_eq!(m.formatted_for_source(s), f, "at {s}");
        }
    }

    #[test]
    fn maps_both_ways() {
        let m = map();
        let mut offset = 0.0;
        while offset <= 1000.0 {
            assert!((m.source_for_formatted(m.formatted_for_source(offset)) - offset).abs() < 1e-4);
            offset += 25.0;
        }
    }

    #[test]
    fn clamps_outside_the_document() {
        let m = ScrollMap::new(&[100.0], &[300.0], 1000.0, 2000.0);
        assert_eq!(m.formatted_for_source(-40.0), 0.0);
        assert_eq!(m.formatted_for_source(5000.0), 2000.0);
    }

    #[test]
    fn different_block_counts_align_only_tops_and_ends() {
        let m = ScrollMap::new(&[100.0, 200.0], &[300.0], 1000.0, 2000.0);
        assert_eq!(m.points.len(), 2);
        assert_eq!(m.formatted_for_source(500.0), 1000.0);
    }

    #[test]
    fn blocks_past_the_end_give_way_to_it() {
        let m = ScrollMap::new(&[100.0, 900.0, 950.0], &[300.0, 1500.0, 2100.0], 920.0, 2000.0);
        assert_eq!(m.points.iter().map(|p| p.source).collect::<Vec<_>>(), [0.0, 100.0, 900.0, 920.0]);
        assert_eq!(m.formatted_for_source(920.0), 2000.0);
    }

    #[test]
    fn skips_blocks_that_dont_move_forward() {
        let m = ScrollMap::new(&[100.0, 100.0, 200.0, 300.0], &[300.0, 350.0, 290.0, 400.0], 1000.0, 2000.0);
        assert_eq!(m.points.iter().map(|p| p.source).collect::<Vec<_>>(), [0.0, 100.0, 300.0, 1000.0]);
    }

    #[test]
    fn content_that_fits_doesnt_scroll() {
        let m = ScrollMap::new(&[10.0], &[20.0], 0.0, 0.0);
        assert_eq!(m.formatted_for_source(0.0), 0.0);
        assert_eq!(m.source_for_formatted(30.0), 0.0);
    }

    fn formatted_starts(markdown: &str) -> usize {
        super::tests_formatted_starts(markdown)
    }
}

/// The formatted editor's block count for a text, building its lines as the editor does.
#[cfg(test)]
pub(crate) fn tests_formatted_starts(markdown: &str) -> usize {
    {
        let doc = parse::parse(markdown);
        let mut lines: Vec<(Kind, bool)> = vec![];
        let mut previous_table = None;
        for p in &doc.paragraphs {
            // a table is one line in the editor
            if let Some(cell) = p.kind.table_cell() {
                if previous_table == Some(cell.table_id) {
                    continue;
                }
                previous_table = Some(cell.table_id);
            } else {
                previous_table = None;
            }
            lines.push((p.kind.clone(), p.runs.is_empty()));
        }
        formatted_block_starts(&lines).len()
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    fn formatted_starts(markdown: &str) -> usize {
        tests_formatted_starts(markdown)
    }

    #[test]
    fn both_sides_count_the_same_blocks() {
        let documents = [
            "",
            "Just text",
            "# A\n## B\n### C",
            "# Título\n\nUn párrafo\nde dos líneas.\n\n- uno\n- dos\n  - anidado\n\n> cita\n>\n> - con lista\n\n```swift\nlet x = 1\n\nprint(x)\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n---\n\n1. último\n",
            "- a\n\n- b\n\n\n1. c\n2. d",
            "> a\n\n> b",
            "Before\n![alt](a.png)\nafter\n\nOther",
            "![a](1.png)\n![b](2.png)\n\ntext",
            "---\ntitle: x\n---\n\n# Doc\n\n<div>\nraw\n</div>\n\nnote[^1]\n\n[^1]: The note.\n",
            "> [!NOTE]\n> alert\n\n```\ncode\n```\n```\nmore\n```\n",
        ];
        for doc in documents {
            assert_eq!(source_block_starts(doc).len(), formatted_starts(doc), "{doc:?}");
        }
    }

    #[test]
    fn source_starts_count_characters() {
        assert_eq!(source_block_starts("ñ\n\n# b"), vec![0, 3]);
        assert_eq!(source_block_starts("---\nt: 1\n---\n\ntext"), vec![14]);
    }
}

#[cfg(test)]
mod corpus {
    /// Real documents count the same blocks on both sides. `AMOXTLI_CORPUS=dir` adds a folder.
    #[test]
    fn block_counts_match_in_real_documents() {
        let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples"))];
        if let Ok(dir) = std::env::var("AMOXTLI_CORPUS") {
            dirs.push(dir.into());
        }
        let mut files = vec![];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "md") {
                    files.push(path);
                }
            }
        }
        let mismatched: Vec<String> = files
            .iter()
            .filter_map(|f| {
                let text = std::fs::read_to_string(f).ok()?;
                let (s, r) = (super::source_block_starts(&text).len(), super::tests_formatted_starts(&text));
                (s != r).then(|| format!("{}: source {s}, formatted {r}", f.display()))
            })
            .collect();
        assert!(mismatched.is_empty(), "{} of {}:\n{}", mismatched.len(), files.len(), mismatched.join("\n"));
    }
}
