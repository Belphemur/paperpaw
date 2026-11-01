//! Drawing the page: its text and pictures, highlights, links, the
//! plugins' layers, the bookmark ribbon, the way back from a link and the
//! status lines.

use super::*;

use paperpaw_sdk::layers::{LayerItem, LineBox, PageView};

impl Book {
    /// The lines of the page as layers see them, made once a page.
    pub(super) fn lines(&self) -> &[LineBox] {
        self.lines.get_or_init(|| self.line_boxes())
    }

    /// The lines of the page as layers see them: each as wide as its
    /// column, as high as a line of body text, on its baseline. A drop
    /// capital (a line on the baseline of a line after it) is part of the
    /// line beside it, which starts with it.
    fn line_boxes(&self) -> Vec<LineBox> {
        let t = &self.typography;
        let text = Rect::new(t.x as u16, t.y as u16, t.width as u16, t.height as u16);
        let lines = &self.page.lines;
        let capital = |i: usize| is_capital(lines, i);
        lines
            .iter()
            .enumerate()
            .filter(|&(i, _)| !capital(i))
            .filter_map(|(i, line)| {
                let first = match i.checked_sub(1).filter(|&before| capital(before)) {
                    Some(before) => lines[before].words.first()?.0,
                    None => line.words.first()?.0,
                };
                let last = line.words.last()?.1;
                let x = (t.x + line.x) as u32;
                let column = t
                    .columns
                    .iter()
                    .find(|c| u32::from(c.x) <= x && x < c.right())
                    .copied()
                    .unwrap_or(text);
                let top = (t.y + line.baseline - t.ascent).max(0.0);
                Some(
                    LineBox::new(
                        Rect::new(column.x, top as u16, column.width, t.line_height as u16),
                        Locator::new(line.section, first),
                        Locator::new(line.section, last),
                    )
                    .leading(line.leading as u16),
                )
            })
            .collect()
    }

    /// The page as layers see it, with its `lines`.
    pub(super) fn page_view<'a>(&'a self, lines: &'a [LineBox]) -> PageView<'a> {
        let t = &self.typography;
        let text = Rect::new(t.x as u16, t.y as u16, t.width as u16, t.height as u16);
        PageView::new(&self.path, self.page.start, self.page.end, text)
            .language(&self.language)
            .columns(&t.columns)
            .lines(lines)
            .status(t.top_status, t.status)
    }

    /// The top of the text and the layers' rooms above it: the margin
    /// above is above that.
    fn content_top(&self) -> f32 {
        let t = &self.typography;
        t.rooms
            .iter()
            .map(|(_, room)| f32::from(room.y))
            .fold(t.y, f32::min)
    }

    /// The right edge of the text and the layers' rooms beside it.
    fn right_edge(&self) -> f32 {
        let t = &self.typography;
        t.rooms
            .iter()
            .map(|(_, room)| room.right() as f32)
            .fold(t.x + t.width, f32::max)
    }

    /// The page as the layers that are on draw it, the words given where
    /// one asks for them.
    pub(super) fn layers_view<'a>(&'a self, env: &Env<'_>) -> PageView<'a> {
        let words = match env.page_layers().want_words(env) {
            true => self.words(env),
            false => &[],
        };
        self.page_view(self.lines()).words(words)
    }

    /// The items the layers that are on drew on the page, those on top
    /// first.
    pub(super) fn layer_items(&self, env: &Env<'_>) -> Vec<(Id, LayerItem)> {
        if env.page_layers().is_empty() {
            return Vec::new();
        }
        env.page_layers().items(env, &self.layers_view(env))
    }

    /// Tells the plugin of layer `layer` its item `item` was tapped, at
    /// `at` (its middle when chosen with the buttons).
    pub(super) fn tap_item(
        &self,
        layer: &Id,
        item: &LayerItem,
        at: Option<Point>,
        cx: &mut Cx<'_, '_>,
    ) {
        let area = item.area;
        let at = at.unwrap_or_else(|| {
            Point::new(
                i32::from(area.x) + i32::from(area.width) / 2,
                i32::from(area.y) + i32::from(area.height) / 2,
            )
        });
        let line = self.page_view(self.lines()).line_at(at).map(|l| l.start);
        let mut tapped = events::PageLayerTapped::new(layer.clone(), self.path.clone(), at.x, at.y)
            .item(item.id);
        if let Some(line) = line {
            tapped = tapped.section(line.section).offset(line.offset);
        }
        cx.command(events::PAGE_LAYER_TAPPED.emit(tapped));
    }

    /// Tells the plugin of layer `layer` its room was tapped, at `at`
    /// (its middle when chosen with the buttons).
    pub(super) fn tap_layer(&self, layer: &Id, at: Option<Point>, cx: &mut Cx<'_, '_>) {
        let Some((_, room)) = self.typography.rooms.iter().find(|(id, _)| id == layer) else {
            return;
        };
        let at = at.unwrap_or_else(|| {
            Point::new(
                i32::from(room.x) + i32::from(room.width) / 2,
                i32::from(room.y) + i32::from(room.height) / 2,
            )
        });
        let line = self.page_view(self.lines()).line_at(at).map(|l| l.start);
        let mut tapped = events::PageLayerTapped::new(layer.clone(), self.path.clone(), at.x, at.y);
        if let Some(line) = line {
            tapped = tapped.section(line.section).offset(line.offset);
        }
        cx.command(events::PAGE_LAYER_TAPPED.emit(tapped));
    }

