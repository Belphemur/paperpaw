//! A sample of text set as the reading settings say: the preview above
//! them on their settings page (a widget that previews the reading
//! group). It follows the values shown while one is being chosen, so the
//! text size, the spacing, the margins, the font, justification and
//! hyphenation show at once, before they are kept.
//!
//! Laid out by the same engine and type as book pages, from two short
//! paragraphs in the language of the interface, as wide as a page with
//! its margins. Laying out happens when the settings change
//! ([`Widget::update`]); drawing only draws the lines kept.

use paperpaw_sdk::Storage;
use paperpaw_sdk::document::{Align, Block, BlockKind, Metadata, OffsetMap};
use paperpaw_sdk::widgets::Widget;

use super::*;

/// The share of the screen's height the sample takes: a few lines, and
/// room for the settings below it.
const SHARE: u16 = 5;

/// The sample: its type and lines, laid out for the values shown.
pub(crate) struct Sample {
    typography: Option<Typography>,
    lines: Vec<PageLine>,
    /// What it was laid out for: the type's key and the text.
    shown: String,
}

impl Sample {
    pub(crate) fn new() -> Self {
        Self {
            typography: None,
            lines: Vec::new(),
            shown: String::new(),
        }
    }
}

/// How high the sample is: a fifth of the screen.
fn height(env: &Env<'_>) -> u16 {
    env.device().display.height / SHARE
}

/// The sample's text: its paragraphs, one to a line.
fn text(env: &Env<'_>) -> String {
    format!(
        "{}\n{}",
        env.i18n().tr("reader.sample.1"),
        env.i18n().tr("reader.sample.2")
    )
}

impl Widget for Sample {
    fn update(&mut self, env: &Env<'_>) -> bool {
        let Some(font) = env.fonts().role(fonts::READING.as_str()) else {
            return false;
        };
        let language = env.i18n().language();
        let pad = env.metrics().mm(2) as f32;
        let t = Typography::sample(env, &font, language, f32::from(height(env)), pad);
        let text = text(env);
        // How the letters are inked too, which the type's key leaves out.
        let s = env.settings();
        let ink = (
            t.weight,
            s.read(&crate::settings::TEXT_CONTRAST),
            s.read(&crate::settings::TEXT_SHARPNESS),
            s.read(&crate::settings::SMOOTH_TEXT),
        );
        let shown = format!("{}|{ink:?}|{text}", t.key);
        if shown == self.shown {
            return false;
        }
        let Some(faces) = Faces::new(env, &t, None) else {
            return false;
        };
        let patterns = t.patterns(env);
        let hyphenate = |word: &str, out: &mut dyn FnMut(usize)| {
            if let Some(patterns) = patterns {
                patterns.hyphenate(word, out);
            }
        };
        let document = SampleText {
            text,
            metadata: Metadata::default(),
        };
        // Through the text filters, as books are.
        let filtered = FilteredDocument::new(&document, env);
        // In the frame chosen, as book pages are.
        let page = t
            .lay_out(Locator::new(0, 0), |width, height, at| {
                let cx = LayoutContext::new(&filtered, env.storage(), width, &t.style, &faces)
                    .hyphenate(patterns.is_some().then_some(&hyphenate as Hyphenate<'_>));
                layout_page(&cx, at, height)
            })
            .unwrap_or_default();
        self.lines = page.lines;
        self.typography = Some(t);
        self.shown = shown;
        true
    }

    /// Only settings change it.
    fn live(&self) -> bool {
        false
    }

    fn height(&self, env: &Env<'_>, _: u16) -> u16 {
        height(env)
    }

    fn draw(&self, canvas: &mut Canvas<'_>, env: &Env<'_>, area: Rect) {
        let Some(t) = &self.typography else {
            return;
        };
        let Some(faces) = Faces::new(env, t, None) else {
            return;
        };
        let origin = (f32::from(area.x) + t.x, f32::from(area.y) + t.y);
        let mut clipped = canvas.clipped(area);
        paperpaw_sdk::reading::draw_lines(&mut clipped, &faces, t, origin, &self.lines, &[]);
    }
}

/// The sample's text as a document: a section of paragraphs, one to a
/// line.
struct SampleText {
    text: String,
    metadata: Metadata,
}

impl Document for SampleText {
    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn sections(&self) -> u32 {
        1
    }

    fn section_len(&self, _: u32) -> u32 {
        self.text.len() as u32
    }

    fn block_at(&self, _: &dyn Storage, at: Locator) -> Result<Option<Block>, DocumentError> {
        let offset = (at.offset as usize).min(self.text.len());
        if at.section > 0 || offset >= self.text.len() {
            return Ok(None);
        }
        let start = self.text[..offset].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[offset..]
            .find('\n')
            .map_or(self.text.len(), |i| offset + i + 1);
        Ok(Some(
            Block::new(
                BlockKind::Paragraph,
                start as u32,
                end as u32,
                self.text[start..end].trim_end_matches('\n').to_string(),
            )
            .map(OffsetMap::IDENTITY)
            .align(Align::Auto),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::NoStorage;

    #[test]
    fn paragraphs_are_blocks_one_after_the_other() {
        let text = SampleText {
            text: String::from("One two.\nThree."),
            metadata: Metadata::default(),
        };
        let first = text.block_at(&NoStorage, Locator::new(0, 3)).unwrap();
        let first = first.expect("the first paragraph");
        assert_eq!(
            (first.text.as_str(), first.start, first.end),
            ("One two.", 0, 9)
        );
        let second = text.block_at(&NoStorage, Locator::new(0, 9)).unwrap();
        let second = second.expect("the second paragraph");
        assert_eq!((second.text.as_str(), second.start), ("Three.", 9));
        assert!(
            text.block_at(&NoStorage, Locator::new(0, 15))
                .unwrap()
                .is_none()
        );
        assert!(
            text.block_at(&NoStorage, Locator::new(1, 0))
                .unwrap()
                .is_none()
        );
    }
}
