// Keyboard chords, and what they run.
//
// The defaults are not written down twice: `COMMANDS` already carries each
// binding as the text shown in the palette and the context menu, and this
// parses those same strings. A binding and its label cannot drift apart
// because there is only one of them.
//
// Only *command* chords live here. Arrow keys, Home/End, Page Up/Down, Tab,
// Enter in the filter box and type-ahead stay in the key handler: they are
// movement, their meaning changes with Shift and Ctrl, and rebinding them
// would mean encoding selection semantics in a settings file.

use std::collections::HashMap;

/// A key with its modifiers. `vk` is a Win32 virtual-key code; keeping it a
/// plain `u16` is what lets this module be tested without a window.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Chord {
    pub vk: u16,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    pub fn new(vk: u16, ctrl: bool, shift: bool, alt: bool) -> Chord {
        Chord {
            vk,
            ctrl,
            shift,
            alt,
        }
    }

    /// Parse `"Ctrl+Shift+P"`. None for anything that is not a usable chord,
    /// which is what a settings file full of typos gets: ignored, not fatal.
    pub fn parse(text: &str) -> Option<Chord> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let mut chord = Chord::new(0, false, false, false);
        let parts: Vec<&str> = text.split('+').map(|p| p.trim()).collect();
        let (key, mods) = parts.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "shift" => chord.shift = true,
                "alt" => chord.alt = true,
                _ => return None,
            }
        }
        chord.vk = vk_from_name(key)?;
        Some(chord)
    }

    /// The text form, which is what the palette shows and the settings file
    /// stores. Round-trips through `parse`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        out.push_str(&vk_name(self.vk));
        out
    }
}

/// Named keys, in both directions. Letters and digits are their own names and
/// are handled arithmetically rather than listed.
const NAMED: &[(&str, u16)] = &[
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Esc", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Ins", 0x2D),
    ("Del", 0x2E),
];

/// Names people also write, accepted on the way in but never produced.
const ALIASES: &[(&str, u16)] = &[
    ("Return", 0x0D),
    ("Escape", 0x1B),
    ("Delete", 0x2E),
    ("Insert", 0x2D),
    ("PgUp", 0x21),
    ("PgDn", 0x22),
];

fn vk_from_name(name: &str) -> Option<u16> {
    if let Some((_, vk)) = NAMED
        .iter()
        .chain(ALIASES)
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
    {
        return Some(*vk);
    }
    // F1..F24
    let lower = name.to_ascii_lowercase();
    if let Some(digits) = lower.strip_prefix('f') {
        if let Ok(n) = digits.parse::<u16>() {
            if (1..=24).contains(&n) {
                return Some(0x70 + n - 1);
            }
        }
    }
    let mut chars = name.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c.is_ascii_alphanumeric() {
        // Virtual-key codes for letters and digits are their uppercase ASCII.
        Some(c.to_ascii_uppercase() as u16)
    } else {
        None
    }
}

fn vk_name(vk: u16) -> String {
    if let Some((n, _)) = NAMED.iter().find(|(_, v)| *v == vk) {
        return (*n).to_string();
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x70 + 1);
    }
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return ((vk as u8) as char).to_string();
    }
    format!("0x{:02X}", vk)
}

/// Which command each chord runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings {
    map: HashMap<Chord, usize>,
}

impl Bindings {
    /// The defaults, read from the command table's own shortcut text.
    pub fn from_defaults(defaults: &[(usize, &str)]) -> Bindings {
        let mut b = Bindings::default();
        for (id, keys) in defaults {
            if let Some(chord) = Chord::parse(keys) {
                b.map.insert(chord, *id);
            }
        }
        b
    }

    pub fn command_for(&self, chord: Chord) -> Option<usize> {
        self.map.get(&chord).copied()
    }

    /// The chord that runs `id`, if any. Linear over a few dozen entries, and
    /// called when a menu is built rather than per keystroke.
    pub fn chord_for(&self, id: usize) -> Option<Chord> {
        self.map.iter().find(|(_, v)| **v == id).map(|(k, _)| *k)
    }

    /// What the palette and the context menu show. Empty for unbound.
    pub fn text_for(&self, id: usize) -> String {
        self.chord_for(id).map(|c| c.text()).unwrap_or_default()
    }

