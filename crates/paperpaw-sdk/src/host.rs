//! Screens inside a screen: tabs, a split view, a pane of another
//! plugin's screen (see [`ScreenHost`]).
//!
//! A screen that hosts another makes it with
//! [`Env::make_screen`](crate::Env::make_screen) for a part of its own
//! area, and passes on to it what it gets: drawing, input in that part
//! (in the hosted screen's coordinates), events, timers and its own
//! showing and closing. [`ScreenHost`] does that passing on; the host
//! decides which screen it holds and where.
//!
//! ```ignore
//! struct Tabs { host: ScreenHost }
//!
//! impl Screen for Tabs {
//!     fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
//!         let pane = Rect::new(0, 40, cx.env().device().display.width, 400);
//!         self.host.show(cx, LIBRARY, Params::new(), pane);
//!         self.host.on_show(cx);
//!     }
//!     fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
//!         draw_tab_row(canvas, env);
//!         self.host.draw(canvas, env);
//!     }
//!     fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
//!         self.host.on_gesture(gesture, cx)
//!     }
//!     fn on_close(&mut self, cx: &mut Cx<'_, '_>) {
//!         self.host.close(cx);
//!     }
//!     // Its revision is no higher than the screen's it holds.
//!     fn revision(&self) -> u16 {
//!         self.host.revision()
//!     }
//!     // on_action, on_event, on_timer and on_hide passed on alike
//! }
//! ```
//!
//! A host passes on only items the screen it holds has: its own
//! [`Screen::revision`] is [`ScreenHost::revision`], and items newer than
//! revision 1 go through [`call_since`].

use alloc::boxed::Box;

use paperpaw_graphics::Canvas;
use paperpaw_platform::{PointerEvent, Rect};

use crate::env::Env;
use crate::gesture::Gesture;
use crate::id::Id;
use crate::params::Params;
use crate::screen::{Cx, Handled, Output, SCREEN_REVISION, Screen, call_since, revision_holding};

/// The screen a host holds.
struct Hosted {
    id: Id,
    params: Params,
    screen: Box<dyn Screen>,
    /// Its part of the host, in the host's coordinates.
    area: Rect,
}

/// Holds one screen inside another and passes on to it what the host
/// gets (see the [module](self)).
#[derive(Default)]
pub struct ScreenHost {
    hosted: Option<Hosted>,
}

impl core::fmt::Debug for ScreenHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScreenHost")
            .field("screen", &self.hosted.as_ref().map(|h| &h.id))
            .field("area", &self.hosted.as_ref().map(|h| h.area))
            .finish()
    }
}

/// `area` of a screen whose part of the screen is `content`, on the
/// screen.
fn on_screen(area: Rect, content: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(content.x),
        area.y.saturating_add(content.y),
        area.width,
        area.height,
    )
}

