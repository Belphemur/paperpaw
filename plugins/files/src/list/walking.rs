//! The library while the kernel makes its catalog anew, and looking for
//! new books: the books found so far counted as such (where there was no
//! catalog, the kernel shows them as it goes on), and an action that has
//! the kernel walk the library folder again, offered on an empty library
//! (books copied onto the card while it was in a computer).

use alloc::string::String;

use paperpaw_sdk::{
    Command, Commands, Cx, Env, FixedAction, Handled, Id, Instant, Params, Rect, library,
};

/// Looks for new books: the kernel walks the library folder anew.
pub(crate) const LOOK_AGAIN: Id = Id::new_static("files.look-again");

/// The key of [`library::Library::extended`] the kernel's library answers
/// with whether it is making the catalog anew (`walking`).
const WALKING: Id = Id::new_static("core.library-walking");

/// How long after looking for new books the library makes sure the
/// kernel began (in milliseconds): it does at once, and tells the view
/// when it ends or fails.
const BEGUN_MS: u64 = 5_000;

/// The action of [`LOOK_AGAIN`].
pub(crate) const ACTION: FixedAction =
    FixedAction::new(LOOK_AGAIN, "files.look-again").handler(look_again);

fn look_again(env: &Env<'_>, _: &Params, out: &mut Commands) {
    // A second press while the books are still being collected lets the
    // walk go on: walked anew, it would start over.
    let (request, said) = if walking(env) {
        (library::request_catalog(), "files.look-again.busy")
    } else {
        (library::request_catalog_anew(), "files.look-again.started")
    };
    out.push(request);
    out.push(Command::Notify(env.i18n().tr(said).into()));
}

impl super::ListScreen {
    /// Looking for new books (the button or the menu, before the action's
    /// own handler, which it passes on to): an empty library says it
    /// collects the books from now on, rather than that there are none,
    /// until the kernel shows the first or says it is done.
    pub(super) fn looking(&mut self, cx: &mut Cx<'_, '_>) -> Handled {
        if self.look_again {
            self.collecting = true;
            self.refilter(cx.env());
            cx.request_redraw();
            cx.wake_at(Instant(cx.env().now().millis() + BEGUN_MS));
        }
        Handled::No
    }

    /// A while after looking for new books: where no catalog is being
    /// made (the kernel never began, as where another handler took the
    /// action), the library says again that it has none, not that it
    /// collects them.
    pub(super) fn looked(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        if self.collecting && !super::collecting(&env) && !walking(&env) {
            self.collecting = false;
            self.refilter(&env);
            cx.request_redraw();
        }
    }
}

/// Whether the library's catalog is being made anew: its books are those
/// found so far.
pub(crate) fn walking(env: &Env<'_>) -> bool {
    library::service(env)
        .extended(env, &WALKING, &Params::new())
        .is_some_and(|answer| answer.get("walking") == Some("true"))
}

/// How many books the filter's row says there are: `count` so far while
/// the catalog is being made anew.
pub(crate) fn count(env: &Env<'_>, count: usize) -> String {
    let key = if walking(env) {
        "files.list.so-far"
    } else {
        "files.books"
    };
    env.i18n().plural(key, count as i64, &[])
}

/// Where the button that looks for new books goes in an empty list's
/// `area`, and what is left above it for what the empty list says.
pub(crate) fn button(area: Rect, row: u16, margin: u16) -> (Rect, Rect) {
    let height = row.min(area.height / 3);
    let width = (area.width.saturating_sub(2 * margin)).min(area.width * 3 / 4);
    let y = area.y + area.height.saturating_sub(height + margin);
    let button = Rect::new(area.x + (area.width - width) / 2, y, width, height);
    let above = Rect::new(area.x, area.y, area.width, y.saturating_sub(area.y));
    (button, above)
}
