//! *Go to letter*: the letters A to Z in a grid; the one chosen goes back
//! to the list that asked ([`LETTER`]), which shows its plugins from that
//! letter on. A group of thousands of plugins is then a few steps away
//! with the buttons, and a tap with a finger.

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::event::Event;
use paperpaw_sdk::replies::Typed;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Color, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, Screen,
};

/// The letter chosen, with the parameters the screen was opened with.
pub(crate) const LETTER: Event<Typed> = Event::new("store.letter");

/// Letters side by side.
const COLUMNS: u32 = 5;
const LETTERS: usize = 26;

pub(crate) struct LettersScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    params: Params,
}

impl LettersScreen {
    pub(crate) fn new(env: &Env<'_>, params: &Params) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let rows = (LETTERS as u32).div_ceil(COLUMNS);
        // As high as the screen leaves room for, no higher than wide.
        let room = u32::from(display.height.saturating_sub(top + style.gap()));
        let width = u32::from(display.width.saturating_sub(2 * style.margin()));
        let row = (room / rows).min(width / COLUMNS).max(1);
        let area = Rect::new(style.margin(), top, width as u16, (row * rows) as u16);
        Self {
            list: ListView::new(area, row).columns(COLUMNS),
            style,
            header,
            params: params.clone(),
        }
    }

    fn choose(&self, index: usize, cx: &mut Cx<'_, '_>) {
        let Some(letter) = letter(index) else {
            return;
        };
        cx.command(Command::Back);
        let mut text = [0u8; 4];
        let text = letter.encode_utf8(&mut text);
        cx.command(LETTER.emit(Typed::new(&*text, self.params.clone())));
    }
}

/// Letter `index` of A to Z.
fn letter(index: usize) -> Option<char> {
    (index < LETTERS).then(|| char::from(b'A' + index as u8))
}

impl Screen for LettersScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, env.i18n().tr("store.letter"), None, None);
        let size = style.title() * 1.2;
        self.list
            .draw(&mut ui, LETTERS, |ui, index, rect, focused| {
                let Some(letter) = letter(index) else {
                    return;
                };
                let mut text = [0u8; 4];
                let text = letter.encode_utf8(&mut text);
                let ink = if focused { Color::WHITE } else { Color::BLACK };
                let width = ui.font().map_or(0.0, |font| font.measure(size, text));
                let x = f32::from(rect.x) + (f32::from(rect.width) - width) / 2.0;
                ui::Label::new(text)
                    .px(size)
                    .color(ink)
                    .draw_from(ui, x, rect);
            });
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let columns = COLUMNS as isize;
        let a = action;
        let delta = if [actions::LEFT, actions::PREVIOUS].contains(a) {
            -1
        } else if [actions::RIGHT, actions::NEXT].contains(a) {
            1
        } else if [actions::UP, actions::PREV_PAGE].contains(a) {
            -columns
        } else if [actions::DOWN, actions::NEXT_PAGE].contains(a) {
            columns
        } else if *a == actions::CONFIRM {
            self.choose(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        self.list
            .move_focus_in(cx.env(), delta, LETTERS)
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
        let Some(index) = self.list.hit(gesture.start, LETTERS) else {
            return Handled::No;
        };
        self.choose(index, cx);
        Handled::Yes
    }
}
