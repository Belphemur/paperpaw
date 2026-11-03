//! How fast and how long one reads: the words a minute of the last pages
//! and how long a stretch of reading lasts, as CrossPoint shows them.
//!
//! Both are a window of samples, the odd ones trimmed away: the words a
//! minute of the last pages read, and the length of the last stretches of
//! reading. A sample of the words a minute comes from a stay on a page:
//! how long it lasted and how many words the page has, which the reader
//! says ([`ask_page_words`]). How long a stretch lasts comes from the
//! log of page stays alone: the stays of a book, grouped by the gaps
//! between them ([`SESSION_GAP`]). Nothing of either is kept in a file of
//! its own: the stretches are worked out from the log when they are
//! shown, and the windows of words a minute live while the plugin runs,
//! seeded by the import of CrossPoint's (see `xcpt`).
//!
//! [`ask_page_words`]: paperpaw_sdk::reading::ask_page_words

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

/// The pace, shared between the recording and the screens: kept while
/// the plugin runs, seeded by the import of CrossPoint's data.
pub(crate) type SharedPace = Rc<RefCell<Pace>>;

/// How many samples of words a minute the window keeps.
pub(crate) const WPM_WINDOW: usize = 15;
/// How many of the fastest and slowest samples are left out of the
/// average.
pub(crate) const WPM_TRIM: usize = 2;
/// A sample faster than this is not a plausible reading of a page: left
/// out whole, not clamped.
pub(crate) const WPM_CAP: u32 = 900;
/// A sample slower than this is clamped up to it: a reader is never shown
/// as slower than this.
pub(crate) const WPM_FLOOR: u32 = 80;

/// The words a minute of the last pages read: a window of samples with
/// the average over the middle of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WpmWindow {
    samples: [u16; WPM_WINDOW],
    avg: u16,
    /// Where the next sample goes.
    pos: u8,
    /// How many samples the window holds, at most [`WPM_WINDOW`].
    count: u8,
}

impl Default for WpmWindow {
    fn default() -> Self {
        Self {
            samples: [0; WPM_WINDOW],
            avg: 0,
            pos: 0,
            count: 0,
        }
    }
}

impl WpmWindow {
    /// The average so far, 0 where nothing was read.
    #[cfg(test)]
    pub(crate) fn avg(&self) -> u16 {
        self.avg
    }

    /// How many samples the window holds.
    pub(crate) fn count(&self) -> u8 {
        self.count
    }

    /// The words a minute, or none where nothing was read.
    pub(crate) fn value(&self) -> Option<u16> {
        (self.avg > 0).then_some(self.avg)
    }

    /// Where the next sample goes.
    #[cfg(test)]
    pub(crate) fn pos(&self) -> u8 {
        self.pos
    }

    /// Takes a page read: `seconds` on a page of `words` words. A page
    /// read for no time, or without words, is no sample; one faster than
    /// the cap is left out; one slower than the floor counts as the floor.
    pub(crate) fn record(&mut self, seconds: u16, words: u32) {
        if seconds == 0 || words == 0 {
            return;
        }
        let wpm = words.saturating_mul(60) / u32::from(seconds);
        if wpm > WPM_CAP {
            return;
        }
        self.samples[usize::from(self.pos)] = wpm.max(WPM_FLOOR).min(u32::from(u16::MAX)) as u16;
        self.pos = (self.pos + 1) % WPM_WINDOW as u8;
        if self.count < WPM_WINDOW as u8 {
            self.count += 1;
        }
        self.avg = self.trimmed_mean();
    }

