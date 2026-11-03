//! Status items "Time left in chapter" and "Time left in book": the
//! pages still to read at one's reading speed, as the reader's status
//! lines show them (the room the reader keeps for them, see the SDK's
//! `StatusRoom`).
//!
//! The speed is CrossPoint's: the words a minute of the book read now
//! (its window whole, and only then), or of all reading, turned into
//! seconds a page by the words of the page shown — a dense page is read
//! longer than a sparse one, and a page without text goes by as many
//! words as a page is calibrated to have. Where no window is full, all
//! reading over the log stands in, once enough pages have been turned.
//! Where there is neither, the item is left out: no number is made up.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use paperpaw_sdk::builtin::{events, params};
use paperpaw_sdk::status::{StatusItemSpec, StatusValue, context};
use paperpaw_sdk::{Arg, Env, Id, Params, RegisterError, Registrar};

use crate::pace::{self, Pace, SharedPace};
use crate::{PAGE_WORDS, fmt, store};

pub(crate) const CHAPTER: Id = Id::new_static("stats.status.left-chapter");
pub(crate) const BOOK: Id = Id::new_static("stats.status.left-book");
/// Pages turned before all reading stands in for a speed (CrossPoint's
/// own threshold).
const MIN_GLOBAL_PAGES: u32 = 50;
/// Pages read before there is a speed to go by, when it is worked out
/// from the log.
const MIN_PAGES: u32 = 10;
/// Pages turned after which the speed is worked out again.
const RECOUNT: u32 = 50;

/// Seconds a page, from the words a minute and the words of the page
/// shown: a page of `words` words read at `wpm` words a minute. A page
/// without text goes by as many words as a page is calibrated to have.
fn from_wpm(wpm: u16, words_on_page: Option<u32>) -> f32 {
    let words = words_on_page
        .filter(|&words| words > 0)
        .unwrap_or(pace::CALIBRATED_WORDS_PER_PAGE);
    (u64::from(words) * 60) as f32 / f32::from(wpm)
}

/// Seconds a page, CrossPoint's chain: the words a minute of the book
/// read now, then of all reading, each only where its window is whole;
/// else all reading over the log, once enough of it there is. `None`
/// where there is neither: no number is made up.
fn pace_seconds(
    env: &Env<'_>,
    pace: &Pace,
    book: Option<u16>,
    words_on_page: Option<u32>,
) -> Option<f32> {
    if let Some(wpm) = pace.full_wpm(book).or_else(|| pace.full_wpm(None)) {
        return Some(from_wpm(wpm, words_on_page));
    }
    whole_log(env)
}

/// Seconds a page over all reading in the log, a page a stay, once
/// enough pages have been turned. Where the log cannot be read to the
/// end, that of what could be.
fn whole_log(env: &Env<'_>) -> Option<f32> {
    let (mut seconds, mut pages) = (0u64, 0u32);
    #[expect(
        clippy::let_underscore_must_use,
        reason = "a log that cannot be read to the end counts what could be"
    )]
    let _ = store::scan(env.storage(), |stat| {
        seconds += u64::from(stat.duration);
        pages += 1;
    });
    (pages >= MIN_GLOBAL_PAGES).then(|| seconds as f32 / pages as f32)
}

/// Pages left in the book (`chapter` false) or in the chapter, from the
/// reader's context: the pages counted, the progress and where the
/// chapters begin.
fn pages_left(context: &Params, chapter: bool) -> Option<f32> {
    let pages: f32 = context.get(params::PAGES)?.parse().ok()?;
    let progress: f32 = context.get(params::PROGRESS)?.parse().ok()?;
    let at = progress * 10.0;
    let end = if chapter {
        context
            .get(context::CHAPTERS)
            .unwrap_or_default()
            .split(',')
            .filter_map(|mark| mark.trim().parse::<f32>().ok())
            .find(|&mark| mark > at)
            .unwrap_or(1000.0)
    } else {
        1000.0
    };
    Some(((end - at) / 1000.0 * pages).max(0.0))
}

/// The reading speed, worked out when a book is opened, when the reader
/// says the words of a page, and now and then while reading: never as
/// status lines are drawn.
#[derive(Debug, Default)]
struct Speed {
    /// Seconds a page, once worked out (`Some(None)`: too little read).
    known: Option<Option<f32>>,
    /// The book it was worked out for.
    book: String,
    /// Its id in the books, where it has one.
    id: Option<u16>,
    /// Pages turned since.
    turns: u32,
}

impl Speed {
    /// Whether the speed is to be worked out anew as the book at `path`
    /// is shown: not when the same book shows again (the reader says so
    /// after every menu).
    fn opened(&mut self, path: &str) -> bool {
        if self.known.is_some() && self.book == path {
            return false;
        }
        self.book = String::from(path);
        self.turns = 0;
        true
    }