impl ScreenHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// The screen held, if any.
    pub fn screen(&self) -> Option<&Id> {
        self.hosted.as_ref().map(|hosted| &hosted.id)
    }

    /// Its part of the host, in the host's coordinates.
    pub fn area(&self) -> Option<Rect> {
        self.hosted.as_ref().map(|hosted| hosted.area)
    }

    /// The revision of the host ([`Screen::revision`]): no higher than
    /// that of the screen it holds ([`revision_holding`]), so that it is
    /// asked only what that one has; this SDK's without one.
    pub fn revision(&self) -> u16 {
        self.hosted.as_ref().map_or(SCREEN_REVISION, |hosted| {
            revision_holding(hosted.screen.as_ref())
        })
    }

    /// Holds screen `id` with `params` in `area` (of the host's
    /// coordinates) in place of the one held before, which is closed:
    /// the same screen with the same params is only moved, and of another
    /// size laid out anew ([`Screen::on_resize`]) or, where it does not
    /// do that, made anew for it. Whether it could be made.
    pub fn show(&mut self, cx: &mut Cx<'_, '_>, id: Id, params: Params, area: Rect) -> bool {
        if let Some(hosted) = &mut self.hosted
            && hosted.id == id
            && hosted.params == params
        {
            let before = hosted.area;
            let resized = (before.width, before.height) != (area.width, area.height);
            hosted.area = area;
            // What it asks for is passed on only where it stays: made
            // anew, it was not asked anything (as the kernel does).
            let kept = !resized
                || self.pass_if(cx, true, |held, cx| {
                    call_since(held, 2, Handled::No, |held| held.on_resize(cx))
                }) == Handled::Yes;
            if kept {
                cx.request_redraw();
                return true;
            }
            // Closed where it was, as the kernel closes a screen it makes
            // anew.
            if let Some(hosted) = &mut self.hosted {
                hosted.area = before;
            }
        }
        self.close(cx);
        let env = *cx.env();
        let place = on_screen(area, env.content());
        let device = env.device_within(place);
        let made = env.within(&device, place).make_screen(&id, &params);
        let Some(screen) = made else {
            return false;
        };
        self.hosted = Some(Hosted {
            id,
            params,
            screen,
            area,
        });
        cx.request_redraw();
        true
    }

    /// Closes the screen held ([`Screen::on_close`]), if any.
    pub fn close(&mut self, cx: &mut Cx<'_, '_>) {
        self.pass(cx, |screen, cx| {
            screen.on_close(cx);
            Handled::Yes
        });
        self.hosted = None;
    }

    /// Runs `f` with the screen held and a context for it in its part,
    /// passing what it asks for on to the host's: commands, refreshes,
    /// redraws (moved to the host's coordinates), times.
    pub fn pass(
        &mut self,
        cx: &mut Cx<'_, '_>,
        f: impl FnOnce(&mut dyn Screen, &mut Cx<'_, '_>) -> Handled,
    ) -> Handled {
        self.pass_if(cx, false, f)
    }

    /// [`pass`](Self::pass); with `only_taken`, what it asks for is
    /// passed on only where `f` answers `Handled::Yes`.
    fn pass_if(
        &mut self,
        cx: &mut Cx<'_, '_>,
        only_taken: bool,
        f: impl FnOnce(&mut dyn Screen, &mut Cx<'_, '_>) -> Handled,
    ) -> Handled {
        let Some(hosted) = &mut self.hosted else {
            return Handled::No;
        };
        let env = *cx.env();
        let place = on_screen(hosted.area, env.content());
        let device = env.device_within(place);
        let own = env.within(&device, place);
        let mut output = Output::default();
        let handled = f(hosted.screen.as_mut(), &mut Cx::new(&own, &mut output));
        if only_taken && handled == Handled::No {
            return handled;
        }
        for command in output.commands {
            cx.command(command);
        }
        if let Some(mode) = output.refresh {
            cx.request_refresh(mode);
        }
        if output.redraw {
            cx.request_redraw_area(hosted.area);
        }
        if let Some(area) = output.redraw_area {
            cx.request_redraw_area(on_screen(area, hosted.area));
        }
        if let Some(at) = output.wake_at {
            cx.wake_at(at);
        }
        handled
    }

    /// Draws the screen held into its part of `canvas` (the host's).
    pub fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let Some(hosted) = &mut self.hosted else {
            return;
        };
        let place = on_screen(hosted.area, env.content());
        let device = env.device_within(place);
        let own = env.within(&device, place);
        let (x, y) = (i32::from(hosted.area.x), i32::from(hosted.area.y));
        let mut canvas = canvas.clipped(hosted.area).offset(x, y);
        hosted.screen.draw(&mut canvas, &own);
    }

    pub fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        self.pass(cx, |screen, cx| screen.on_action(action, cx))
    }

    /// Passes `gesture` on where it began in the screen held, in its
    /// coordinates.
    pub fn on_gesture(&mut self, mut gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        let Some(area) = self.area().filter(|area| area.contains(gesture.start)) else {
            return Handled::No;
        };
        let (dx, dy) = (i32::from(area.x), i32::from(area.y));
        gesture.start.x -= dx;
        gesture.start.y -= dy;
        gesture.end.x -= dx;
        gesture.end.y -= dy;
        self.pass(cx, |screen, cx| screen.on_gesture(gesture, cx))
    }

    /// Passes a pointer sample on, in the screen held's coordinates; a
    /// finger coming down outside its part is not its own.
    pub fn on_pointer(&mut self, event: &PointerEvent, cx: &mut Cx<'_, '_>) -> Handled {
        let Some(area) = self.area() else {
            return Handled::No;
        };
        if event.phase == paperpaw_platform::PointerPhase::Down && !area.contains(event.point) {
            return Handled::No;
        }
        let mut event = *event;
        event.point.x -= i32::from(area.x);
        event.point.y -= i32::from(area.y);
        self.pass(cx, |screen, cx| screen.on_pointer(&event, cx))
    }

    pub fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        self.pass(cx, |screen, cx| {
            screen.on_event(event, params, cx);
            Handled::Yes
        });
    }

    pub fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        self.pass(cx, |screen, cx| {
            screen.on_show(cx);
            Handled::Yes
        });
    }

    pub fn on_timer(&mut self, cx: &mut Cx<'_, '_>) {
        self.pass(cx, |screen, cx| {
            screen.on_timer(cx);
            Handled::Yes
        });
    }

    pub fn on_hide(&mut self) {
        if let Some(hosted) = &mut self.hosted {
            hosted.screen.on_hide();
        }
    }
}
