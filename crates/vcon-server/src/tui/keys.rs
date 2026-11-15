//! The key bindings: one table, read by the key reader, the footer and the
//! help view, so what is shown is what is bound.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::reveal::Part;
use crate::tui::state::Action;

/// One action and the keys that ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// What the keys ask for.
    pub action: Action,
    /// Every key that asks for it.
    pub keys: &'static [(KeyCode, KeyModifiers)],
    /// The first key, as the footer and help write it.
    pub label: &'static str,
    /// The footer's word for the action.
    pub hint: &'static str,
    /// The help view's sentence for the action.
    pub does: &'static str,
}

const PLAIN: KeyModifiers = KeyModifiers::NONE;

/// Every binding. A key appears once.
pub const BINDINGS: [Binding; 18] = [
    Binding {
        action: Action::Down,
        keys: &[(KeyCode::Char('j'), PLAIN), (KeyCode::Down, PLAIN)],
        label: "j",
        hint: "down",
        does: "Select the vCon below (also Down)",
    },
    Binding {
        action: Action::Up,
        keys: &[(KeyCode::Char('k'), PLAIN), (KeyCode::Up, PLAIN)],
        label: "k",
        hint: "up",
        does: "Select the vCon above (also Up)",
    },
    Binding {
        action: Action::Top,
        keys: &[(KeyCode::Char('g'), PLAIN), (KeyCode::Home, PLAIN)],
        label: "g",
        hint: "first",
        does: "Select the first vCon (also Home)",
    },
    Binding {
        action: Action::Bottom,
        keys: &[(KeyCode::Char('G'), PLAIN), (KeyCode::End, PLAIN)],
        label: "G",
        hint: "last",
        does: "Select the last vCon read (also End)",
    },
    Binding {
        action: Action::PageUp,
        keys: &[(KeyCode::PageUp, PLAIN)],
        label: "PgUp",
        hint: "page up",
        does: "Move up one screen of the list or of the detail view's text",
    },
    Binding {
        action: Action::PageDown,
        keys: &[(KeyCode::PageDown, PLAIN)],
        label: "PgDn",
        hint: "page down",
        does: "Move down one screen of the list or of the detail view's text",
    },
    Binding {
        action: Action::Open,
        keys: &[(KeyCode::Enter, PLAIN)],
        label: "Enter",
        hint: "open",
        does: "Open the selected vCon: its metadata and masked view",
    },
    Binding {
        action: Action::Back,
        keys: &[(KeyCode::Esc, PLAIN), (KeyCode::Backspace, PLAIN)],
        label: "Esc",
        hint: "back",
        does: "Go back to the view before (also Backspace)",
    },
    Binding {
        action: Action::Reveal(Part::Parties),
        keys: &[(KeyCode::Char('p'), PLAIN)],
        label: "p",
        hint: "parties",
        does: "Reveal the parties of the vCon open: names, numbers, addresses",
    },
    Binding {
        action: Action::Reveal(Part::Subject),
        keys: &[(KeyCode::Char('s'), PLAIN)],
        label: "s",
        hint: "subject",
        does: "Reveal the subject of the vCon open",
    },
    Binding {
        action: Action::Reveal(Part::Dialog),
        keys: &[(KeyCode::Char('d'), PLAIN)],
        label: "d",
        hint: "dialog",
        does: "Reveal the dialog of the vCon open: what was said",
    },
    Binding {
        action: Action::Reveal(Part::Analysis),
        keys: &[(KeyCode::Char('a'), PLAIN)],
        label: "a",
        hint: "analysis",
        does: "Reveal the analysis of the vCon open, transcripts among it",
    },
    Binding {
        action: Action::Reveal(Part::Links),
        keys: &[(KeyCode::Char('l'), PLAIN)],
        label: "l",
        hint: "links",
        does: "Reveal where the vCon open links to",
    },
    Binding {
        action: Action::Reveal(Part::All),
        keys: &[(KeyCode::Char('w'), PLAIN)],
        label: "w",
        hint: "whole",
        does: "Reveal the whole vCon open",
    },
    Binding {
        action: Action::Refresh,
        keys: &[(KeyCode::Char('r'), PLAIN)],
        label: "r",
        hint: "refresh",
        does: "Read the list again from the server",
    },
    Binding {
        action: Action::More,
        keys: &[(KeyCode::Char('n'), PLAIN)],
        label: "n",
        hint: "more",
        does: "Read the next page of vCons",
    },
    Binding {
        action: Action::Help,
        keys: &[(KeyCode::Char('?'), PLAIN)],
        label: "?",
        hint: "help",
        does: "Show or close this help",
    },
    Binding {
        action: Action::Quit,
        keys: &[
            (KeyCode::Char('q'), PLAIN),
            (KeyCode::Char('c'), KeyModifiers::CONTROL),
        ],
        label: "q",
        hint: "quit",
        does: "Quit (also Ctrl-C)",
    },
];