    /// Whether the speed is to be worked out anew after a page was
    /// turned: every [`RECOUNT`] pages, every [`MIN_PAGES`] while there
    /// is none.
    fn turned(&mut self) -> bool {
        self.turns = self.turns.wrapping_add(1);
        let every = match self.known {
            Some(Some(_)) => RECOUNT,
            _ => MIN_PAGES,
        };
        self.turns.is_multiple_of(every)
    }

    /// Whether the speed is to be worked out anew, now that the words of
    /// the page shown are known: where the pace is told by the words a
    /// minute, every page's words change it; where it is told by the
    /// log, they change nothing.
    fn said(&mut self, from_wpm: bool) -> bool {
        from_wpm && self.known.is_some()
    }

    /// Works the speed out: the book at `path` is shown.
    fn recount(&mut self, env: &Env<'_>, pace: &Pace) {
        let book = store::books(env.storage())
            .into_iter()
            .find(|b| b.path == self.book)
            .map(|b| b.id);
        self.id = book;
        self.known = Some(pace_seconds(env, pace, book, pace.page_words(&self.book)));
    }
}

pub(crate) fn register(r: &mut Registrar, pace: SharedPace) -> Result<(), RegisterError> {
    let shared: Rc<RefCell<Speed>> = Rc::default();
    let s = shared.clone();
    let p = pace.clone();
    r.on(events::BOOK_OPENED, move |env, reading, _| {
        let mut s = s.borrow_mut();
        if s.opened(&reading.path) {
            s.recount(env, &p.borrow());
        }
    })?;
    let s = shared.clone();
    let p = pace.clone();
    r.on(events::PAGE_TURNED, move |env, _, _| {
        let mut s = s.borrow_mut();
        if s.turned() {
            s.recount(env, &p.borrow());
        }
    })?;
    // The words of the page shown: where the pace is told by the words
    // a minute, the time left changes with them; where it is told by
    // the log, they change nothing and are not worked out for.
    let s = shared.clone();
    let p = pace.clone();
    r.on(PAGE_WORDS, move |env, _, _| {
        let mut s = s.borrow_mut();
        if s.known.is_none() {
            return;
        }
        let from_wpm = {
            let pace = p.borrow();
            pace.full_wpm(None).is_some()
                || s.id.is_some_and(|id| pace.full_wpm(Some(id)).is_some())
        };
        if s.said(from_wpm) {
            s.recount(env, &p.borrow());
        }
    })?;
    for (id, title, key, chapter) in [
        (
            CHAPTER,
            "stats.status.left-chapter",
            "stats.left.chapter",
            true,
        ),
        (BOOK, "stats.status.left-book", "stats.left.book", false),
    ] {
        let s = shared.clone();
        r.add_status_item(StatusItemSpec::new(
            id,
            title,
            Box::new(move |env, context| {
                let speed = s.borrow().known.flatten()?;
                let seconds = (pages_left(context, chapter)? * speed) as u64;
                let time = fmt::duration(env, seconds.max(60));
                Some(StatusValue::Text(
                    env.i18n().format(key, &[("time", Arg::Str(&time))]),
                ))
            }),
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_left_to_the_next_chapter() {
        let context = Params::new()
            .with(params::PAGES, "200")
            .with(params::PROGRESS, "25.0")
            .with(context::CHAPTERS, "0,100,300,700");
        assert_eq!(pages_left(&context, false), Some(150.0));
        // From 250 to 300 thousandths: 10 of 200 pages.
        assert_eq!(pages_left(&context, true), Some(10.0));
        // In the last chapter: to the end.
        let last = context.with(params::PROGRESS, "80.0");
        assert_eq!(pages_left(&last, true), Some(40.0));
        // Pages not counted yet: nothing to say.
        assert_eq!(
            pages_left(&Params::new().with(params::PROGRESS, "5"), false),
            None
        );
    }

    #[test]
    fn the_speed_is_worked_out_per_book_not_per_showing() {
        let mut speed = Speed::default();
        assert!(speed.opened("books/a.epub"));
        speed.known = Some(Some(30.0));
        // The same book after a menu: the speed stays.
        assert!(!speed.opened("books/a.epub"));
        assert!(speed.opened("books/b.epub"));
        speed.known = Some(Some(30.0));
        let recounts = (0..RECOUNT * 2).filter(|_| speed.turned()).count();
        assert_eq!(recounts, 2);
        // Without a speed yet, more often, but not at every page.
        speed.known = Some(None);
        let recounts = (0..MIN_PAGES * 2).filter(|_| speed.turned()).count();
        assert_eq!(recounts, 2);
    }

    #[test]
    fn a_page_of_words_makes_seconds_of_the_words_a_minute() {
        // 300 words at 300 a minute: a minute.
        assert_eq!(from_wpm(300, Some(300)), 60.0);
        // A dense page takes longer, a sparse one less.
        assert!(from_wpm(300, Some(600)) > from_wpm(300, Some(150)));
        // A page without words: as many as a page is calibrated to have.
        assert_eq!(from_wpm(300, Some(0)), from_wpm(300, None));
        assert_eq!(from_wpm(300, None), from_wpm(300, Some(220)));
    }
}
