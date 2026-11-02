//! Choosing a link on the page with the buttons, following it, and the
//! way back.

use super::*;

/// Notes remembered for *Back*, from the one opened last.
const NOTES_KEPT: usize = 16;

/// Choosing a link with the buttons: which of the page's links that can
/// be followed has the focus (`None` if the page has none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Linking {
    pub(super) focus: Option<usize>,
}

impl ReaderScreen {
    pub(super) fn set_linking(&mut self, to: Option<Linking>, cx: &mut Cx<'_, '_>) {
        if self.linking != to {
            self.linking = to;
            cx.request_redraw();
            cx.request_refresh(RefreshMode::Fast);
        }
    }

    /// Buttons while choosing a link: the next or previous one (on to
    /// the next or previous page past the last or first), *confirm*
    /// follows it, *back* ends.
    pub(super) fn on_linking(
        &mut self,
        action: &Id,
        linking: Linking,
        cx: &mut Cx<'_, '_>,
    ) -> Handled {
        let env = *cx.env();
        let a = action;
        let forward = [
            actions::RIGHT,
            actions::DOWN,
            actions::NEXT,
            actions::NEXT_PAGE,
        ];
        let backward = [
            actions::LEFT,
            actions::UP,
            actions::PREVIOUS,
            actions::PREV_PAGE,
        ];
        let Ok(book) = &mut self.book else {
            self.linking = None;
            return Handled::No;
        };
        let count = book.targets(&env).len();
        if forward.contains(a) {
            match linking.focus {
                Some(i) if i + 1 < count => {
                    self.set_linking(Some(Linking { focus: Some(i + 1) }), cx);
                }
                _ => {
                    let end = book.page.end;
                    book.show(cx, end, true);
                    let focus = (!book.targets(&env).is_empty()).then_some(0);
                    self.linking = Some(Linking { focus });
                    cx.request_redraw();
                }
            }
        } else if backward.contains(a) {
            match linking.focus {
                Some(i) if i > 0 => {
                    self.set_linking(Some(Linking { focus: Some(i - 1) }), cx);
                }
                _ => {
                    if let Some(start) = book.previous_start(&env) {
                        book.show(cx, start, false);
                    }
                    let count = book.targets(&env).len();
                    self.linking = Some(Linking {
                        focus: count.checked_sub(1),
                    });
                    cx.request_redraw();
                }
            }
        } else if *a == actions::CONFIRM {
            let target = linking
                .focus
                .and_then(|i| book.targets(&env).get(i).copied());
            let words = book.words(&env);
            let link = match target {
                Some(Target::Link(i)) => book.links.get(i).cloned(),
                Some(Target::Mark(i)) => {
                    // The plugin answers (e.g. the word's meaning).
                    book.tap_mark(words, i, cx);
                    None
                }
                Some(Target::Layer(i)) => {
                    if let Some((layer, _)) = book.typography.rooms.get(i) {
                        book.tap_layer(layer, None, cx);
                    }
                    None
                }
                Some(Target::Item(i)) => {
                    if let Some((layer, item)) = book.layer_items(&env).get(i) {
                        book.tap_item(layer, item, None, cx);
                    }
                    None
                }
                None => None,
            };
            let link = link.map(|l| (l.href.clone(), link_label(book, words, &l)));
            self.set_linking(None, cx);
            if let Some((href, label)) = link {
                return self.open_link(&href, &label, cx);
            }
        } else if *a == actions::BACK || *a == actions::MENU {
            self.set_linking(None, cx);
        } else {
            // Anything else ends choosing and goes on as usual.
            self.set_linking(None, cx);
            return Handled::No;
        }
        Handled::Yes
    }

