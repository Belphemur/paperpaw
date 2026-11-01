use super::*;
use crate::report::Level;

fn folder(name: &str, files: &[(&str, &[u8])]) -> DataPlugin {
    let files = files
        .iter()
        .map(|(path, bytes)| (path.to_string(), bytes.to_vec()));
    let mut plugin = DataPlugin::new(name, files);
    plugin.folder = Some(std::path::PathBuf::from("plugins").join(name));
    plugin
}

fn findings(plugin: &DataPlugin) -> Vec<(Level, &'static str, String, String)> {
    let mut report = Report::default();
    check(plugin, &mut report);
    report
        .findings
        .into_iter()
        .map(|f| (f.level, f.check, f.place, f.message))
        .collect()
}

/// `store/store.ini` names one of the store's groups, the new ones too.
#[test]
fn the_store_s_group_is_one_it_has() {
    let manifest: &[u8] =
        b"[plugin]\nid = snake\nname = Snake\ndescription = A game.\nversion = 1.0.0\n";
    let group = |name: &str| {
        let store = format!("[store]\ngroup = {name}\n");
        findings(&folder(
            "snake",
            &[
                ("plugin.ini", manifest),
                ("store/store.ini", store.as_bytes()),
            ],
        ))
        .into_iter()
        .filter(|f| f.1 == "package.store-group")
        .map(|f| (f.2, f.3))
        .collect::<Vec<_>>()
    };
    for fits in [
        "games", "learning", "tools", "news", "notes", "reading", "system",
    ] {
        assert_eq!(group(fits), [], "{fits}");
    }
    let found = group("fun");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "store/store.ini:2");
    assert!(
        found[0].1.starts_with("`fun` is no group of the store"),
        "{}",
        found[0].1
    );
}

/// Every file the kernel reads for a data plugin (`data_plugin.rs`)
/// passes as read, none said to be unread.
#[test]
fn every_file_the_kernel_reads_passes() {
    let ini: &[u8] = b"[a]\nb = c\n";
    let files: [(&str, &[u8]); 12] = [
        (
            "plugin.ini",
            b"[plugin]\nid = every\nname = Every\ndescription = All of them.\nversion = 1.0.0\nformats = 3\n",
        ),
        ("settings.ini", b"[core]\nui-size = 14\n"),
        ("keymap.ini", b"[keymap.global]\nbutton.menu = core.refresh-screen\n"),
        ("styles.css", b"p { line-height: 1.6 }\n"),
        ("presets.ini", b"[night]\nkind = core.power-mode\nname = Night\n"),
        ("marks.ini", ini),
        ("menus.ini", b"[menu core.reader-menu]\nadd = every.night\n"),
        ("actions.ini", b"[night]\ntitle = Night\nrun = core.night-mode=on\n"),
        ("icons.ini", b"every.night = moon\n"),
        ("setup.ini", b"[size]\nsetting = core.font-size\n"),
        ("points/core.about.txt", b"A line of About.\n"),
        (
            "i18n/de.ini",
            b"[meta]\nlanguage = de\nname = Deutsch\nplural = one-other\n\n[messages]\nevery.plugin.name = Alle\n",
        ),
    ];
    let found = findings(&folder("every", &files));
    assert!(
        !found
            .iter()
            .any(|f| f.3.contains("does not read") || f.1 == "package.formats"),
        "{found:#?}"
    );
    // Each is one the kernel reads.
    for (path, _) in &files {
        assert!(
            read_as(&Folder::default(), path, "every").is_some(),
            "{path}"
        );
    }
}

/// A file of later data formats is read only where plugin.ini says
/// those formats: said, for each.
#[test]
fn files_of_later_formats_need_their_formats() {
    let files: [(&str, &[u8]); 4] = [
        (
            "plugin.ini",
            b"[plugin]\nid = old\nname = Old\ndescription = Formats 2.\nversion = 1.0.0\nformats = 2\n",
        ),
        ("marks.ini", b"[names]\ntitle = Names\n"),
        ("actions.ini", b"[night]\ntitle = Night\n"),
        ("points/core.about.txt", b"A line.\n"),
    ];
    let found = findings(&folder("old", &files));
    let formats: Vec<&str> = found
        .iter()
        .filter(|f| f.1 == "package.formats")
        .map(|f| f.2.as_str())
        .collect();
    assert_eq!(
        formats,
        ["actions.ini", "points/core.about.txt"],
        "{found:#?}"
    );
    // A file of them that does not parse says where.
    let broken: [(&str, &[u8]); 2] = [files[0], ("menus.ini", b"[core.menu\n")];
    let found = findings(&folder("old", &broken));
    assert!(
        found
            .iter()
            .any(|f| f.1 == "package.syntax" && f.2.starts_with("menus.ini:")),
        "{found:#?}"
    );
}