    /// The average over the middle of the samples: the [`WPM_TRIM`]
    /// fastest and slowest left out where there are enough to trim, all
    /// of them averaged while there are not.
    fn trimmed_mean(&self) -> u16 {
        let count = usize::from(self.count);
        if count == 0 {
            return 0;
        }
        let mut sorted = Vec::with_capacity(count);
        sorted.extend_from_slice(&self.samples[..count]);
        sorted.sort_unstable();
        if count <= WPM_TRIM * 2 {
            return (sorted.iter().map(|&s| u32::from(s)).sum::<u32>() / count as u32) as u16;
        }
        let sum: u32 = sorted[WPM_TRIM..count - WPM_TRIM]
            .iter()
            .map(|&s| u32::from(s))
            .sum();
        (sum / (count - WPM_TRIM * 2) as u32) as u16
    }

    /// The window as CrossPoint kept it: the average, how many samples
    /// (two bytes, cut to the window's size), the samples themselves and
    /// where the next goes. What cannot be is normalized (see
    /// [`normalize`](Self::normalize)), for the file may say anything.
    pub(crate) fn from_wire(avg: u16, count: u16, samples: &[u8], pos: u8) -> Self {
        let mut window = Self::default();
        for (at, sample) in window.samples.iter_mut().enumerate() {
            *sample = samples
                .get(at * 2..at * 2 + 2)
                .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]));
        }
        window.avg = avg;
        window.count = count.min(WPM_WINDOW as u16) as u8;
        window.pos = pos;
        window.normalize();
        window
    }

    /// Makes a window read from bytes sound: a count larger than the
    /// window holds is cut to it; a window that is not full has its next
    /// sample where its samples end (so that the next one recorded goes
    /// there and not over a sample kept); a full window's cursor that
    /// points outside it goes to the start; and the average is worked out
    /// from the samples, never taken from the file.
    pub(crate) fn normalize(&mut self) {
        if usize::from(self.count) < WPM_WINDOW {
            self.pos = self.count;
        } else if usize::from(self.pos) >= WPM_WINDOW {
            self.pos = 0;
        }
        self.avg = self.trimmed_mean();
    }
}

/// How many samples the window of session lengths keeps.
pub(crate) const SESSION_WINDOW: usize = 10;
/// How many of the shortest and longest sessions are left out of the
/// average.
pub(crate) const SESSION_TRIM: usize = 2;
/// A stretch shorter than this many seconds is no session.
pub(crate) const SESSION_MIN_SECONDS: u32 = 30;
/// A stretch with fewer page turns than this is no session.
pub(crate) const SESSION_MIN_TURNS: u32 = 2;
/// The average of the window is shown once this many sessions are in it;
/// before that, the average over all of them.
pub(crate) const SESSIONS_BEFORE: u8 = 4;
/// Seconds a session lasts at most, as they are kept.
pub(crate) const SESSION_MAX: u32 = u16::MAX as u32;

/// How long a stretch of reading lasted: the seconds it covers and the
/// pages turned in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Session {
    pub seconds: u64,
    pub turns: u32,
}

/// A gap of this many seconds between two stays on pages ends a stretch
/// of reading: the reader went away and came back. A page may be read
/// longer than the log counts it for, so a pause shorter than this is
/// still one stretch, and one longer is a new one. (CrossPoint counted a
/// stretch from opening a book to leaving it; PaperPaw's log has no such
/// marker, so the gap between pages stands in for it.)
pub(crate) const SESSION_GAP: u32 = 300;

/// The stretches of reading in `stays`, stays on pages in the order of
/// the log (a stay that started before the one before it may sit among
/// them, where the clock was set back). A stay begins a new stretch when
/// it starts [`SESSION_GAP`] seconds or more after the one before ended;
/// the seconds and the page turns of the stays between are one stretch.
pub(crate) fn sessions(stays: &[Stay]) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    let mut last_end: Option<u32> = None;
    for stay in stays {
        let new = last_end.is_none_or(|last| stay.start.saturating_sub(last) >= SESSION_GAP);
        if new {
            out.push(Session {
                seconds: 0,
                turns: 0,
            });
        }
        if let Some(session) = out.last_mut() {
            session.seconds = session.seconds.saturating_add(u64::from(stay.duration));
            session.turns += 1;
        }
        // The clock may have been set back: the end of the last stay
        // never goes past the end of time.
        last_end = Some(
            u32::try_from(u64::from(stay.start) + u64::from(stay.duration)).unwrap_or(u32::MAX),
        );
    }
    out
}

