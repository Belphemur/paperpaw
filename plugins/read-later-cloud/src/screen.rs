//! The set-up: the access token, and fetching.

use alloc::string::{String, ToString};

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::event::Event;
use paperpaw_sdk::replies::Typed;
use paperpaw_sdk::secrets::SecretChange;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Command, Commands, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect,
    RefreshMode, Screen,
};

use crate::{DONE, FETCH, SECRET};

/// Typed on the keyboard; parameter `field`.
pub const ENTERED: Event<Typed> = Event::new("read-later-cloud.entered");
pub const FIELD: &str = "field";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Token,
    Fetch,
}

const ROWS: [Row; 2] = [Row::Token, Row::Fetch];

impl Row {
    fn title(self) -> &'static str {
        match self {
            Row::Token => "read-later-cloud.token",
            Row::Fetch => "read-later-cloud.fetch",
        }
    }

    fn field(self) -> &'static str {
        match self {
            Row::Token => "token",
            Row::Fetch => "",
        }
    }

    fn of(field: &str) -> Option<Self> {
        ROWS.into_iter()
            .find(|r| r.field() == field && !field.is_empty())
    }
}

/// The token typed: kept with the passwords and keys (not an empty one).
pub fn entered(typed: &Typed, out: &mut Commands) {
    let Some(Row::Token) = typed.context.get(FIELD).and_then(Row::of) else {
        return;
    };
    let text = typed.text.trim();
    if !text.is_empty() {
        out.push(Command::set_secret(SecretChange::keep(
            SECRET,
            "read-later-cloud.secret",
            text,
        )));
    }
}

pub struct SetupScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    note: Rect,
    message: String,
}

impl SetupScreen {
    pub fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let height = style.row() * ROWS.len() as u16;
        let note_top = top + height + style.gap();
        Self {
            list: ListView::new(
                Rect::new(0, top, display.width, height),
                u32::from(style.row()),
            )
            .focus_on_keys(env),
            note: Rect::new(
                0,
                note_top,
                display.width,
                display.height.saturating_sub(note_top),
            ),
            message: env.i18n().tr("read-later-cloud.help").to_string(),
            style,
            header,
        }
    }

    fn activate(&mut self, row: Row, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        if row == Row::Fetch {
            cx.command(Command::run(FETCH));
            return;
        }
        let question = paperpaw_sdk::ask::text(env.i18n().tr(row.title()), &ENTERED)
            .password()
            .with(FIELD, row.field());
        cx.command(question.command());
    }
}

impl Screen for SetupScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let mut ui = ui::Ui::new(canvas, env, &style);
        let title = env.i18n().tr("read-later-cloud.title");
        self.header.draw(&mut ui, title, None, None);
        self.list
            .draw(&mut ui, ROWS.len(), |ui, index, rect, focused| {
                let row = ROWS[index];
                let value = match row {
                    Row::Token if env.secrets().get(SECRET).is_some() => "••••••",
                    Row::Token => "",
                    Row::Fetch => "›",
                };
                let title = env.i18n().tr(row.title());
                ui::Row::value(title, value).focused(focused).draw(ui, rect);
            });
        ui.message(self.note, &self.message);
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let a = action;
        let delta = if [actions::UP, actions::LEFT, actions::PREVIOUS].contains(a) {
            -1
        } else if [actions::DOWN, actions::RIGHT, actions::NEXT].contains(a) {
            1
        } else if *a == actions::CONFIRM {
            self.activate(ROWS[self.list.focus().min(ROWS.len() - 1)], cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        self.list
            .move_focus_in(cx.env(), delta, ROWS.len())
            .request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        if let Some(action) = self.header.tap(cx.env(), &self.style, gesture.start, None) {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        let Some(index) = self.list.hit(gesture.start, ROWS.len()) else {
            return Handled::No;
        };
        self.list.set_focus(index);
        cx.request_redraw_area(self.list.area());
        self.activate(ROWS[index], cx);
        Handled::Yes
    }

    fn on_event(&mut self, event: &Id, p: &Params, cx: &mut Cx<'_, '_>) {
        if let Some(finished) = DONE.read(event, p) {
            self.message = finished.text;
        } else if let Some(typed) = ENTERED.read(event, p) {
            if typed.context.get(FIELD).and_then(Row::of) == Some(Row::Token) {
                self.list.set_focus(ROWS.len() - 1);
            }
        } else {
            return;
        }
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_by_field() {
        assert_eq!(Row::of("token"), Some(Row::Token));
        assert_eq!(Row::of(""), None);
        assert_eq!(Row::of("nonsense"), None);
    }
}