    /// The page's pictures and text, each word in its ink (`inks`, by the
    /// words' order as [`Book::words`] has them; black where none).
    pub(super) fn draw_page(&self, canvas: &mut Canvas<'_>, env: &Env<'_>, inks: &[Option<Color>]) {
        let t = &self.typography;
        let Some(faces) = Faces::new(env, t, self.embedded.as_ref()) else {
            return;
        };
        for image in &self.page.images {
            paperpaw_sdk::reading::draw_image(canvas, env, t, image);
        }
        paperpaw_sdk::reading::draw_lines(canvas, &faces, t, (t.x, t.y), &self.page.lines, inks);
    }

    /// Where *Back* is offered on the page a link led to: a small button
    /// in the top margin, on the left.
    /// On a plugin's pages (web pages) it is always there: back to the
    /// page before.
    pub(super) fn back_area(&self, env: &Env<'_>) -> Option<Rect> {
        let landed = !self.returns.is_empty() && self.landed == Some(self.page.start);
        if !landed && !self.page_back {
            return None;
        }
        let style = UiStyle::new(env);
        let font = style.font(env)?;
        let label = env.i18n().tr("reader.link-back");
        let pad = style.gap() as f32;
        let width = ui::button_width(&style, font.measure(style.small(), label));
        // Within the top margin where it is wide enough, so it covers no
        // text.
        let line = font.line_metrics(style.small()).line_height;
        let top = self.content_top();
        let height = (line + pad).min(top - 4.0).max(line);
        let y = ((top - height) / 2.0).max(0.0);
        Some(Rect::new(
            self.typography.x as u16,
            y as u16,
            width as u16,
            height as u16,
        ))
    }

    pub(super) fn draw_back(&self, ui: &mut ui::Ui<'_>) {
        let env = ui.env();
        let Some(area) = self.back_area(env) else {
            return;
        };
        // A button as the interface's others.
        let label = env.i18n().tr("reader.link-back");
        ui.button(area, label, false, false);
    }

    /// Where a tap sets or removes a bookmark: the top right corner.
    pub(super) fn corner_area(&self, env: &Env<'_>) -> Rect {
        let display = env.device().display;
        let size = (env.metrics().mm(12) as u16).min(display.width / 4);
        Rect::new(display.width - size, 0, size, size)
    }

    pub(super) fn ribbon_area(&self, env: &Env<'_>) -> Rect {
        let width = env.metrics().mm(3) as u16;
        let height = (self.content_top() * 0.8) as u16;
        let x = self.right_edge() as u16 - width;
        Rect::new(x, 0, width, height.max(width))
    }

    /// A ribbon with a notch, like a bookmark in a book.
    pub(super) fn draw_ribbon(&self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let r = self.ribbon_area(env);
        let notch = u32::from(r.width) / 2;
        let body = u32::from(r.height).saturating_sub(notch);
        canvas.fill_rect(
            i32::from(r.x),
            i32::from(r.y),
            u32::from(r.width),
            body,
            Color::BLACK,
        );
        // Both sides of the notch narrow towards the bottom.
        for row in 0..notch {
            let side = notch - row;
            let y = i32::from(r.y) + (body + row) as i32;
            canvas.fill_rect(i32::from(r.x), y, side, 1, Color::BLACK);
            canvas.fill_rect(
                i32::from(r.x) + (u32::from(r.width) - side) as i32,
                y,
                side,
                1,
                Color::BLACK,
            );
        }
    }

    /// A note's sign in the right margin, beside the first line of its
    /// highlight (the highlights themselves are marks, see `marks`).
    pub(super) fn draw_note_signs(&self, ui: &mut ui::Ui<'_>, words: &[WordBox]) {
        let highlights: Vec<&Mark> = self
            .marks
            .iter()
            .filter(|m| m.kind == Kind::Highlight && m.overlaps(self.page.start, self.page.end))
            .collect();
        if highlights.is_empty() {
            return;
        }
        let style = ui.style();
        let size = style.small();
        let columns = &self.typography.columns;
        for mark in highlights.iter().filter(|m| m.noted) {
            if let Some(first) = words.iter().find(|w| mark.overlaps(w.start, w.end)) {
                // Beside its column; beside the layers' rooms past the last.
                let x = u32::from(first.area.x);
                let right = match columns.iter().position(|c| x < c.right()) {
                    Some(i) if i + 1 < columns.len() => columns[i].right() as f32,
                    _ => self.right_edge(),
                };
                let x = right + (style.gap() as f32) / 2.0;
                ui.icon("note", size, x, first.area, Color::BLACK);
            }
        }
    }