/// A stay on a page, as the log keeps it: when it began and how long it
/// counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stay {
    /// Unix seconds.
    pub start: u32,
    /// Seconds.
    pub duration: u16,
}

/// The window of the last session lengths, from the sessions in the order
/// they came: the ones too short or too still to count are left out, and
/// a longer one than a session is kept counts as long as a session can.
pub(crate) fn session_window(sessions: &[Session]) -> super::pace::SessionWindow {
    let mut window = SessionWindow::default();
    for session in sessions {
        window.record(
            session.seconds.min(SESSION_MAX as u64) as u32,
            session.turns,
        );
    }
    window
}

/// How long the sessions were, as CrossPoint kept them: a window of
/// samples with the average over the middle of them. A session shorter
/// than [`SESSION_MIN_SECONDS`] or with fewer page turns than
/// [`SESSION_MIN_TURNS`] is no sample; one longer than a session is kept
/// counts as long as a session can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionWindow {
    samples: [u16; SESSION_WINDOW],
    avg: u16,
    pos: u8,
    count: u8,
}

impl Default for SessionWindow {
    fn default() -> Self {
        Self {
            samples: [0; SESSION_WINDOW],
            avg: 0,
            pos: 0,
            count: 0,
        }
    }
}

impl SessionWindow {
    /// The average so far, 0 where there is no session.
    pub(crate) fn avg(&self) -> u16 {
        self.avg
    }

    /// How many sessions the window holds.
    pub(crate) fn count(&self) -> u8 {
        self.count
    }

    /// Where the next session goes.
    #[cfg(test)]
    pub(crate) fn pos(&self) -> u8 {
        self.pos
    }

    /// Takes a stretch of reading.
    pub(crate) fn record(&mut self, seconds: u32, turns: u32) {
        if seconds < SESSION_MIN_SECONDS || turns < SESSION_MIN_TURNS {
            return;
        }
        self.samples[usize::from(self.pos)] = seconds.min(SESSION_MAX) as u16;
        self.pos = (self.pos + 1) % SESSION_WINDOW as u8;
        if self.count < SESSION_WINDOW as u8 {
            self.count += 1;
        }
        self.avg = self.trimmed_mean();
    }

    /// The average over the middle of the sessions, as the words a
    /// minute's average is.
    fn trimmed_mean(&self) -> u16 {
        let count = usize::from(self.count);
        if count == 0 {
            return 0;
        }
        let mut sorted = Vec::with_capacity(count);
        sorted.extend_from_slice(&self.samples[..count]);
        sorted.sort_unstable();
        if count <= SESSION_TRIM * 2 {
            return (sorted.iter().map(|&s| u32::from(s)).sum::<u32>() / count as u32) as u16;
        }
        let sum: u32 = sorted[SESSION_TRIM..count - SESSION_TRIM]
            .iter()
            .map(|&s| u32::from(s))
            .sum();
        (sum / (count - SESSION_TRIM * 2) as u32) as u16
    }

    /// The window as CrossPoint kept it, normalized as the words a
    /// minute's is (see [`WpmWindow::normalize`]).
    pub(crate) fn from_wire(avg: u16, count: u16, samples: &[u8], pos: u8) -> Self {
        let mut window = Self::default();
        for (at, sample) in window.samples.iter_mut().enumerate() {
            *sample = samples
                .get(at * 2..at * 2 + 2)
                .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]));
        }
        window.avg = avg;
        window.count = count.min(SESSION_WINDOW as u16) as u8;
        window.pos = pos;
        window.normalize();
        window
    }

    pub(crate) fn normalize(&mut self) {
        if usize::from(self.count) < SESSION_WINDOW {
            self.pos = self.count;
        } else if usize::from(self.pos) >= SESSION_WINDOW {
            self.pos = 0;
        }
        self.avg = self.trimmed_mean();
    }
}

