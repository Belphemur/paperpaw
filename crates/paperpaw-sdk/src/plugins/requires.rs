//! What a plugin on the card needs of other plugins: `requires` in the
//! `[plugin]` section of its `plugin.ini` (data formats 4), e.g.
//! `requires = stats, vocabulary 1.2.0`. Read by the store, which fetches
//! what is missing with it, and by the plugin manager, which says what is
//! missing; the kernel starts a plugin whatever it requires (a setup
//! template's defaults wait for the plugin they are for).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::id::Id;
use crate::version::Version;

/// Plugins one plugin names at most.
pub const MAX_REQUIRED: usize = 16;

/// A plugin another needs: its id, and the version it needs at least.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Requirement {
    pub id: Id,
    pub at_least: Option<Version>,
}

impl Requirement {
    pub fn new(id: Id, at_least: Option<Version>) -> Self {
        Self { id, at_least }
    }

    /// Whether a plugin of `version` meets it.
    pub fn met_by(&self, version: Version) -> bool {
        self.at_least.is_none_or(|least| version >= least)
    }
}

/// The value of `requires`: plugin ids apart by commas, each with the
/// version it needs at least after a space. An id twice, a plugin's own
/// id (`own`), more than [`MAX_REQUIRED`] or anything else not as said is
/// refused, with why.
pub fn parse(value: &str, own: &str) -> Result<Vec<Requirement>, String> {
    let mut list: Vec<Requirement> = Vec::new();
    for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let mut words = part.split_whitespace();
        let id = words.next().unwrap_or_default();
        let id = Id::parse(id)
            .ok()
            .filter(|parsed| parsed.as_str() == parsed.namespace())
            .ok_or_else(|| format!("invalid plugin id `{id}`"))?;
        let at_least = match words.next() {
            Some(text) => Some(
                Version::try_parse(text)
                    .map_err(|why| format!("invalid version `{text}`: {why}"))?,
            ),
            None => None,
        };
        if words.next().is_some() {
            return Err(format!("`{part}`: an id and a version at most"));
        }
        if id.as_str() == own {
            return Err(format!("`{own}` requires itself"));
        }
        if list.iter().any(|r| r.id == id) {
            return Err(format!("`{id}` named twice"));
        }
        if list.len() == MAX_REQUIRED {
            return Err(format!("more than {MAX_REQUIRED} plugins required"));
        }
        list.push(Requirement::new(id, at_least));
    }
    Ok(list)
}

/// What plugin `plugin` on the storage requires (its
/// `plugins/<id>/plugin.ini`); none where it names none, has no such file,
/// or names them wrongly (the kernel then does not load it).
pub fn of(storage: &dyn crate::Storage, plugin: &str) -> Vec<Requirement> {
    let path = format!("plugins/{plugin}/plugin.ini");
    let Ok(text) = crate::read_file(storage, &path, 16 * 1024) else {
        return Vec::new();
    };
    let Ok(text) = core::str::from_utf8(&text) else {
        return Vec::new();
    };
    paperpaw_config::parse(text)
        .filter_map(Result::ok)
        .find(|entry| entry.section == "plugin" && entry.key == "requires")
        .and_then(|entry| parse(&entry.value.as_str(), plugin).ok())
        .unwrap_or_default()
}

/// Of `required`, those not met by the plugins there are (`installed`:
/// each id and version).
pub fn missing<'a>(
    required: &'a [Requirement],
    installed: &[(&str, Version)],
) -> impl Iterator<Item = &'a Requirement> + 'a {
    let met: Vec<bool> = required
        .iter()
        .map(|r| {
            installed
                .iter()
                .any(|(id, version)| *id == r.id.as_str() && r.met_by(*version))
        })
        .collect();
    required
        .iter()
        .zip(met)
        .filter_map(|(r, met)| (!met).then_some(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_are_ids_with_a_least_version() {
        let list = parse(" stats, vocabulary 1.2.0 ,", "template").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Requirement::new(Id::new_static("stats"), None));
        assert_eq!(list[1].at_least, Some(Version::new(1, 2, 0)));
        assert!(parse("", "t").unwrap().is_empty());
        for wrong in [
            "Stats",
            "stats.widget",
            "stats 1.x",
            "stats 1.0.0 more",
            "stats, stats",
            "t",
        ] {
            assert!(parse(wrong, "t").is_err(), "{wrong}");
        }
        let many: Vec<String> = (0..=MAX_REQUIRED).map(|n| format!("p{n}")).collect();
        assert!(parse(&many.join(","), "t").is_err());
        assert!(parse(&many[..MAX_REQUIRED].join(","), "t").is_ok());
    }

    #[test]
    fn missing_are_those_not_there_or_too_old() {
        let list = parse("stats 2.0.0, vocabulary, news", "t").unwrap();
        let installed = [
            ("stats", Version::new(1, 9, 0)),
            ("vocabulary", Version::new(0, 1, 0)),
        ];
        let missing: Vec<&str> = missing(&list, &installed).map(|r| r.id.as_str()).collect();
        assert_eq!(missing, ["stats", "news"]);
    }

    #[test]
    fn read_from_the_plugins_file() {
        let storage = crate::MemoryStorage::new()
            .with(
                "plugins/cosy/plugin.ini",
                "[plugin]\nname = Cosy\nformats = 4\nrequires = stats 1.0.0\n",
            )
            .with("plugins/bad/plugin.ini", "[plugin]\nrequires = Bad!\n");
        assert_eq!(of(&storage, "cosy").len(), 1);
        assert!(of(&storage, "bad").is_empty());
        assert!(of(&storage, "none").is_empty());
    }
}