/// An action of more than 16 steps, one running an action of its own
/// file and one emitting another plugin's event are refused by the
/// kernel, and said here; a sound file is not.
#[test]
fn actions_the_kernel_refuses_are_said() {
    let manifest: &[u8] =
        b"[plugin]\nid = acts\nname = Acts\ndescription = Actions.\nversion = 1.0.0\nformats = 3\n";
    let many = vec!["run = core.home"; MAX_STEPS + 1].join("\n");
    let source = format!(
        "[long]\ntitle = Long\n{many}\n\n\
         [first]\ntitle = First\nrun = core.night-mode=on\nrun = acts.second\n\n\
         [second]\ntitle = Second\nrun = notify:acts.hello\n\n\
         [loud]\ntitle = Loud\nrun = emit:core.started\n\n\
         [fine]\ntitle = Fine\nrun = core.night-mode=on\nrun = open:core.library\nrun = emit:acts.done\nrun = core.home\n"
    );
    let found = findings(&folder(
        "acts",
        &[
            ("plugin.ini", manifest),
            ("actions.ini", source.as_bytes()),
            (
                "i18n/en.ini",
                b"[meta]\nlanguage = en\nname = English\nplural = one-other\n\n\
                  [messages]\nacts.hello = Hello, reader\n",
            ),
        ],
    ));
    let actions: Vec<(&str, &str)> = found
        .iter()
        .filter(|f| f.1 == "package.actions")
        .map(|f| (f.2.as_str(), f.3.as_str()))
        .collect();
    let at = |place: &str, part: &str| actions.iter().any(|(p, m)| *p == place && m.contains(part));
    assert!(
        at("actions.ini:2", "runs 17 steps, at most 16"),
        "{actions:#?}"
    );
    assert!(
        at(
            "actions.ini:22",
            "runs `acts.second`, an action of this same file"
        ),
        "{actions:#?}"
    );
    assert!(
        !actions
            .iter()
            .any(|(p, _)| ["actions.ini:27", "actions.ini:28"].contains(p)),
        "{actions:#?}"
    );
    assert!(at("actions.ini:31", "emits `core.started`"), "{actions:#?}");
    assert!(
        !actions.iter().any(|(p, _)| *p == "actions.ini:35"),
        "{actions:#?}"
    );
    assert!(actions.iter().all(|(_, m)| m.contains("shows as Error")));
}

/// Steps written on one line separated by commas, as `run` was first
/// written, and a notification of no translation key are said; a
/// step a line with commas of its own is not.
#[test]
fn steps_on_one_line_and_notes_of_no_key_are_said() {
    let manifest: &[u8] =
        b"[plugin]\nid = acts\nname = Acts\ndescription = Actions.\nversion = 1.0.0\nformats = 3\n";
    let source: &[u8] = b"[night]\ntitle = Night\n\
        run = core.night-mode=on, core.font-size=14, notify:acts.night\n\
        run = notify:Good night\n\
        run = acts.label=Hello, reader\n\
        run = notify:acts.night\n";
    let found = findings(&folder(
        "acts",
        &[
            ("plugin.ini", manifest),
            ("actions.ini", source),
            (
                "i18n/de.ini",
                b"[meta]\nlanguage = de\nname = Deutsch\nplural = one-other\n\n\
                  [messages]\nacts.night = Gute Nacht, schlaf gut\n",
            ),
        ],
    ));
    let actions: Vec<(&str, &str)> = found
        .iter()
        .filter(|f| f.1 == "package.actions")
        .map(|f| (f.2.as_str(), f.3.as_str()))
        .collect();
    let at = |place: &str, part: &str| actions.iter().any(|(p, m)| *p == place && m.contains(part));
    assert!(
        at("actions.ini:3", "a `run` line of its own"),
        "{actions:#?}"
    );
    assert!(
        at("actions.ini:4", "`notify:Good night` names no text"),
        "{actions:#?}"
    );
    assert_eq!(actions.len(), 2, "{actions:#?}");
    assert!(
        found
            .iter()
            .filter(|f| f.1 == "package.actions")
            .all(|f| f.0 == Level::Warning)
    );
}