/// A session had to last this long before CrossPoint counted it in what
/// came before its window (its count of sessions had its own gate, a
/// minute long).
pub(crate) const SESSION_LEGACY: u32 = 60;

/// The window of the sessions of `stays`, and the average over all the
/// sessions that counted before there is a window to show (a minute or
/// longer, as CrossPoint counted them).
pub(crate) fn session_metric(stays: &[Stay]) -> (SessionWindow, Option<u32>) {
    let all = sessions(stays);
    let window = session_window(&all);
    let counted: Vec<u64> = all
        .iter()
        .filter(|session| session.seconds >= u64::from(SESSION_LEGACY))
        .map(|session| session.seconds)
        .collect();
    let mean = (!counted.is_empty()).then(|| {
        (counted.iter().sum::<u64>() / counted.len() as u64).min(u64::from(u32::MAX)) as u32
    });
    (window, mean)
}

/// How long a session lasted, from the log and what the import brought:
/// the window of the last sessions of the log once enough of them are
/// in, then the window the import brought where it has enough, then the
/// average over the sessions the log holds, then the one the import
/// brought, and none where there is neither.
pub(crate) fn session_shown(
    own: &SessionWindow,
    own_mean: Option<u32>,
    seeded: Option<&SessionWindow>,
    seeded_mean: Option<u32>,
) -> Option<u32> {
    if own.count() >= SESSIONS_BEFORE {
        return Some(u32::from(own.avg()));
    }
    if let Some(avg) = seeded
        .filter(|window| window.count() >= SESSIONS_BEFORE)
        .map(|window| u32::from(window.avg()))
    {
        return Some(avg);
    }
    own_mean.or(seeded_mean).filter(|mean| *mean > 0)
}

/// The stays on pages of one book, or of all of them, in the order of
/// the log. Where the log cannot be read to the end, those that could
/// be.
pub(crate) fn stays_of(storage: &dyn paperpaw_sdk::Storage, book: Option<u16>) -> Vec<Stay> {
    let mut stays = Vec::new();
    #[expect(
        clippy::let_underscore_must_use,
        reason = "a log that cannot be read to the end counts what could be"
    )]
    let _ = crate::store::scan(storage, |stat| {
        if book.is_none_or(|id| stat.book == id) {
            stays.push(Stay {
                start: stat.start,
                duration: stat.duration,
            });
        }
    });
    stays
}

/// What a record said of the reading it belongs to, as the import
/// brought it: the windows of words a minute and of session lengths,
/// and what came before them (the seconds and the sessions summed).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FromRecord {
    pub wpm: WpmWindow,
    pub session: SessionWindow,
    pub seconds: u64,
    pub sessions: u32,
}

/// The average before there is a window, as a record said it: the
/// seconds over the sessions, where there were any and it is a minute
/// or longer.
fn record_mean(record: &FromRecord) -> Option<u32> {
    (record.sessions > 0)
        .then(|| (record.seconds / u64::from(record.sessions)).min(u64::from(u32::MAX)) as u32)
        .filter(|mean| *mean >= SESSION_LEGACY)
}

/// The pace the plugin keeps while it runs: the words a minute of the
/// pages read (which the log alone cannot say, for it does not keep how
/// many words a page has), and what the import of CrossPoint's data
/// brought for how long the sessions were.
#[derive(Debug, Default)]
pub(crate) struct Pace {
    global_wpm: WpmWindow,
    books_wpm: Vec<(u16, WpmWindow)>,
    global_session: Option<SessionWindow>,
    books_session: Vec<(u16, SessionWindow)>,
    global_mean: Option<u32>,
    books_mean: Vec<(u16, u32)>,
}

