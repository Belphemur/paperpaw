//! The editor of the setting `keyboard.layouts`: every layout with *On*
//! or *Off*; the order of switching on is the order of switching layouts.
//! *Automatic* (the UI language's layout) stands for itself: choosing it
//! leaves the others, choosing another layout leaves it.

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Rect, RefreshMode, Screen,
};

use crate::LAYOUTS_SETTING;
use crate::layouts::{AUTO, LAYOUTS, added, drawable};

pub(crate) struct ChooseScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    /// The rows ([`ids`]), found once: whether the fonts draw a layout
    /// takes a look at every letter.
    rows: Vec<String>,
}

/// The rows: *Automatic*, the built-in layouts the fonts can draw, then
/// those other plugins added.
fn ids(env: &Env<'_>) -> Vec<String> {
    let builtin = LAYOUTS
        .iter()
        .filter(|l| drawable(env, l.letters))
        .map(|l| String::from(l.id));
    let added = added(env)
        .into_iter()
        .filter(|l| drawable(env, &l.letters))
        .map(|l| l.id);
    core::iter::once(String::from(AUTO))
        .chain(builtin)
        .chain(added)
        .collect()
}

fn chosen<'e>(env: &Env<'e>) -> Vec<&'e str> {
    env.settings()
        .text(LAYOUTS_SETTING.as_str())
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect()
}

impl ChooseScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let display = env.device().display;
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height - top);
        Self {
            list: ListView::new(area, u32::from(style.row())).focus_on_keys(env),
            style,
            header,
            rows: ids(env),
        }
    }

    fn toggle(&mut self, index: usize, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let Some(id) = self.rows.get(index).map(String::as_str) else {
            return;
        };
        let mut chosen = chosen(&env);
        if id == AUTO {
            chosen = Vec::from([AUTO]);
        } else {
            chosen.retain(|c| *c != AUTO);
            match chosen.iter().position(|c| *c == id) {
                Some(at) => {
                    chosen.remove(at);
                }
                None => chosen.push(id),
            }
            if chosen.is_empty() {
                chosen.push(AUTO);
            }
        }
        cx.command(Command::set(
            &LAYOUTS_SETTING,
            Cow::Owned(chosen.join(", ")),
        ));
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
    }
}

impl Screen for ChooseScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.rows.len();
        let title = env.i18n().tr("keyboard.setting.layouts");
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, title, None, self.list.paging(len));
        let chosen = chosen(env);
        let rows = &self.rows;
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let id = rows[index].as_str();
            let on = chosen.iter().position(|c| *c == id);
            // Chosen layouts carry their place in switching: "On (2)".
            let value = match on {
                Some(at) if chosen.len() > 1 => {
                    format!("{} ({})", env.i18n().tr("keyboard.on"), at + 1)
                }
                Some(_) => env.i18n().tr("keyboard.on").to_string(),
                None => env.i18n().tr("keyboard.off").to_string(),
            };
            ui::Row::value(env.i18n().tr(id), &value)
                .focused(focused)
                .draw(ui, rect);
        });
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = self.rows.len();
        let rows = self.list.rows() as isize;
        let a = action;
        let delta = if let Some(delta) = ui::list_step(a, rows) {
            delta
        } else if *a == actions::CONFIRM {
            self.toggle(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        let change = self.list.move_focus_in(cx.env(), delta, len);
        change.request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let len = self.rows.len();
        let paging = self.list.paging(len);
        if let Some(action) = self
            .header
            .tap(cx.env(), &self.style, gesture.start, paging)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        let Some(index) = self.list.hit(gesture.start, len) else {
            return Handled::No;
        };
        self.list.set_focus(index);
        self.toggle(index, cx);
        Handled::Yes
    }
}