    /// What status items get to know about the book.
    pub(super) fn status_context(&self, env: &Env<'_>) -> Params {
        let reading = self.reading(env);
        let in_chapter = match (reading.page, reading.chapter_left) {
            (Some(page), Some(left)) => self
                .chapter_first(env)
                .map(|first| (page.saturating_sub(first), left)),
            _ => None,
        };
        let mut context = reading
            .to_params()
            .with(status_context::PAGE, self.position_label(env));
        if let Some((page, left)) = in_chapter {
            context.set(status::CHAPTER_PAGE_CONTEXT, format!("{page}"));
            context.set(status::CHAPTER_PAGES_CONTEXT, format!("{}", page + left));
        }
        if let Some((_, _, chapter)) = &self.chapter
            && !chapter.is_empty()
        {
            context.set(status_context::CHAPTER, chapter.clone());
        }
        if !self.chapter_marks.is_empty() {
            context.set(status_context::CHAPTERS, self.chapter_marks.clone());
        }
        if let Some(icon) = self.hands_free {
            context.set(status::ALOUD_CONTEXT, icon);
        }
        if let Some(page) = self.document.page_label(env.storage(), self.page.start) {
            context.set(status::PRINTED_CONTEXT, page);
        }
        context
    }

    /// Where the chapters begin, as thousandths of the book (see
    /// [`chapter_ticks`]).
    pub(super) fn chapter_marks(&self, env: &Env<'_>) -> String {
        let (document, storage) = (&self.document, env.storage());
        let entries: Vec<(u8, u32)> = (0..document.toc_len(storage).min(500))
            .filter_map(|index| document.toc_entry(storage, index))
            .map(|entry| (entry.level, (document.progress(entry.at) * 1000.0) as u32))
            .collect();
        let marks: Vec<String> = chapter_ticks(&entries)
            .iter()
            .map(ToString::to_string)
            .collect();
        marks.join(",")
    }

    /// The status line at the top (or bottom) that `point` is on: its
    /// place reaches to the edge of the screen.
    pub(super) fn status_hit(&self, env: &Env<'_>, point: Point) -> Option<bool> {
        let height = env.device().display.height;
        self.status_areas().find_map(|(area, top)| {
            let zone = if top {
                Rect::new(0, 0, env.device().display.width, area.y + area.height)
            } else {
                Rect::new(0, area.y, env.device().display.width, height - area.y)
            };
            zone.contains(point).then_some(top)
        })
    }

    /// What a tap at `point` on the status line at the top (or bottom)
    /// does, where an item of it does something: the item under it from
    /// side to side, its place reaching to the edge.
    pub(super) fn status_tap(
        &self,
        env: &Env<'_>,
        top: bool,
        point: Point,
    ) -> Option<paperpaw_sdk::Command> {
        let (area, _) = self.status_areas().find(|(_, at_top)| *at_top == top)?;
        let context = self.status_context(env);
        let frame = status::frame(env, area, top, &context);
        let id = status::current(env, top, &self.status_lines[usize::from(!top)]);
        let x = point.x;
        status::targets(env, id, &frame)
            .into_iter()
            .find(|t| (i32::from(t.area.x)..t.area.right() as i32).contains(&x))
            .map(|t| t.command)
    }

    /// The status lines' areas.
    pub(super) fn status_areas(&self) -> impl Iterator<Item = (Rect, bool)> {
        let t = &self.typography;
        t.top_status
            .map(|r| (r, true))
            .into_iter()
            .chain(t.status.map(|r| (r, false)))
    }

    /// Draws the status lines in the canvas's band; in the others what
    /// they show is not even asked (the X4 draws a page in eight bands).
    /// What they show ([`status::signature`]) where drawn.
    pub(super) fn draw_status(&self, canvas: &mut Canvas<'_>, env: &Env<'_>) -> Option<u64> {
        let clip = canvas.clip();
        let mut areas = self
            .status_areas()
            .filter(|(area, _)| area.intersection(&clip).is_some())
            .peekable();
        areas.peek()?;
        let context = self.status_context(env);
        for (area, top) in areas {
            canvas.fill(area, Color::WHITE);
            let id = status::current(env, top, &self.status_lines[usize::from(!top)]);
            let frame = status::frame(env, area, top, &context);
            env.status_items().draw_line(id, canvas, env, &frame);
        }
        Some(status::signature(env, &context, &self.status_lines))
    }
}

/// Ticks on a bar across the book that deeper levels of the contents
/// may bring it to: more crowd it into a line.
const MAX_TICKS: usize = 60;

/// The places of the contents' `entries` (level, thousandths) that get a
/// tick: every top-level entry, and the deeper ones (chapters in parts)
/// down to the deepest level whose entries and those above still leave
/// the ticks apart.
pub(super) fn chapter_ticks(entries: &[(u8, u32)]) -> Vec<u32> {
    let deepest = entries.iter().map(|&(level, _)| level).max().unwrap_or(0);
    let shown = (0..=deepest)
        .rev()
        .find(|&depth| entries.iter().filter(|&&(level, _)| level <= depth).count() <= MAX_TICKS)
        .unwrap_or(0);
    let mut ticks: Vec<u32> = entries
        .iter()
        .filter(|&&(level, _)| level <= shown)
        .map(|&(_, at)| at)
        .collect();
    ticks.dedup();
    ticks
}