impl Pace {
    /// A page read: its words a minute go into the window of its book
    /// and of all books.
    pub(crate) fn record_wpm(&mut self, book: Option<u16>, seconds: u16, words: u32) {
        self.global_wpm.record(seconds, words);
        let Some(book) = book else { return };
        match self.books_wpm.iter_mut().find(|(id, _)| *id == book) {
            Some((_, window)) => window.record(seconds, words),
            None => {
                let mut window = WpmWindow::default();
                window.record(seconds, words);
                self.books_wpm.push((book, window));
            }
        }
    }

    /// The words a minute of a book, or of all books, as read so far.
    pub(crate) fn wpm(&self, book: Option<u16>) -> Option<u16> {
        match book {
            Some(id) => self
                .books_wpm
                .iter()
                .find(|(b, _)| *b == id)
                .and_then(|(_, window)| window.value()),
            None => self.global_wpm.value(),
        }
    }

    /// How long a session lasted, of a book or of all books (see
    /// [`session_shown`]).
    pub(crate) fn avg_session(
        &self,
        storage: &dyn paperpaw_sdk::Storage,
        book: Option<u16>,
    ) -> Option<u32> {
        let seeded = match book {
            Some(id) => self
                .books_session
                .iter()
                .find(|(b, _)| *b == id)
                .map(|(_, window)| window),
            None => self.global_session.as_ref(),
        };
        let seeded_mean = match book {
            Some(id) => self
                .books_mean
                .iter()
                .find(|(b, _)| *b == id)
                .map(|(_, m)| *m),
            None => self.global_mean,
        };
        let (own, own_mean) = session_metric(&stays_of(storage, book));
        session_shown(&own, own_mean, seeded, seeded_mean)
    }

    /// Takes what the import brought: the windows of words a minute and
    /// of session lengths of the books it matched (by their path, which
    /// `id_of` turns into a book's id) and of all books, and what the
    /// records said of how long a session was before there was a
    /// window.
    pub(crate) fn seed(
        &mut self,
        seeded: &[(String, FromRecord)],
        global: Option<&FromRecord>,
        id_of: impl Fn(&str) -> Option<u16>,
    ) {
        for (path, record) in seeded {
            let Some(id) = id_of(path) else { continue };
            self.take_book(id, record);
        }
        if let Some(global) = global {
            if global.wpm.count() > 0 {
                self.global_wpm = global.wpm;
            }
            if global.session.count() > 0 {
                self.global_session = Some(global.session);
            }
            self.global_mean = record_mean(global).or(self.global_mean);
        }
    }