/// Another plugin's menu changed in a plain section is said, a menu
/// of one's own made in `[menu <id>]` refused; each as it is written
/// is not.
#[test]
fn menus_are_told_apart_by_their_section() {
    let manifest: &[u8] =
        b"[plugin]\nid = tools\nname = Tools\ndescription = Menus.\nversion = 1.0.0\nformats = 3\n";
    let source: &[u8] = b"[tools]\ntitle = Tools\nentries = core.home\n\
        [menu core.reader-menu]\nadd = core.home after core.toc\nhide = core.toc\n\
        [core.reader-more]\nhide = core.home\n\
        [menu core.library-menu]\ntitle = Mine\n";
    let found = findings(&folder(
        "tools",
        &[("plugin.ini", manifest), ("menus.ini", source)],
    ));
    let menus: Vec<_> = found.iter().filter(|f| f.1 == "package.menus").collect();
    assert_eq!(menus.len(), 2, "{menus:#?}");
    assert!(
        menus.iter().any(|f| f.0 == Level::Warning
            && f.2 == "menus.ini:8"
            && f.3.contains("[menu core.reader-more]")),
        "{menus:#?}"
    );
    assert!(
        menus.iter().any(|f| f.0 == Level::Error
            && f.2 == "menus.ini:10"
            && f.3
                .contains("[menu core.library-menu] is another plugin's menu")),
        "{menus:#?}"
    );
}

#[test]
fn a_folder_of_pictures_named_by_settings_is_read() {
    let plugin = folder(
        "nights",
        &[
            (
                "plugin.ini",
                b"[plugin]\nid = nights\nname = Nights\nversion = 1.0.0\n",
            ),
            ("settings.ini", b"[sleep]\nfolder = plugins/nights/sleep\n"),
            ("sleep/moon.png", b"\x89PNG\r\n\x1a\nrest"),
            ("sleep/stars.png", b"\x89PNG\r\n\x1a\nrest"),
        ],
    );
    let found = findings(&plugin);
    assert!(
        !found
            .iter()
            .any(|f| f.1 == "package.pictures" || f.3.contains("does not read")),
        "{found:#?}"
    );
}

#[test]
fn a_sound_folder_passes() {
    let plugin = folder(
        "sound",
        &[
            (
                "plugin.ini",
                b"[plugin]\nid = sound\nname = Sound\ndescription = A test.\nversion = 1.0.0\nformats = 1\n",
            ),
            ("settings.ini", b"[core]\nui-size = 14\n"),
            (
                "i18n/de.ini",
                b"[meta]\nlanguage = de\nname = Deutsch\nplural = one-other\n\n[messages]\nsound.plugin.name = Klang\n",
            ),
            ("README.md", b"# Sound\n"),
        ],
    );
    let found = findings(&plugin);
    assert!(found.iter().all(|f| f.0 == Level::Info), "{found:#?}");
}

#[test]
fn a_pre_release_a_version_does_not_hold_is_refused_saying_so() {
    let with = |version: &str| {
        let ini = format!(
            "[plugin]
id = pre
name = Pre
description = A test.
version = {version}
"
        );
        let plugin = folder("pre", &[("plugin.ini", ini.as_bytes())]);
        findings(&plugin)
    };
    let refused = with("1.0.0-gamma");
    let found = refused
        .iter()
        .find(|f| f.1 == "package.version" && f.0 == Level::Error)
        .unwrap_or_else(|| panic!("{refused:#?}"));
    assert!(
        found.3.contains("alpha, beta, dev, pre or rc"),
        "{}",
        found.3
    );
    // A pre-release it holds is no refusal of PaperPaw's: the plugin
    // library takes versions like 1.2.0 only (a separate finding).
    assert!(
        !with("1.0.0-beta.2")
            .iter()
            .any(|f| f.3.contains("is no version"))
    );
}