    /// Opens a link: a note over the page if it looks like one and the
    /// reader shows notes that way, else follows it.
    pub(super) fn open_link(&mut self, href: &str, label: &str, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let popup =
            env.settings().read(&crate::settings::FOOTNOTES) == crate::settings::FOOTNOTES_POPUP;
        if popup
            && !paperpaw_sdk::document::is_external(href)
            && crate::footnote::note_like(label)
            && let Ok(book) = &self.book
            && let Some(text) = book.note_text(&env, href)
        {
            cx.command(Command::OpenSlot(
                slots::FOOTNOTE,
                Params::new()
                    .with(params::TEXT, text)
                    .with(params::PATH, book.path.clone())
                    .with(params::URL, href.to_string()),
            ));
            return Handled::Yes;
        }
        self.follow(href, cx)
    }

    /// Follows link `href`: to its place in the book, remembering where
    /// it was followed from for *Back*.
    pub(super) fn follow(&mut self, href: &str, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let Ok(book) = &mut self.book else {
            return Handled::No;
        };
        if paperpaw_sdk::document::is_external(href) {
            // To the web, if a browser is there.
            if !env.has_slot(&slots::BROWSER) {
                return Handled::No;
            }
            // Told apart by the time, so a page asked for again is loaded
            // again rather than the open browser just brought forward.
            cx.command(Command::OpenSlot(
                slots::BROWSER,
                Params::new()
                    .with(params::URL, href.to_string())
                    .with(params::ID, format!("{}", env.now().0)),
            ));
            return Handled::Yes;
        }
        let Some(target) = book.document.link_target(env.storage(), href) else {
            return self.follow_note(href, cx);
        };
        let start = book.page_start(&env, target);
        book.jump(cx, start);
        Handled::Yes
    }
}

impl ReaderScreen {
    /// Follows a link that leads to another note (a wikilink, or the path
    /// of a note): the note opens over this book, at its heading if the
    /// link names one, and *Back* returns here. Said when it is not
    /// found.
    fn follow_note(&mut self, href: &str, cx: &mut Cx<'_, '_>) -> Handled {
        let Some(link) = notelinks::read(href) else {
            return Handled::No;
        };
        let env = *cx.env();
        let Ok(book) = &mut self.book else {
            return Handled::No;
        };
        let Some(path) = notelinks::find(env.storage(), &book.path, &link) else {
            cx.command(Command::Notify(env.i18n().format(
                "reader.note-missing",
                &[("name", paperpaw_sdk::Arg::Str(link.what.as_str()))],
            )));
            return Handled::Yes;
        };
        // Where it was left: what is kept, as with any book.
        book.save_due(cx, true);
        let mut before = book.notes_back.clone();
        before.push(book.path.clone());
        // The farthest forgotten, as far back as one goes.
        let skip = before.len().saturating_sub(NOTES_KEPT);
        cx.command(notelinks::open(
            &env,
            &path,
            link.anchor.as_deref(),
            &before[skip..],
        ));
        Handled::Yes
    }
}

/// The text of `link` on the page: the parts of its words inside it (a
/// note mark is often part of a word, as in `source¹`).
pub(super) fn link_label(book: &Book, words: &[WordBox], link: &Link) -> String {
    let mut label = String::new();
    for word in words.iter().filter(|w| {
        w.start.section == book.page.start.section
            && w.end.offset > link.start
            && w.start.offset < link.end
    }) {
        let from = link.start.saturating_sub(word.start.offset) as usize;
        let to = (link.end.min(word.end.offset) - word.start.offset) as usize;
        let part = word.text.get(from..to).unwrap_or(&word.text);
        if !label.is_empty() {
            label.push(' ');
        }
        label.push_str(part);
    }
    label
}

