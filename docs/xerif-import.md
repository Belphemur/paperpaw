# Bringing CrossPoint's reading data over

A card that a CrossPoint reader (the X4, X3 and their kind) has written
keeps its reading data in `/.crosspoint` at the root of the card. The
statistics plugin takes it over: *Statistics → Sync with KOReader →
Take over from CrossPoint*. Nothing of CrossPoint's is changed or
removed, what was taken over is marked as done so that running it again
takes nothing twice, and a file of a newer CrossPoint than this knows is
left alone whole (it is never read as data of an older layout).

The decoder of CrossPoint's files and the conversion are in the plugin's
`xcpt`; the import itself is `xerif`. Each file is decoded with the
constants of its own version, and what cannot be read is never written
over.

## The files on the card

| File | Versions read | What it is |
|------|---------------|------------|
| `/.crosspoint/global_stats.bin` (+ `.bak`) | v3, v4, v5, v6 | All reading: totals, a bit a day for 730 days, the words a minute and the length of a session of the last pages, and (v6) the minutes read on each of the last 91 days. |
| `/.crosspoint/finished_books.bin` (+ `.bak`) | CPFB v1–v3 | Up to 32 finished books, named by the path hashed with FNV-1a, with title, author, dates and seconds read. |
| `/.crosspoint/{epub,xtc}_<hash>/stats_v8.bin` | v5, v6, v7, v8 | One book's reading: totals, dates, the words a minute, the length of a session, how far in the book. A `stats_v9.bin` in the folder, or a record whose version byte is higher than 8, marks the folder as of a newer CrossPoint: left alone. The `.tmp` of the newest is read, as a record a failed publish left whole. |
| `…/progress.bin` | 6, 10, 16 bytes (EPUB), 4 bytes (XTC) | Where the book was left. |
| `…/book.bin` | version 10 | The book's title, author, language and how many files it has. |
| `/.crosspoint/recent.json` | — | The books CrossPoint remembered last reading, with titles and authors. |
| `/.crosspoint/library.idx` | CLX1 version 2 | The index of the books on the card: paths, titles, authors. |

## Which folder is which book

CrossPoint's per-book folders are named `epub_<n>` or `xtc_<n>`, where
`n` is the book's path (`/books/anna.epub`, with the slash, as the card
names it) run through C++'s `std::hash<std::string>` — on the machines
CrossPoint runs on, libstdc++'s 32-bit Murmur hash with its own seed
(`0xc70f6907`), written as a decimal number. The records themselves say
nothing of which book they belong to, so the import hashes the paths of
the books on the card again and matches the folder by the hash. The
paths come from, in this order: `recent.json`, `library.idx`, and a walk
of the card itself (the card and five levels of folders, hidden ones
left out, as CrossPoint's own walk did).

A folder whose hash matches no book on the card brings nothing over: it
cannot be told which book the reading belongs to, and neither its
records nor the book's own carry a fingerprint that could bind them. The
import says how many folders it left out for that, and they can be
matched by hand once the book is back on the card (the mark of what was
brought over does not stop a folder that was left out from being brought
over later).

## What maps to what

| CrossPoint | PaperPaw | How |
|------------|----------|-----|
| Global minutes per day (v6), or a minute a read day backfilled from the history bits (v3–v5) | `days.bin`, the seconds read per local day | The day the history begins and the days before it, turned into local days; a day already read here keeps its own seconds and adds what CrossPoint knew. |
| Book folder, matched to a book on the card | `books.ini` | The book is added if new (by its partial MD5, else by its path), with title, author and language from `book.bin`, or from `recent.json`/`library.idx`, or from the file's name. |
| The book's finished date (record or index) | `books.ini`'s `finished` | The date as the middle of that local day, where none was marked before. |
| Where the EPUB was left (spine, page in it) | `.paperpaw/positions/` | The section (the file of the book) at its start, with the percent: the record's own, else worked out from the file of the book and the page in it, of all its files. |
| Where the XTC book was left (a page) | `.paperpaw/positions/` | The page itself, exact, with the record's percent. |
| The words a minute (window) and the length of a session (window), of a book and of all books | the pace the plugin keeps while it runs | Seeded at the import; see below. |
| Total sessions and seconds of a book and of all books | the pace's fallback | The average before there is a window to show, as CrossPoint worked it. |

### What is left out, and why

- **The log of page stays is never written to.** Those rows are
  KOReader's, shared with it, and CrossPoint kept no stays — writing
  rows would be making up reading that did not happen here. The days
  (`days.bin`) are derived from the log, so what CrossPoint knew per day
  is added there instead, marked as not coming from the log (no stays).
- **The books' reading time, pages turned and sessions have no home in
  `books.ini`** (that file is KOReader's, and the plugin keeps it so the
  two can exchange it). They live on in the pace the plugin keeps until
  the device is switched off.
- **The time-of-day and day-of-week sums** are not brought over: the
  plugin works them out from the log, which has no such rows to add.
- **The longest streak** is not brought over: the plugin works it out
  from the days, and the 91 days of minutes do not cover the 730 days of
  history.
- **A position's xpointer** is not made up: only a reader of the book
  itself can name the place. The position is kept by the section it was
  in and the percent; the reader takes up where that means.
- **A position whose percent cannot be said** (no percent in the record
  and no `book.bin` to work it out from) is not kept: a made-up percent
  would read as a real one.
- **`estimatedTimeLeftSeconds`** is not brought over: it belongs to
  CrossPoint's layout of the book.

## The pace after an import

CrossPoint kept the words a minute and the length of a session in its
records; PaperPaw's log cannot say either (it has no words per page and
no marker of where a session began). What the import brings is seeded
into the pace the plugin keeps while it runs: the words a minute of
every book it matched and of all books, and the length of their
sessions. As reading happens here, the plugin's own samples join in:
pages read now add to the words a minute, and the sessions of the log
stand in for how long a session lasts once four of them are known (see
`pace`). After a restart the windows of CrossPoint are gone — the
import can be run again (it brings nothing twice) to seed them anew.

## The reading brought over, and the mark of what was done

The minutes read per day are kept in a file of the plugin's own
(`imported-days.bin`), the day as the card counted it and the seconds
read on it. The days of the statistics add them wherever they are
summed anew from the log — after a merge with KOReader, a change of
time zone or a cut while writing — so what was taken over is not lost
with the log it is not part of. A later import replaces those minutes
day by day (it never adds to what it brought before), so running it
again says the same.

What was taken over is kept in `xerif.ini` in the plugin's folder: one
line per book folder (its name and the version of the record), and one
for the index of finished books and one for the global record (their
size and a hash of their content). A file that changed is brought over
again; a book folder whose record is of another version than what was
brought is brought again. A folder whose position could not be kept, a
book whose file could not be read, and an index that names a book that
is not on the card are left for the next run. Where the marks
themselves cannot be read, nothing is brought over and nothing is
written: they are not overwritten blind. Removing the file makes the
next run bring everything over again, which changes nothing that is
already there.

Where CrossPoint's own copy of the global record or of the index of
finished books cannot be read as one, the copy beside it (`.bak`) is
read in its place; a copy of a newer CrossPoint is never replaced by an
older one.
