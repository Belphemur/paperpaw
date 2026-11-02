//! Every tool at once, as icons in a grid (as a phone lists its apps):
//! opened by swiping up on the home screen. Tapping one runs it; the
//! buttons move between them. And the hint on the gestures, once, the
//! first time the home screen shows on a touch screen. Where the interface
//! runs right to left the grid does too: the first tool at the top right,
//! the left button on to the next one.

use alloc::vec::Vec;

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::ui::{self, ListChange, ListView, UiStyle};
use paperpaw_sdk::widgets::grid;
use paperpaw_sdk::{
    Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Point, Rect, Screen,
};

use crate::{blocks, pages};

/// Columns of the overview on a screen higher than wide; more on one that
/// is wider ([`grid::columns_for`]).
const COLUMNS: u8 = 4;

pub(crate) struct DrawerScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    tools: Vec<Id>,
    /// Tools side by side.
    columns: u32,
    /// The lines the longest name takes: every tool keeps room for them.
    lines: usize,
}

impl DrawerScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let area = Rect::new(
            style.margin(),
            top,
            display.width.saturating_sub(2 * style.margin()),
            display.height.saturating_sub(top),
        );
        // About as high as a column is wide; a line higher where a tool's
        // name takes two (its icon as large as the others').
        let columns = u32::from(grid::columns_for(area, COLUMNS));
        let row = (u32::from(area.width) / columns * 4 / 5).max(u32::from(style.row()));
        let tools: Vec<Id> = pages::tools(env).into_iter().map(|(id, _)| id).collect();
        // The names' lines as they fall in a row grown by that line.
        let line = u32::from(paperpaw_sdk::widgets::line_height(env, style.small()));
        let width = (u32::from(area.width) / columns) as u16;
        let cell = Rect::new(0, 0, width, (row + line) as u16);
        // Rows are higher by the room the focus ring takes above and below
        // an icon and name (see `clear_of`).
        let row = row + 2 * u32::from(inset(&style));
        let lines = tools
            .iter()
            .map(|id| blocks::tool_name_lines(env, cell, id))
            .max()
            .unwrap_or(0);
        let more = if lines > 1 { line } else { 0 };
        Self {
            list: ListView::new(area, row + more)
                .columns(columns)
                .focus_on_keys(env),
            tools,
            columns,
            lines,
            style,
            header,
        }
    }

    /// `rect`, a place of the list's grid (laid out from the left), as it
    /// lies where the interface runs right to left and the grid with it:
    /// the columns' leftover width stays at the left.
    fn flip(&self, rect: Rect) -> Rect {
        self.style.flip(self.list.area(), rect)
    }

    fn run(&self, index: usize, cx: &mut Cx<'_, '_>) {
        if let Some(action) = self.tools.get(index) {
            cx.command(Command::Back);
            cx.command(Command::run(action.clone()));
        }
    }
}

/// How far the focus ring stands from the ink of a tool's icon and name
/// above and below (see [`clear_of`]).
fn reach(style: &UiStyle) -> u16 {
    style.gap() / 2 * 2 + style.line() * 3
}

/// How far the ring may go past a tool's cell above and below: the room
/// a redraw takes there.
fn past(style: &UiStyle) -> u16 {
    style.gap() / 2 + 2
}

/// The room a tool's icon and name keep from the top and the bottom of
/// its cell, so that the focus ring is no more than [`past`] outside it.
fn inset(style: &UiStyle) -> u16 {
    reach(style).saturating_sub(past(style))
}

/// Where the focus of the tool at `cell` whose icon and name have ink in
/// `ink` goes round: a half gap further from the ink than the focus
/// usually stands, so that the name is not close to the ring. Sideways it
/// never goes past the cell (the next tool is there) nor through the ink:
/// a name as wide as the cell keeps the usual distance. Above and below it
/// may go a little past the cell, into the room the rows have for it.
fn clear_of(style: &UiStyle, cell: Rect, ink: Rect) -> Rect {
    let more = i64::from(style.gap() / 2);
    // What the ring takes round the ink it is given (see `draw_tool_focus`).
    let ring = i64::from(style.gap() / 2 + style.line() * 3);
    let left = (i64::from(ink.x) - more)
        .max(i64::from(cell.x) + ring)
        .min(i64::from(ink.x));
    let right = (i64::from(ink.right()) + more)
        .min(i64::from(cell.right()) - ring)
        .max(i64::from(ink.right()));
    Rect::new(
        left.max(0) as u16,
        ink.y.saturating_sub(more as u16),
        (right - left).max(0) as u16,
        ink.height.saturating_add(2 * more as u16),
    )
}