#[test]
fn what_the_installer_and_the_website_refuse() {
    let plugin = folder(
        "bad",
        &[
            (
                "plugin.ini",
                b"[plugin]\nname = Bad\nversion = 1.0\nformats = 9\nsdk = 0.4\n",
            ),
            (".DS_Store", b""),
            ("run.exe", b"MZ"),
            ("fonts/a.ppfp", b"PPFP"),
            ("loose.ppfp", b"PPFP"),
            ("fonts/b.ttf", b"nope"),
            ("style.css", b"p {}"),
            ("i18n/de.ini", b"[messages]\nx = y\n"),
            ("settings.ini", b"[core]\nlogo = plugins/bad/logo.png\n"),
        ],
    );
    let found = findings(&plugin);
    let has = |check: &str, place: &str, level: Level| {
        found
            .iter()
            .any(|f| f.1 == check && f.2 == place && f.0 == level)
    };
    assert!(has("package.id", "plugin.ini", Level::Error), "{found:#?}");
    assert!(has("package.version", "plugin.ini", Level::Error));
    assert!(has("package.formats", "plugin.ini", Level::Error));
    assert!(has("package.manifest", "plugin.ini:5", Level::Warning));
    assert!(has("package.files", ".DS_Store", Level::Error));
    assert!(has("package.website", "run.exe", Level::Error));
    // A font pack is taken in fonts/, not elsewhere.
    assert!(!has("package.website", "fonts/a.ppfp", Level::Error));
    assert!(has("package.website", "loose.ppfp", Level::Error));
    assert!(has("package.files", "fonts/a.ppfp", Level::Warning));
    assert!(has("package.fonts", "fonts/b.ttf", Level::Error));
    assert!(has("package.files", "style.css", Level::Warning));
    assert!(has("package.i18n", "i18n/de.ini", Level::Error));
    assert!(has("package.pictures", "settings.ini:2", Level::Error));
    assert!(
        has("package.files", "", Level::Error),
        "the installer refuses it"
    );
}

#[test]
fn tests_beside_the_plugin_are_named() {
    let plugin = folder(
        "tested",
        &[
            (
                "plugin.ini",
                b"[plugin]\nid = tested\nname = Tested\nversion = 1.0.0\n",
            ),
            ("night.flow", b"confirm\n"),
            ("flows/day.flow", b"confirm\n"),
            ("snapshots/night-1.png", b"\x89PNG\r\n\x1a\nrest"),
            ("tests/notes.txt", b"notes"),
            ("README.md", b"# Tested\n"),
        ],
    );
    let found = findings(&plugin);
    let mut tests: Vec<&str> = found
        .iter()
        .filter(|f| f.1 == "package.tests" && f.0 == Level::Warning)
        .map(|f| f.2.as_str())
        .collect();
    tests.sort_unstable();
    assert_eq!(
        tests,
        [
            "flows/day.flow",
            "night.flow",
            "snapshots/night-1.png",
            "tests/notes.txt"
        ],
        "{found:#?}"
    );
    assert!(
        !found.iter().any(|f| f.3.contains("does not read")),
        "{found:#?}"
    );
}

#[test]
fn families_and_packs_are_named() {
    let plugin = folder(
        "font-round",
        &[
            (
                "plugin.ini",
                b"[plugin]\nid = font-round\nname = Round\ndescription = A font.\nversion = 1.0.0\n[family]\nname = Round\nbold = bold.ppfp\noblique = x.ppfp\n[extends]\ncyrillic.ppfp = Not An Id\n",
            ),
            ("fonts/bold.ppfp", b"PPFP...."),
        ],
    );
    let found = findings(&plugin);
    let at = |place: &str| found.iter().filter(|f| f.2 == place).count();
    // No regular face; no such file to extend with, nor such a font.
    assert!(at("plugin.ini") >= 1, "{found:#?}");
    assert!(at("plugin.ini:9") == 1, "{found:#?}");
    assert!(at("plugin.ini:11") == 2, "{found:#?}");
    assert!(
        !found
            .iter()
            .any(|f| f.2 == "fonts/bold.ppfp" && f.1 == "package.files"),
        "a face is read: {found:#?}"
    );
}

/// A PNG header of `width` x `height`, interlaced or not, padded to `size`.
fn png(width: u32, height: u32, interlaced: bool, size: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 0, 0, 0, u8::from(interlaced)]);
    bytes.resize(size, 0);
    bytes
}

