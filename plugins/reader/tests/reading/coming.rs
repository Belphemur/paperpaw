//! A book still coming in (a download): opened when it is there.

use super::*;

/// How often the coming book was asked for.
type Opened = Rc<std::cell::Cell<u32>>;

const COMING: &str = "books/coming.part";
const ARRIVED: Id = Id::new_static("coming.arrived");

/// A format of books still coming in: never ready here; and an action
/// saying the book arrived.
struct Coming(Opened);

impl Plugin for Coming {
    fn manifest(&self) -> Manifest {
        Manifest::new(Id::new_static("coming"), "Coming", Version::new(0, 1, 0))
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        let asked = self.0.clone();
        r.add_format(paperpaw_sdk::FormatSpec::new(
            Id::new_static("coming.part"),
            "coming",
            vec![std::borrow::Cow::Borrowed("part")],
            Box::new(move |_, _, _| {
                asked.set(asked.get() + 1);
                Err(paperpaw_sdk::document::DocumentError::NotReady)
            }),
        ))?;
        r.add_action(
            paperpaw_sdk::ActionSpec::new(ARRIVED, "coming.arrived").handler(|_, _, out| {
                for path in ["books/other.part", COMING] {
                    let ready = events::DocumentReady::new(path.to_string());
                    out.push(events::DOCUMENT_READY.emit(ready));
                }
            }),
        )
    }
}

#[test]
fn a_book_still_coming_opens_when_it_is_there() {
    let storage = Shared::default();
    storage.create_dir("books").unwrap();
    storage.write(COMING, b"part of it").unwrap();
    let asked = Opened::default();
    let mut kernel = Kernel::builder(DEVICE)
        .storage(storage.clone())
        .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
        .plugin(Reader)
        .plugin(Coming(asked.clone()))
        .build()
        .unwrap();
    let effects = kernel.start(Instant(0));
    let mut reading = Reading {
        kernel,
        storage,
        heard: Heard::default(),
        now: 0,
        book: COMING,
    };
    reading.settle(effects);
    reading.open();
    assert_eq!(asked.get(), 1);
    // Told that another book and then this one is there: asked again once.
    reading.run(ARRIVED);
    assert_eq!(asked.get(), 2);
}