impl Screen for DrawerScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.tools.len();
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header.draw(
            &mut ui,
            env.i18n().tr("home.drawer"),
            None,
            self.list.paging(len),
        );
        let tools = &self.tools;
        let list = &self.list;
        let first = list.page() * list.rows();
        // Room for as many lines of a name as the longest takes, so that
        // icons and names line up.
        let lines = self.lines;
        for (slot, index) in (first..len).take(list.rows()).enumerate() {
            let rect = self.flip(list.row_rect(slot));
            let focused = list.shown() && index == list.focus();
            // Named after its action, also where its name is not shown.
            let name = grid::What::Tool(tools[index].clone()).title(env);
            ui.canvas()
                .describe(paperpaw_sdk::semantics::Role::Button, &name, rect)
                .focused(focused);
            let inner = rect.inset(0, inset(&style));
            let ink = blocks::draw_tool_lines(ui.canvas(), env, inner, &tools[index], lines);
            if focused {
                // Round its icon and name, never through them.
                paperpaw_sdk::widgets::grid::draw_tool_focus(
                    ui.canvas(),
                    &style,
                    clear_of(&style, rect, ink),
                );
            }
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = self.tools.len();
        let a = action;
        let columns = self.columns as isize;
        // Left and right are where the tools are: from the right, the next
        // one is to the left.
        let (back, on) = if self.style.is_rtl() {
            (actions::RIGHT, actions::LEFT)
        } else {
            (actions::LEFT, actions::RIGHT)
        };
        let delta = if *a == back || *a == actions::PREVIOUS {
            -1
        } else if *a == on || *a == actions::NEXT {
            1
        } else if *a == actions::UP {
            -columns
        } else if *a == actions::DOWN {
            columns
        } else if *a == actions::NEXT_PAGE {
            self.list.rows() as isize
        } else if *a == actions::PREV_PAGE {
            -(self.list.rows() as isize)
        } else if *a == actions::CONFIRM {
            self.run(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        match self.list.move_focus(delta, len) {
            ListChange::Focus { left, reached, .. } => {
                // The focus goes round a tool's icon and name: past its cell
                // above and below by the room the rows keep for it (a half
                // gap and a bit), sideways by a name as wide as the cell.
                let (side, high) = (
                    self.style.gap() / 2 + self.style.line() * 3 + 1,
                    past(&self.style),
                );
                let round = |cell: Rect| {
                    Rect::new(
                        cell.x.saturating_sub(side),
                        cell.y.saturating_sub(high),
                        cell.width.saturating_add(2 * side),
                        cell.height.saturating_add(2 * high),
                    )
                };
                ListChange::focus(round(self.flip(left)), round(self.flip(reached))).request(cx);
            }
            change => change.request(cx),
        }
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        match gesture.kind {
            GestureKind::Tap => {
                let len = self.tools.len();
                let paging = self.list.paging(len);
                if let Some(action) = self
                    .header
                    .tap(cx.env(), &self.style, gesture.start, paging)
                {
                    cx.command(Command::run(action));
                    return Handled::Yes;
                }
                // Where the grid is mirrored, the point is where it would be
                // on the grid from the left.
                let area = self.list.area();
                let start = if self.style.is_rtl() && area.contains(gesture.start) {
                    // A point mirrors as a rectangle 1 pixel wide.
                    let x = u16::try_from(gesture.start.x).unwrap_or_default();
                    let mirrored = self.style.flip(area, Rect::new(x, 0, 1, 1));
                    Point::new(i32::from(mirrored.x), gesture.start.y)
                } else {
                    gesture.start
                };
                match self.list.hit(start, len) {
                    Some(index) => {
                        self.run(index, cx);
                        Handled::Yes
                    }
                    None => Handled::No,
                }
            }
            // Down again: back to the home screen.
            GestureKind::Swipe(paperpaw_sdk::Direction::Down) => {
                cx.command(Command::Back);
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}