const MANIFEST: &[u8] = b"[plugin]\nid = owls\nname = Owls\ndescription = Owls.\nversion = 1.0.0\n";

/// What the conventions say of `files` (`package.*` warnings of theirs).
fn conventions(files: &[(&str, &[u8])]) -> Vec<(&'static str, String)> {
    let mut all = vec![("plugin.ini", MANIFEST)];
    all.extend_from_slice(files);
    findings(&folder("owls", &all))
        .into_iter()
        .filter(|f| {
            f.0 == Level::Warning
                && [
                    "package.folder",
                    "package.picture",
                    "package.picture-place",
                    "package.how",
                    "package.languages",
                    "package.code-data",
                ]
                .contains(&f.1)
        })
        .map(|f| (f.1, f.2))
        .collect()
}

#[test]
fn a_plugin_that_keeps_the_conventions_is_not_warned() {
    let picture = png(480, 800, false, 30_000);
    let logo = png(160, 160, false, 4_000);
    let found = conventions(&[
        ("pictures/owl.png", &picture),
        ("pictures/logo.png", &logo),
        (
            "i18n/en.ini",
            b"[meta]\nlanguage = en\nname = English\nplural = one-other\n",
        ),
        (
            "i18n/de.ini",
            b"[meta]\nlanguage = de\nname = Deutsch\nplural = one-other\n",
        ),
        ("how/en.txt", b"x"),
        ("how/de.txt", b"x"),
        ("data/words.txt", b"x"),
        ("store/shot.png", &png(1000, 1000, true, 600_000)),
    ]);
    assert_eq!(found, []);
}

#[test]
fn a_folder_that_is_none_of_the_conventions_is_said_once() {
    let found = conventions(&[("images/a.txt", b"x"), ("images/b.txt", b"x")]);
    assert_eq!(found, [("package.folder", "images/".to_string())]);
}

#[test]
fn a_picture_outside_pictures_is_said_to_belong_there() {
    let found = conventions(&[("logo.png", &png(100, 100, false, 2_000))]);
    assert_eq!(found, [("package.picture-place", "logo.png".to_string())]);
}

#[test]
fn pictures_are_checked_for_interlacing_size_and_weight() {
    let found = conventions(&[
        ("pictures/interlaced.png", &png(100, 100, true, 1_000)),
        ("pictures/big.png", &png(1200, 1600, false, 1_000)),
        ("pictures/heavy.png", &png(400, 600, false, 61_000)),
        ("pictures/logo.png", &png(100, 100, false, 11_000)),
        ("pictures/turned.png", &png(800, 480, false, 1_000)),
    ]);
    let places: Vec<&str> = found.iter().map(|f| f.1.as_str()).collect();
    assert_eq!(
        places,
        [
            "pictures/big.png",
            "pictures/heavy.png",
            "pictures/interlaced.png",
            "pictures/logo.png",
        ]
    );
    assert!(found.iter().all(|f| f.0 == "package.picture"));
}

#[test]
fn how_it_works_comes_in_english_and_german_also_per_plugin() {
    let found = conventions(&[("how/en.txt", b"x"), ("how/format-cbz/de.txt", b"x")]);
    let places: Vec<&str> = found.iter().map(|f| f.1.as_str()).collect();
    assert_eq!(places, ["how/de.txt", "how/format-cbz/en.txt"]);
}

#[test]
fn english_texts_without_german_ones_are_said() {
    let en: &[u8] = b"[meta]\nlanguage = en\nname = English\nplural = one-other\n";
    let found = conventions(&[("i18n/en.ini", en)]);
    assert_eq!(found, [("package.languages", "i18n/de.ini".to_string())]);
    // A language of its own is a plugin of its own.
    let fr: &[u8] = b"[meta]\nlanguage = fr\nname = Francais\nplural = one-other\n";
    assert_eq!(conventions(&[("i18n/fr.ini", fr)]), []);
}

#[test]
fn a_folder_with_code_and_kernel_data_files_is_said() {
    let found = conventions(&[("code.ppn", b"x"), ("keymap.ini", b"[keymap.reader]\n")]);
    assert!(found.contains(&("package.code-data", "keymap.ini".to_string())));
    assert!(!found.iter().any(|f| f.1 == "plugin.ini"));
}
