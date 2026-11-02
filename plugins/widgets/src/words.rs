//! Words and one's own: the quote of the day (`quote`), a picture from
//! the card (`picture`), a note (`note`), a to-do list (`todo`) and a
//! greeting by the time of day (`greeting`); PaperPaw's logo (`logo`).
//!
//! What a widget keeps of its own (its own quotes, the to-do list, which
//! picture or quote is turned to) is in the plugin's folder,
//! `.paperpaw/plugins/widgets`. A tap that changes what a widget shows
//! (the next quote, an item ticked off) writes it there and has the
//! screen read its widgets anew.

mod greeting;
mod logo;
mod note;
mod picture;
mod quote;
mod text;
mod todo;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::Cell;

use paperpaw_sdk::widgets;
use paperpaw_sdk::{Canvas, Commands, Env, Id, RegisterError, Registrar, Storage};

use crate::{frame_height, today};

pub(crate) fn register(r: &mut Registrar) -> Result<(), RegisterError> {
    let changes = Changes::default();
    quote::register(r, &changes)?;
    picture::register(r, &changes)?;
    note::register(r)?;
    todo::register(r, &changes)?;
    greeting::register(r)?;
    logo::register(r)?;
    Ok(())
}

/// The path of `name` in the plugin's folder.
fn data(name: &str) -> String {
    format!(
        "{}/{name}",
        paperpaw_sdk::Env::plugin_dir(crate::PLUGIN.as_str())
    )
}

/// Counts the changes of what these widgets keep (see
/// [`Changes::changed`]): a widget that read it at another count reads it
/// anew. Shared by the handlers and the widgets (on the heap: a plugin
/// file has no writable data of its own, ADR 0024).
#[derive(Debug, Clone, Default)]
struct Changes(Rc<Cell<u32>>);

impl Changes {
    /// The changes so far.
    fn count(&self) -> u32 {
        self.0.get()
    }

    /// What widget `id` keeps changed (a quote turned, an item ticked
    /// off): the screens showing it read it anew.
    fn changed(&self, out: &mut Commands, id: &Id) {
        self.0.set(self.0.get().wrapping_add(1));
        out.push(widgets::changed(id));
    }
}

/// The one of `count` things shown on `day`: each of them once before
/// any comes again, neighbours far apart (the order of a file says little
/// about the order of the days).
fn of_the_day(day: i64, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    // A prime: with any count it does not divide, a step that reaches
    // every one.
    const STEP: u64 = 7919;
    let step = if (count as u64).is_multiple_of(STEP) {
        1
    } else {
        STEP
    };
    let day = day.rem_euclid(count as i64) as u64;
    (day * step % count as u64) as usize
}

/// How far a widget was turned on (to the next quote, the next picture),
/// kept in its file: the day it was turned on and how often since.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Turns {
    day: i64,
    turns: u32,
}

impl Turns {
    /// As kept at `path` (`<day> <turns>`); none if not kept.
    fn load(storage: &dyn Storage, path: &str) -> Self {
        let Ok(data) = paperpaw_sdk::read_file(storage, path, 64) else {
            return Self::default();
        };
        let text = String::from_utf8_lossy(&data);
        let mut numbers = text.split_whitespace();
        let day = numbers.next().and_then(|d| d.parse().ok()).unwrap_or(0);
        let turns = numbers.next().and_then(|t| t.parse().ok()).unwrap_or(0);
        Self { day, turns }
    }

    fn save(&self, storage: &dyn Storage, path: &str) {
        let _ = storage.write(path, format!("{} {}\n", self.day, self.turns).as_bytes());
    }

    /// The turns of `today`: none since the day began, where what is shown
    /// changes daily.
    fn on(self, today: i64, daily: bool) -> u32 {
        if daily && self.day != today {
            0
        } else {
            self.turns
        }
    }

    /// Turned on once more on `today`, kept at `path`.
    fn turn(storage: &dyn Storage, path: &str, today: i64, daily: bool) {
        let turns = Self::load(storage, path).on(today, daily);
        Self {
            day: today,
            turns: turns.wrapping_add(1),
        }
        .save(storage, path);
    }
}

/// Draws `text` in the middle of `look`'s area, e.g. how to add what
/// a widget would show: in the body size where it fits, else smaller
/// down to the small size, broken between words only
/// ([`widgets::draw_message`]).
/// Nothing where not even a line has room.
fn draw_empty(canvas: &mut Canvas<'_>, env: &Env<'_>, look: &widgets::WidgetLook, text: &str) {
    let small = paperpaw_sdk::ui::UiStyle::new(env).small();
    if widgets::line_height(env, small) <= look.area.height {
        widgets::draw_message(canvas, env, look, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::MemoryStorage;

    #[test]
    fn each_day_has_its_own_until_all_were_shown() {
        for count in [1usize, 2, 7, 60, 86, 7919] {
            let mut seen = alloc::vec![false; count];
            for day in 20_000..20_000 + count as i64 {
                let pick = of_the_day(day, count);
                assert!(pick < count);
                assert!(!seen[pick], "{count}: {pick} twice");
                seen[pick] = true;
            }
        }
        // The same each time on the same day; another the next.
        assert_eq!(of_the_day(20_375, 86), of_the_day(20_375, 86));
        assert_ne!(of_the_day(20_375, 86), of_the_day(20_376, 86));
        assert_eq!(of_the_day(-3, 5), of_the_day(2, 5));
        assert_eq!(of_the_day(5, 0), 0);
    }

    /// What version 1 wrote stays readable (see `docs/data-files.md`).
    #[test]
    fn turns_of_version_1_are_read() {
        let storage = MemoryStorage::new().with("t.txt", include_str!("../testdata/turns.txt.v1"));
        let turns = Turns::load(&storage, "t.txt");
        assert_eq!(
            turns,
            Turns {
                day: 20_375,
                turns: 3
            }
        );
    }

    #[test]
    fn turns_begin_again_each_day() {
        let storage = MemoryStorage::new();
        let path = data("turns.txt");
        Turns::turn(&storage, &path, 100, true);
        Turns::turn(&storage, &path, 100, true);
        assert_eq!(Turns::load(&storage, &path).on(100, true), 2);
        assert_eq!(Turns::load(&storage, &path).on(101, true), 0);
        // Not daily: they go on counting.
        assert_eq!(Turns::load(&storage, &path).on(101, false), 2);
        Turns::turn(&storage, &path, 101, true);
        assert_eq!(Turns::load(&storage, &path), Turns { day: 101, turns: 1 });
    }
}
