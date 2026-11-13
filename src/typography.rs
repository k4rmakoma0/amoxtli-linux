//! The typeface and text size people chose. Global: they apply to every window, and to the
//! source and the formatted document alike. Headings scale with body text (their tags use
//! relative scales) and code stays monospaced whatever the typeface.

use crate::i18n::tr;

/// The typefaces people can choose from: the system's own fonts, so every language the
/// system supports is covered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadingFont {
    /// The interface font.
    System,
    Serif,
    Monospace,
    /// Linux has no standard rounded family: the first one installed from `ROUNDED`.
    Rounded,
}

/// Rounded families, in order of preference. Without any of them, Rounded isn't offered.
pub const ROUNDED: &[&str] = &["Nunito", "Varela Round", "M PLUS Rounded 1c", "Quicksand", "Comfortaa"];

impl ReadingFont {
    pub const ALL: [ReadingFont; 4] = [Self::System, Self::Serif, Self::Monospace, Self::Rounded];

    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Serif => "serif",
            Self::Monospace => "monospace",
            Self::Rounded => "rounded",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }

    pub fn label(self) -> String {
        match self {
            Self::System => tr("System"),
            Self::Serif => tr("Serif"),
            Self::Monospace => tr("Monospace"),
            Self::Rounded => tr("Rounded"),
        }
    }

    /// The CSS family; `None` keeps the interface font. `installed` lists the font families
    /// on the system, for Rounded.
    pub fn family(self, installed: &[String]) -> Option<String> {
        match self {
            Self::System => None,
            Self::Serif => Some("serif".into()),
            Self::Monospace => Some("monospace".into()),
            Self::Rounded => rounded_family(installed),
        }
    }
}

pub fn rounded_family(installed: &[String]) -> Option<String> {
    ROUNDED
        .iter()
        .find(|want| installed.iter().any(|f| f.eq_ignore_ascii_case(want)))
        .map(|f| (*f).to_owned())
}

/// The size of body text, in steps like Make Text Bigger in other apps. Steps are the macOS
/// app's point sizes; 13, its body size, is the system's font size here, so the others are
/// relative to it (10 is 77 %, 32 is 246 %).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSize(i32);

impl TextSize {
    pub const STEPS: [i32; 13] = [10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32];
    pub const STANDARD: TextSize = TextSize(13);

    /// Stored values come back as the closest step, so an edited or outdated preference
    /// can't produce a size the app doesn't offer.
    pub fn nearest(points: i32) -> Self {
        let step = Self::STEPS.iter().min_by_key(|s| (**s - points).abs()).copied().unwrap_or(13);
        Self(step)
    }

    /// The size closest to a zoom percentage of Amoxtli 1.x.
    pub fn from_zoom(percent: i32) -> Self {
        Self::nearest((percent * Self::STANDARD.0 + 50) / 100)
    }

    pub fn points(self) -> i32 {
        self.0
    }

    /// How much bigger than the system's font size.
    pub fn scale(self) -> f64 {
        self.0 as f64 / Self::STANDARD.0 as f64
    }

    pub fn percent(self) -> i32 {
        (self.scale() * 100.0).round() as i32
    }

    /// `None` at the largest step.
    pub fn larger(self) -> Option<Self> {
        Self::STEPS.iter().find(|s| **s > self.0).map(|s| Self(*s))
    }

    /// `None` at the smallest step.
    pub fn smaller(self) -> Option<Self> {
        Self::STEPS.iter().rev().find(|s| **s < self.0).map(|s| Self(*s))
    }
}

/// The style sheet for both views (`.md-document`), sized relative to the inherited font.
pub fn css(family: Option<&str>, size: TextSize) -> String {
    let family = family.map(|f| format!(" font-family: \"{}\";", f.replace(['"', '\\'], ""))).unwrap_or_default();
    format!(".md-document {{ font-size: {:.4}em;{family} }}\n", size.scale())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_move_through_steps() {
        let standard = TextSize::STANDARD;
        assert_eq!(standard.larger(), Some(TextSize(14)));
        assert_eq!(standard.smaller(), Some(TextSize(12)));
        assert_eq!(TextSize(16).larger(), Some(TextSize(18)));
        assert_eq!(TextSize(32).larger(), None);
        assert_eq!(TextSize(10).smaller(), None);
        assert_eq!(standard.percent(), 100);
    }

    #[test]
    fn stored_values_snap_to_the_closest_step() {
        assert_eq!(TextSize::nearest(17), TextSize(16));
        assert_eq!(TextSize::nearest(19), TextSize(18));
        assert_eq!(TextSize::nearest(3), TextSize(10));
        assert_eq!(TextSize::nearest(99), TextSize(32));
    }

    #[test]
    fn old_zoom_becomes_a_size() {
        assert_eq!(TextSize::from_zoom(100), TextSize::STANDARD);
        assert_eq!(TextSize::from_zoom(150), TextSize(20));
        assert_eq!(TextSize::from_zoom(50), TextSize(10));
        assert_eq!(TextSize::from_zoom(300), TextSize(32));
    }

    #[test]
    fn fonts() {
        let installed = vec!["Noto Sans".to_owned(), "quicksand".to_owned(), "Nunito".to_owned()];
        assert_eq!(ReadingFont::System.family(&installed), None);
        assert_eq!(ReadingFont::Serif.family(&installed).as_deref(), Some("serif"));
        assert_eq!(ReadingFont::Rounded.family(&installed).as_deref(), Some("Nunito"));
        assert_eq!(ReadingFont::Rounded.family(&["Noto Sans".to_owned()]), None);
        assert_eq!(ReadingFont::from_name("monospace"), Some(ReadingFont::Monospace));
        assert_eq!(ReadingFont::from_name("comic"), None);
    }

    #[test]
    fn style_sheet() {
        assert_eq!(css(None, TextSize::STANDARD), ".md-document { font-size: 1.0000em; }\n");
        assert_eq!(css(Some("serif"), TextSize(26 / 2)), ".md-document { font-size: 1.0000em; font-family: \"serif\"; }\n");
        assert!(css(Some("x\"y"), TextSize(20)).contains("1.5385em; font-family: \"xy\""));
    }
}