/// The action a key press asks for, if the key is bound. Shift is part of
/// the character a terminal reports (`G`, `?`), so it is not compared for
/// one.
#[must_use]
pub fn action(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    let modifiers = if matches!(code, KeyCode::Char(_)) {
        modifiers.difference(KeyModifiers::SHIFT)
    } else {
        modifiers
    };
    BINDINGS
        .iter()
        .find(|binding| binding.keys.contains(&(code, modifiers)))
        .map(|binding| binding.action)
}

/// The binding of `action`.
#[must_use]
pub fn binding(action: Action) -> Option<&'static Binding> {
    BINDINGS.iter().find(|binding| binding.action == action)
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    use super::{BINDINGS, action};
    use crate::tui::state::Action;

    #[test]
    fn every_action_has_a_binding() {
        for wanted in [
            Action::Up,
            Action::Down,
            Action::Top,
            Action::Bottom,
            Action::PageUp,
            Action::PageDown,
            Action::Open,
            Action::Back,
            Action::Refresh,
            Action::More,
            Action::Help,
            Action::Quit,
        ] {
            assert!(
                BINDINGS.iter().any(|b| b.action == wanted),
                "{wanted:?} can be reached from the keyboard"
            );
        }
    }

    #[test]
    fn every_listed_key_gives_its_action() {
        for binding in &BINDINGS {
            assert!(!binding.keys.is_empty(), "{:?} lists a key", binding.action);
            for (code, modifiers) in binding.keys {
                assert_eq!(
                    action(*code, *modifiers),
                    Some(binding.action),
                    "{code:?} is listed for {:?}",
                    binding.action
                );
            }
        }
    }

    #[test]
    fn no_key_is_listed_twice() {
        let mut seen = Vec::new();
        for binding in &BINDINGS {
            for key in binding.keys {
                assert!(!seen.contains(key), "{key:?} has one meaning");
                seen.push(*key);
            }
        }
    }

    #[test]
    fn each_reveal_key_reveals_its_part() {
        use crate::reveal::Part;
        for (key, part) in [
            ('p', Part::Parties),
            ('s', Part::Subject),
            ('d', Part::Dialog),
            ('a', Part::Analysis),
            ('l', Part::Links),
            ('w', Part::All),
        ] {
            assert_eq!(
                action(KeyCode::Char(key), KeyModifiers::NONE),
                Some(Action::Reveal(part)),
                "{key}"
            );
        }
    }

    /// No key plays a recording: the terminal UI has no player.
    #[test]
    fn no_key_reveals_a_recording() {
        assert!(
            BINDINGS
                .iter()
                .all(|b| b.action != Action::Reveal(crate::reveal::Part::Audio)),
            "audio is the console's"
        );
    }

    #[test]
    fn an_upper_case_reveal_letter_is_no_key() {
        for key in ['P', 'S', 'D', 'A', 'L', 'W'] {
            assert_eq!(
                action(KeyCode::Char(key), KeyModifiers::SHIFT),
                None,
                "{key}"
            );
        }
    }

    #[test]
    fn control_c_quits() {
        assert_eq!(
            action(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Action::Quit),
            "the terminal's interrupt key ends the program in raw mode too"
        );
    }

    #[test]
    fn a_key_with_no_binding_is_no_action() {
        assert_eq!(
            action(KeyCode::Char('x'), KeyModifiers::NONE),
            None,
            "x is unbound"
        );
        assert_eq!(
            action(KeyCode::Char('q'), KeyModifiers::CONTROL),
            None,
            "a modifier makes another key"
        );
    }

    #[test]
    fn shift_is_part_of_an_upper_case_letter() {
        assert_eq!(
            action(KeyCode::Char('G'), KeyModifiers::SHIFT),
            Some(Action::Bottom),
            "terminals report G with shift held"
        );
        assert_eq!(
            action(KeyCode::Char('?'), KeyModifiers::SHIFT),
            Some(Action::Help),
            "and ? likewise"
        );
    }

    #[test]
    fn each_binding_says_what_it_does_and_how_it_is_written() {
        for binding in &BINDINGS {
            assert!(
                !binding.label.is_empty(),
                "{:?} has a label",
                binding.action
            );
            assert!(
                !binding.does.is_empty(),
                "{:?} has a description",
                binding.action
            );
            assert!(
                !binding.hint.is_empty(),
                "{:?} has a footer word",
                binding.action
            );
        }
    }
}