    /// Takes one book's record into the pace, as the import brought it.
    fn take_book(&mut self, id: u16, record: &FromRecord) {
        if record.wpm.count() > 0 {
            match self.books_wpm.iter_mut().find(|(b, _)| *b == id) {
                Some((_, window)) => *window = record.wpm,
                None => self.books_wpm.push((id, record.wpm)),
            }
        }
        if record.session.count() > 0 {
            match self.books_session.iter_mut().find(|(b, _)| *b == id) {
                Some((_, window)) => *window = record.session,
                None => self.books_session.push((id, record.session)),
            }
        }
        if let Some(mean) = record_mean(record) {
            match self.books_mean.iter_mut().find(|(b, _)| *b == id) {
                Some((_, there)) => *there = mean,
                None => self.books_mean.push((id, mean)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(samples: &[u32]) -> WpmWindow {
        let mut w = WpmWindow::default();
        for &sample in samples {
            w.record(60, sample);
        }
        w
    }

    #[test]
    fn samples_are_counted_as_words_a_minute() {
        let mut w = WpmWindow::default();
        // A page without words, or read for no time, is no sample.
        w.record(60, 0);
        w.record(0, 200);
        assert_eq!(w.count(), 0);
        // 200 words in 60 seconds.
        w.record(60, 200);
        assert_eq!(w.avg(), 200);
        // Slower than the floor counts as the floor.
        w.record(600, 20);
        assert_eq!(w.count(), 2);
        assert_eq!(w.avg(), (80 + 200) / 2);
        // Faster than the cap is left out whole.
        w.record(1, 6000);
        assert_eq!(w.count(), 2);
    }

    #[test]
    fn the_window_keeps_the_last_samples_and_trims_the_odd_ones() {
        // Sixteen pages a minute apart: the first has fallen out.
        let samples: Vec<u32> = (0..16).map(|i| 200 + i).collect();
        let w = window(&samples);
        assert_eq!(w.count(), WPM_WINDOW as u8);
        assert_eq!(w.pos(), 1);
        // The two fastest and two slowest of fifteen are left out: the
        // middle eleven run from 203 to 213.
        assert_eq!(w.avg(), 208);
        // Fewer than four samples are all averaged.
        let w = window(&[100, 300, 500]);
        assert_eq!(w.avg(), 300);
        // A window of four is not trimmed yet either.
        let w = window(&[100, 300, 500, 700]);
        assert_eq!(w.avg(), 400);
    }

    /// A window read from bytes that say what they like comes out sound.
    #[test]
    fn a_window_from_bytes_is_normalized() {
        let mut samples = [0u8; 30];
        for (i, sample) in [100u16, 200, 300].iter().enumerate() {
            samples[i * 2..i * 2 + 2].copy_from_slice(&sample.to_le_bytes());
        }
        // A count larger than the window holds, a cursor outside it, and
        // an average that is not the samples'.
        let w = WpmWindow::from_wire(999, 40, &samples, 200);
        assert_eq!(w.count(), WPM_WINDOW as u8);
        assert_eq!(w.pos(), 0);
        // Twelve zeros and three samples, the odd ones trimmed: the
        // middle eleven are ten zeros and the 100.
        assert_eq!(w.avg(), 100 / 11);
        // A window that is not full has its cursor where its samples end.
        let w = WpmWindow::from_wire(0, 3, &samples, 1);
        assert_eq!(w.count(), 3);
        assert_eq!(w.pos(), 3);
        assert_eq!(w.avg(), 200);
        // The next sample recorded goes there, not over a kept one.
        let mut w = w;
        w.record(60, 400);
        assert_eq!(w.avg(), 250);
    }

    fn stays(stays: &[(u32, u16)]) -> Vec<Stay> {
        stays
            .iter()
            .map(|&(start, duration)| Stay { start, duration })
            .collect()
    }

    #[test]
    fn stays_are_grouped_by_their_gaps() {
        // Five pages read one after the other, then a pause of five
        // minutes, then three more.
        let all = stays(&[
            (100, 60),
            (200, 60),
            (300, 60),
            (400, 60),
            (500, 60),
            (3600, 60),
            (3700, 60),
            (3800, 60),
        ]);
        assert_eq!(
            sessions(&all),
            [
                Session {
                    seconds: 300,
                    turns: 5
                },
                Session {
                    seconds: 180,
                    turns: 3
                },
            ]
        );
        // A page read longer than the log counts it for does not end the
        // stretch by itself: the next page follows within the gap.
        let long = stays(&[(0, 120), (180, 120), (360, 120)]);
        assert_eq!(sessions(&long).len(), 1);
        // A pause of exactly the gap ends one.
        let apart = stays(&[(0, 60), (60, 60), (60 + 60 + 300, 60)]);
        assert_eq!(sessions(&apart).len(), 2);
        // Stays out of order (the clock was set back) still group.
        let back = stays(&[(100, 60), (300, 60), (200, 60)]);
        assert_eq!(sessions(&back).len(), 1);
        // Nothing read: nothing grouped.
        assert!(sessions(&[]).is_empty());
    }

    #[test]
    fn sessions_are_gated_and_kept_in_a_window() {
        let mut w = SessionWindow::default();
        // Too short, or too still: no session.
        w.record(29, 5);
        w.record(100, 1);
        assert_eq!(w.count(), 0);
        w.record(100, 3);
        assert_eq!(w.count(), 1);
        assert_eq!(w.avg(), 100);
        // Longer than a session can be kept: it counts as long as one can.
        w.record(u32::MAX, 3);
        assert_eq!(w.count(), 2);
        assert_eq!(u32::from(w.avg()), (100 + SESSION_MAX) / 2);
        // The window keeps the last ten.
        for i in 0..12 {
            w.record(100 + i * 10, 3);
        }
        assert_eq!(w.count(), SESSION_WINDOW as u8);
        // Fourteen recorded, ten kept: the next goes after the last.
        assert_eq!(w.pos(), 4);
    }

    #[test]
    fn a_session_window_from_bytes_is_normalized() {
        let mut samples = [0u8; 20];
        for i in 0..SESSION_WINDOW {
            let sample = 100 + i as u16 * 10;
            samples[i * 2..i * 2 + 2].copy_from_slice(&sample.to_le_bytes());
        }
        let w = SessionWindow::from_wire(999, 99, &samples, 200);
        assert_eq!(w.count(), SESSION_WINDOW as u8);
        assert_eq!(w.pos(), 0);
        // The two shortest (100, 110) and two longest (190, 200) are out:
        // the middle six run from 120 to 170, 145 on average.
        assert_eq!(w.avg(), 145);
        // The average is worked out again, never taken from the file.
        let w = SessionWindow::from_wire(7, 1, &samples, 0);
        assert_eq!(w.avg(), 100);
    }

    #[test]
    fn the_average_is_shown_once_enough_sessions_are_in() {
        let mut w = SessionWindow::default();
        // Nothing yet, and nothing before: nothing to show.
        assert_eq!(session_shown(&w, None, None, None), None);
        // Fewer than four in the window: what came before it.
        assert_eq!(session_shown(&w, Some(600), None, None), Some(600));
        w.record(100, 3);
        w.record(200, 3);
        w.record(300, 3);
        assert_eq!(session_shown(&w, Some(600), None, None), Some(600));
        // The fourth is in: the window's average is shown.
        w.record(400, 3);
        assert_eq!(w.count(), 4);
        assert_eq!(session_shown(&w, Some(600), None, None), Some(250));
        // Nothing comes before the window where nothing was read.
        assert_eq!(session_shown(&w, None, None, None), Some(250));
        // What the import brought shows where the log has not enough of
        // its own yet, and what it said before its window where it has
        // neither.
        let mut from = SessionWindow::default();
        from.record(500, 3);
        from.record(600, 3);
        from.record(700, 3);
        from.record(800, 3);
        assert_eq!(session_shown(&w, Some(600), Some(&from), None), Some(250));
        // A window of the import with too few in it is not shown
        // either: what it said before it is, then.
        let mut few = SessionWindow::default();
        few.record(500, 3);
        few.record(600, 3);
        few.record(700, 3);
        assert_eq!(session_shown(&few, None, Some(&few), Some(650)), Some(650));
        assert_eq!(session_shown(&few, None, Some(&few), None), None);
        let none = SessionWindow::default();
        assert_eq!(session_shown(&none, None, Some(&few), Some(650)), Some(650));
    }

    #[test]
    fn what_came_before_is_the_average_of_the_sessions_that_count() {
        // Two sessions of a minute and longer, one shorter: the two
        // count, the short one does not.
        let stays = stays(&[
            (0, 70),
            (100, 70),
            (100 + 70 + 400, 20),
            (100 + 70 + 400 + 60, 70),
            (100 + 70 + 400 + 60 + 100, 70),
        ]);
        let (window, mean) = session_metric(&stays);
        // Two sessions came out of the five stays, both long enough to
        // have been recorded in the window.
        assert_eq!(window.count(), 2);
        // The mean over the ones of a minute and longer.
        assert_eq!(mean, Some(150));
    }
}