/// The link or mark being chosen, framed like selected text, and a bar at
/// the bottom saying where it leads (or whose mark it is) and what the
/// buttons do.
pub(super) fn draw_linking(ui: &mut ui::Ui<'_>, book: &Book, words: &[WordBox], linking: Linking) {
    let (env, style) = (ui.env(), ui.style());
    let target = linking
        .focus
        .and_then(|i| book.targets(env).get(i).copied());
    let line = env.metrics().tenth_mm(4).max(2);
    // An item a layer drew, framed.
    if let Some(Target::Item(i)) = target
        && let Some((_, item)) = book.layer_items(env).get(i)
    {
        let a = item.area;
        ui.canvas().stroke_rect(
            i32::from(a.x) - 2,
            i32::from(a.y) - 2,
            u32::from(a.width) + 4,
            u32::from(a.height) + 4,
            line,
            Color::BLACK,
        );
    }
    // A layer's room, framed whole.
    if let Some(Target::Layer(i)) = target
        && let Some((_, room)) = book.typography.rooms.get(i)
    {
        ui.canvas().stroke_rect(
            i32::from(room.x),
            i32::from(room.y),
            u32::from(room.width),
            u32::from(room.height),
            line,
            Color::BLACK,
        );
    }
    if let Some((start, end)) = target.and_then(|t| book.target_range(t)) {
        let inside = |i: usize| words[i].end > start && words[i].start < end;
        for b in selection::line_boxes(words, inside) {
            ui.canvas().stroke_rect(
                i32::from(b.x) - 4,
                i32::from(b.y) - 2,
                u32::from(b.width) + 8,
                u32::from(b.height) + 4,
                line,
                Color::BLACK,
            );
        }
    }
    let bar = selection::bar_area(env, style);
    ui.canvas().fill(bar, Color::WHITE);
    ui.canvas().fill_rect(
        i32::from(bar.x),
        i32::from(bar.y),
        u32::from(bar.width),
        u32::from(style.line()).max(2),
        Color::BLACK,
    );
    let Some(font) = ui.font() else {
        return;
    };
    let text = bar_text(env, book, target);
    let margin = style.margin() as f32;
    let text = ui::fit(
        &font,
        style.small(),
        &text,
        f32::from(bar.width) - 2.0 * margin,
    );
    ui::Label::new(&text)
        .px(style.small())
        .color(Color::BLACK)
        .draw_from(ui, f32::from(bar.x) + margin, bar);
}

/// What the bar says while `target` is chosen: where it leads, or whose
/// it is, and what the buttons do.
fn bar_text(env: &Env<'_>, book: &Book, target: Option<Target>) -> String {
    match target {
        Some(Target::Link(i)) => {
            let href = book.links.get(i).map_or("", |l| l.href.as_str());
            let place = if paperpaw_sdk::document::is_external(href) {
                paperpaw_sdk::web::host(href).to_string()
            } else {
                env.i18n().tr("reader.link.in-book").to_string()
            };
            format!("{place} · {}", env.i18n().tr("reader.link.hint"))
        }
        Some(Target::Mark(i)) => {
            // The name of the plugin's decorator, as it marks the word.
            let name = book
                .decorated
                .get(i)
                .and_then(|m| env.decorators().get(&m.decorator))
                .map_or("", |d| env.i18n().tr(&d.title));
            format!("{name} · {}", env.i18n().tr("reader.link.hint"))
        }
        Some(Target::Layer(i)) => {
            let name = book
                .typography
                .rooms
                .get(i)
                .and_then(|(id, _)| env.page_layers().get(id))
                .map_or("", |l| env.i18n().tr(&l.title));
            format!("{name} · {}", env.i18n().tr("reader.link.hint"))
        }
        Some(Target::Item(i)) => {
            let items = book.layer_items(env);
            let said = items
                .get(i)
                .map(|(layer, item)| match item.label.is_empty() {
                    true => env
                        .page_layers()
                        .get(layer)
                        .map_or("", |l| env.i18n().tr(&l.title))
                        .to_string(),
                    false => item.label.clone(),
                });
            format!(
                "{} · {}",
                said.unwrap_or_default(),
                env.i18n().tr("reader.link.hint")
            )
        }
        None => env.i18n().tr("reader.link.none").to_string(),
    }
}
