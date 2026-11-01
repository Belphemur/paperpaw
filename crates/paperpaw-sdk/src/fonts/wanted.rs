//! Which font files are opened for the roles plugins and readers bind.

use alloc::vec::Vec;

use super::{Fonts, Id};

impl Fonts {
    /// The entries of the faces that take a role no chosen family takes:
    /// the user's choice, else the best proposal (as `proposed` picks it
    /// once loaded), e.g. Literata's bold from the card, where the
    /// firmware has none. Losing proposals stay unloaded.
    #[cfg(any(test, feature = "kernel"))]
    pub(super) fn role_faces(&self) -> Vec<usize> {
        let position = |font: &Id| self.entries.iter().position(|e| e.id == *font);
        let mut wanted: Vec<usize> = self.user.values().filter_map(position).collect();
        let mut seen: Vec<&Id> = Vec::new();
        for (role, _, _) in &self.candidates {
            if seen.contains(&role)
                || self.user.contains_key(role.as_str())
                || self.family_face(role.as_str()).is_some()
            {
                continue;
            }
            seen.push(role);
            let best = self
                .candidates
                .iter()
                .filter(|(r, _, _)| r == role)
                .filter_map(|(_, font, priority)| Some((position(font)?, *priority)))
                .max_by_key(|(_, priority)| *priority);
            wanted.extend(best.map(|(entry, _)| entry));
        }
        wanted
    }
}
