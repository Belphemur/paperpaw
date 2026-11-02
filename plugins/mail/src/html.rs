//! A simple HTML mail as text: tags go, blocks and line breaks become line
//! breaks, list items get a bullet, `<script>`, `<style>` and `<head>` go
//! with what is in them, entities are text. It comes in pieces (a tag or
//! an entity may be cut anywhere) and takes any bytes.

use alloc::string::String;

/// The state of the text made so far.
#[derive(Debug, Clone, Default)]
pub struct HtmlText {
    state: State,
    tag: String,
    entity: String,
    /// The end tag that closes what is skipped (`</style>`).
    skipping: Option<&'static str>,
    /// Characters of the text made since the last line break, and
    /// whether the last one was white space (it folds into one space).
    column: usize,
    space: bool,
    breaks: u8,
    /// Whether a quotation mark `"` opened a comment `<!--`.
    comment: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum State {
    #[default]
    Text,
    Tag,
    Entity,
}

/// Elements whose end or start is a line break.
const BLOCKS: [&str; 22] = [
    "p", "div", "br", "tr", "li", "ul", "ol", "table", "h1", "h2", "h3", "h4", "h5", "h6",
    "blockquote", "pre", "hr", "section", "article", "header", "footer", "dl",
];

impl HtmlText {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends the text of `html` to `out`.
    pub fn push(&mut self, html: &str, out: &mut String) {
        for c in html.chars() {
            self.char(c, out);
        }
    }

    /// Ends the text (an open tag is dropped, an open entity is text).
    pub fn finish(&mut self, out: &mut String) {
        if self.state == State::Entity {
            let entity = core::mem::take(&mut self.entity);
            self.text('&', out);
            for c in entity.chars() {
                self.text(c, out);
            }
        }
        self.state = State::Text;
    }

    fn char(&mut self, c: char, out: &mut String) {
        match self.state {
            State::Tag => {
                if self.tag.len() < 1024 {
                    self.tag.push(c);
                }
                // `<!-- … -->` may hold a `>`.
                if self.tag.starts_with("!--") && !(c == '>' && self.tag.ends_with("-->")) {
                    return;
                }
                if c == '>' {
                    let tag = core::mem::take(&mut self.tag);
                    self.state = State::Text;
                    self.tag_done(&tag, out);
                }
            }
            State::Entity => {
                if c == ';' {
                    let entity = core::mem::take(&mut self.entity);
                    self.state = State::Text;
                    match entity_char(&entity) {
                        Some(decoded) => self.text(decoded, out),
                        None => {
                            self.text('&', out);
                            for c in entity.chars() {
                                self.text(c, out);
                            }
                            self.text(';', out);
                        }
                    }
                } else if (c.is_ascii_alphanumeric() || c == '#') && self.entity.len() < 10 {
                    self.entity.push(c);
                } else {
                    // Not an entity: the text it was.
                    let entity = core::mem::take(&mut self.entity);
                    self.state = State::Text;
                    self.text('&', out);
                    for e in entity.chars() {
                        self.text(e, out);
                    }
                    self.char(c, out);
                }
            }
            State::Text => {
                if self.skipping.is_some() {
                    self.skip(c);
                } else if c == '<' {
                    self.state = State::Tag;
                    self.tag.clear();
                } else if c == '&' {
                    self.state = State::Entity;
                    self.entity.clear();
                } else {
                    self.text(c, out);
                }
            }
        }
    }

    /// Inside `<style>`/`<script>`/`<head>`: watches for the end tag.
    fn skip(&mut self, c: char) {
        let Some(end) = self.skipping else {
            return;
        };
        self.tag.push(c.to_ascii_lowercase());
        if self.tag.len() > end.len() {
            let cut = self.tag.len() - end.len();
            self.tag.drain(..cut);
        }
        if self.tag == end {
            self.skipping = None;
            self.tag.clear();
        }
    }

    fn tag_done(&mut self, tag: &str, out: &mut String) {
        self.comment = false;
        let body = tag.trim_end_matches('>').trim();
        let (closing, body) = match body.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, body),
        };
        let mut name = String::new();
        name.extend(
            body.chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase()),
        );
        if !closing {
            let end = match name.as_str() {
                "script" => Some("</script>"),
                "style" => Some("</style>"),
                "head" => Some("</head>"),
                _ => None,
            };
            if end.is_some() && !body.ends_with('/') {
                self.skipping = end;
                self.tag.clear();
                return;
            }
        }
        if name == "li" && !closing {
            self.line_break(out, 1);
            self.raw("• ", out);
            return;
        }
        if BLOCKS.contains(&name.as_str()) {
            let lines = if matches!(name.as_str(), "br" | "li" | "tr") { 1 } else { 2 };
            self.line_break(out, lines);
        }
    }

    /// A line break of `lines` (at most two empty lines in a row come out).
    fn line_break(&mut self, out: &mut String, lines: u8) {
        if out.is_empty() && self.column == 0 {
            return;
        }
        let have = self.breaks;
        for _ in have..lines.min(2) {
            out.push('\n');
        }
        self.breaks = self.breaks.max(lines.min(2));
        self.column = 0;
        self.space = false;
    }

    fn raw(&mut self, text: &str, out: &mut String) {
        out.push_str(text);
        self.column += text.chars().count();
        self.breaks = 0;
        self.space = false;
    }

    /// A character of the text: white space folds into one space.
    fn text(&mut self, c: char, out: &mut String) {
        if c == '\u{a0}' {
            self.raw(" ", out);
            return;
        }
        if c.is_whitespace() || c == '\u{200b}' {
            if !self.space && self.column > 0 {
                out.push(' ');
                self.column += 1;
                self.space = true;
            }
            return;
        }
        if c.is_control() {
            return;
        }
        out.push(c);
        self.column += 1;
        self.space = false;
        self.breaks = 0;
    }
}

/// The character of an entity's name (`amp`, `#39`, `#x1F600`).
fn entity_char(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let value = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(value).filter(|c| *c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "bull" => '•',
        "euro" => '€',
        "copy" => '©',
        "reg" => '®',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "szlig" => 'ß',
        "eacute" => 'é',
        "egrave" => 'è',
        "agrave" => 'à',
        "ccedil" => 'ç',
        _ => return None,
    })
}

/// The text of a whole `html`.
pub fn to_text(html: &str) -> String {
    let mut parser = HtmlText::new();
    let mut out = String::new();
    parser.push(html, &mut out);
    parser.finish(&mut out);
    String::from(out.trim_end())
}
