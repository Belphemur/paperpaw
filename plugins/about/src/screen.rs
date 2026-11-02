//! The page: a header, then rows as the other settings pages have them.
//! Rows with "›" open their text in the reader.
//!
//! Buttons: up and down move between the rows, *Select* opens one,
//! *Back* closes. Touch: tap.

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Color, Command, Cx, Env, Gesture, GestureKind, Handled, Id, RefreshMode, Screen,
};

/// What a row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Version,
    Source,
    BuiltIn,
    Native,
    Licences,
}

pub(crate) struct AboutScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    rows: Vec<Row>,
}

impl AboutScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let display = env.device().display;
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        let area =
            paperpaw_sdk::Rect::new(0, top, display.width, display.height.saturating_sub(top));
        // Rows as high as their notes need (the largest text); the names of
        // the plugins built in open whole from their row.
        let native = crate::native(env).unwrap_or_default().1;
        let notes = [
            env.i18n().tr("about.summary"),
            crate::SOURCE.trim_start_matches("https://"),
            native.as_str(),
            env.i18n().tr("about.licences.help"),
        ];
        let row = style.noted_row_for(env, display.width, notes);
        let list = ListView::new(area, row).focus_on_keys(env);
        let mut rows = Vec::from([Row::Version, Row::Source]);
        if env.firmware().is_some_and(|f| !f.plugins.is_empty()) {
            rows.push(Row::BuiltIn);
        }
        if env.native().is_some() {
            rows.push(Row::Native);
        }
        rows.push(Row::Licences);
        Self {
            style,
            header,
            list,
            rows,
        }
    }

    /// Opens what row `index` opens: its text in the reader.
    fn activate(&self, env: &Env<'_>, index: usize, cx: &mut Cx<'_, '_>) {
        let command = match self.rows.get(index) {
            Some(Row::Licences) => crate::read_licences(env),
            Some(Row::BuiltIn) => crate::read(env, crate::plugins_path(), &crate::built_in(env)),
            _ => None,
        };
        if let Some(command) = command {
            cx.command(command);
        }
    }
}

/// The plugins built in, by name, for a row's note.
fn built_in_names(env: &Env<'_>) -> (String, String) {
    let Some(firmware) = env.firmware() else {
        return (String::new(), String::new());
    };
    let names: Vec<&str> = firmware
        .plugins
        .iter()
        .map(|p| p.name.strip_prefix("paperpaw-plugin-").unwrap_or(p.name))
        .collect();
    (alloc::format!("{} ›", names.len()), names.join(", "))
}

impl Screen for AboutScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        canvas.clear(Color::WHITE);
        let style = self.style;
        let mut ui = ui::Ui::new(canvas, env, &style);
        let len = self.rows.len();
        self.list.set_len(len);
        self.header.draw(
            &mut ui,
            env.i18n().tr("about.title"),
            None,
            self.list.paging(len),
        );
        let rows = &self.rows;
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let tr = |key| env.i18n().tr(key);
            match rows[index] {
                Row::Version => ui::Row::value("PaperPaw", &crate::version(env))
                    .note(tr("about.summary"))
                    .focused(focused)
                    .draw(ui, rect),
                Row::Source => ui::Row::value(tr("about.source"), "")
                    .note(crate::SOURCE.trim_start_matches("https://"))
                    .focused(focused)
                    .draw(ui, rect),
                Row::BuiltIn => {
                    let (count, names) = built_in_names(env);
                    ui::Row::value(tr("about.plugins.title"), &count)
                        .note(&names)
                        .focused(focused)
                        .draw(ui, rect);
                }
                Row::Native => {
                    let (fingerprint, note) = crate::native(env).unwrap_or_default();
                    ui::Row::value(tr("about.native.title"), &fingerprint)
                        .note(&note)
                        .focused(focused)
                        .draw(ui, rect);
                }
                Row::Licences => ui::Row::value(tr("about.licences"), "›")
                    .note(tr("about.licences.help"))
                    .focused(focused)
                    .draw(ui, rect),
            }
        });
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let len = self.rows.len();
        let a = action;
        let delta = if [actions::UP, actions::LEFT, actions::PREVIOUS].contains(a) {
            -1
        } else if [actions::DOWN, actions::RIGHT, actions::NEXT].contains(a) {
            1
        } else if *a == actions::CONFIRM {
            self.activate(&env, self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        self.list.move_focus_in(cx.env(), delta, len).request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let env = *cx.env();
        let paging = self.list.paging(self.rows.len());
        if let Some(action) = self.header.tap(&env, &self.style, gesture.start, paging) {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        let Some(index) = self.list.hit(gesture.start, self.rows.len()) else {
            return Handled::No;
        };
        self.list.set_focus(index);
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
        self.activate(&env, index, cx);
        Handled::Yes
    }
}