    /// Bind `chord` to `id`, taking it from whatever held it and dropping any
    /// chord `id` already had — one chord per command and one command per
    /// chord, so neither a duplicate nor a stranded binding can be produced.
    pub fn set(&mut self, chord: Chord, id: usize) {
        self.map.retain(|_, v| *v != id);
        self.map.insert(chord, id);
    }

    pub fn clear(&mut self, id: usize) {
        self.map.retain(|_, v| *v != id);
    }

    /// Bindings that differ from `defaults`, as (chord text, id). Only these
    /// are written to the settings file, so a default that changes in a later
    /// version still reaches someone who never rebound it. An empty chord text
    /// records a shortcut that was removed.
    pub fn changed_from(&self, defaults: &Bindings) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = self
            .map
            .iter()
            .filter(|(c, id)| defaults.map.get(*c) != Some(*id))
            .map(|(c, id)| (c.text(), *id))
            .collect();
        for id in defaults.map.values() {
            if self.chord_for(*id).is_none() {
                out.push((String::new(), *id));
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip_through_text() {
        for text in [
            "Ctrl+Shift+P",
            "Alt+P",
            "F5",
            "Del",
            "Backspace",
            "Ctrl+1",
            "Ctrl+Shift+Left",
            "Enter",
        ] {
            let c = Chord::parse(text).unwrap_or_else(|| panic!("{} should parse", text));
            assert_eq!(c.text(), text, "{} did not round-trip", text);
        }
    }

    #[test]
    fn aliases_are_accepted_but_normalised() {
        assert_eq!(Chord::parse("Delete"), Chord::parse("Del"));
        assert_eq!(Chord::parse("Return"), Chord::parse("Enter"));
        assert_eq!(Chord::parse("Del").unwrap().text(), "Del");
        // Case and spacing are the user's business, not ours.
        assert_eq!(
            Chord::parse(" ctrl + shift + p "),
            Chord::parse("Ctrl+Shift+P")
        );
    }

    #[test]
    fn nonsense_is_ignored_rather_than_fatal() {
        assert!(Chord::parse("").is_none());
        assert!(Chord::parse("Hyper+P").is_none());
        assert!(Chord::parse("Ctrl+").is_none());
        assert!(Chord::parse("Ctrl+NotAKey").is_none());
        assert!(Chord::parse("F99").is_none());
    }

    #[test]
    fn rebinding_leaves_one_chord_per_command() {
        let defaults = Bindings::from_defaults(&[(1, "Ctrl+A"), (2, "Ctrl+B")]);
        let mut b = defaults.clone();

        // Moving command 1 to a new chord releases its old one.
        b.set(Chord::parse("Ctrl+Q").unwrap(), 1);
        assert_eq!(b.command_for(Chord::parse("Ctrl+Q").unwrap()), Some(1));
        assert_eq!(b.command_for(Chord::parse("Ctrl+A").unwrap()), None);
        assert_eq!(b.text_for(1), "Ctrl+Q");

        // Taking a chord that belonged to another command unbinds that one.
        b.set(Chord::parse("Ctrl+B").unwrap(), 1);
        assert_eq!(b.command_for(Chord::parse("Ctrl+B").unwrap()), Some(1));
        assert_eq!(b.text_for(2), "", "command 2 lost its chord to command 1");
    }

    #[test]
    fn only_differences_from_the_defaults_are_saved() {
        let defaults = Bindings::from_defaults(&[(1, "Ctrl+A"), (2, "Ctrl+B")]);
        let mut b = defaults.clone();
        assert!(
            b.changed_from(&defaults).is_empty(),
            "untouched saves nothing"
        );

        b.set(Chord::parse("Ctrl+Q").unwrap(), 1);
        assert_eq!(b.changed_from(&defaults), vec![("Ctrl+Q".to_string(), 1)]);

        // An unbound command is a difference too, or removing a shortcut would
        // not survive a restart.
        let mut cleared = defaults.clone();
        cleared.clear(2);
        assert_eq!(cleared.changed_from(&defaults), vec![(String::new(), 2)]);
    }
}
