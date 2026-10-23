//! One keyboard map for dispatch, editing and tooltips. Empty keys disable actions.
use eframe::egui::{self, Key, Modifiers};
use sortypix_app::settings::{Settings, Shortcut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Keep,
    Reject,
    Undo,
    Expand,
    Collapse,
    Zoom,
    RotateRight,
    RotateLeft,
    Next,
    Previous,
    Down,
    Up,
    Open,
    KeepAll,
    RejectAll,
    OpenKept,
    OpenRejected,
    OpenSource,
    /// Sortypix AI: the details card of the hovered or selected photo.
    #[cfg(feature = "ai")]
    PhotoCard,
}
/// Actions of the edition: Sortypix AI adds its photo card.
const COUNT: usize = 18 + cfg!(feature = "ai") as usize;
impl Action {
    pub const ALL: [Self; COUNT] = [
        Self::Keep,
        Self::Reject,
        Self::Undo,
        Self::Expand,
        Self::Collapse,
        Self::Zoom,
        Self::RotateRight,
        Self::RotateLeft,
        Self::Next,
        Self::Previous,
        Self::Down,
        Self::Up,
        Self::Open,
        Self::KeepAll,
        Self::RejectAll,
        Self::OpenKept,
        Self::OpenRejected,
        Self::OpenSource,
        #[cfg(feature = "ai")]
        Self::PhotoCard,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Reject => "reject",
            Self::Undo => "undo",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::Zoom => "zoom",
            Self::RotateRight => "rotate_right",
            Self::RotateLeft => "rotate_left",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Down => "down",
            Self::Up => "up",
            Self::Open => "open",
            Self::KeepAll => "keep_all",
            Self::RejectAll => "reject_all",
            Self::OpenKept => "open_kept",
            Self::OpenRejected => "open_rejected",
            Self::OpenSource => "open_source",
            #[cfg(feature = "ai")]
            Self::PhotoCard => "photo_card",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "Keep photo",
            Self::Reject => "Reject photo",
            Self::Undo => "Undo",
            Self::Expand => "Open full screen",
            Self::Collapse => "Back to grid",
            Self::Zoom => "Fit / 100%",
            Self::RotateRight => "Rotate right",
            Self::RotateLeft => "Rotate left",
            Self::Next => "Next photo",
            Self::Previous => "Previous photo",
            Self::Down => "Next row",
            Self::Up => "Previous row",
            Self::Open => "Open photo folder",
            Self::KeepAll => "Keep all remaining",
            Self::RejectAll => "Reject all remaining",
            Self::OpenKept => "Open kept folder",
            Self::OpenRejected => "Open rejected folder",
            Self::OpenSource => "Open source folder in Explorer",
            #[cfg(feature = "ai")]
            Self::PhotoCard => "Photo details",
        }
    }
    pub fn default_binding(self) -> Shortcut {
        let key = match self {
            Self::Keep => "ArrowUp",
            Self::Reject => "ArrowDown",
            Self::Undo | Self::Zoom => "Z",
            Self::Expand => "Enter",
            Self::Collapse => "Escape",
            Self::RotateRight | Self::RotateLeft => "R",
            Self::Next => "ArrowRight",
            Self::Previous => "ArrowLeft",
            #[cfg(feature = "ai")]
            Self::PhotoCard => "I",
            _ => "",
        };
        Shortcut {
            key: key.into(),
            ctrl: self == Self::Undo,
            shift: self == Self::RotateLeft,
            alt: false,
        }
    }
}
/// A saved choice always wins: a default that a later version gave to keys the person
/// already chose for something else stays unassigned (shown in the editor), never shared.
pub fn binding(settings: &Settings, action: Action) -> Shortcut {
    if let Some(saved) = settings.shortcuts.get(action.id()) {
        return saved.clone();
    }
    let default = action.default_binding();
    if settings
        .shortcuts
        .values()
        .any(|saved| same_keys(saved, &default))
    {
        Shortcut::default()
    } else {
        default
    }
}
fn same_keys(a: &Shortcut, b: &Shortcut) -> bool {
    !a.key.is_empty()
        && Key::from_name(&a.key).is_some()
        && Key::from_name(&a.key) == Key::from_name(&b.key)
        && a.ctrl == b.ctrl
        && a.shift == b.shift
        && a.alt == b.alt
}
pub fn label(binding: &Shortcut) -> String {
    if binding.key.is_empty() {
        return "Not assigned".into();
    }
    let mut parts = Vec::new();
    if binding.ctrl {
        parts.push("Ctrl");
    }
    if binding.alt {
        parts.push("Alt");
    }
    if binding.shift {
        parts.push("Shift");
    }
    parts.push(&binding.key);
    parts.join("+")
}
pub fn from_key(key: Key, modifiers: Modifiers) -> Shortcut {
    Shortcut {
        key: key.name().into(),
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        alt: modifiers.alt,
    }
}
pub fn resolve(settings: &Settings, key: Key, modifiers: Modifiers) -> Option<Action> {
    if modifiers.mac_cmd {
        return None;
    }
    let pressed = from_key(key, modifiers);
    Action::ALL
        .into_iter()
        .find(|action| same_keys(&binding(settings, *action), &pressed))
}
pub fn assign(settings: &mut Settings, action: Action, candidate: Shortcut) -> Result<(), String> {
    if !candidate.key.is_empty() {
        Key::from_name(&candidate.key).ok_or("Unknown key")?;
        for other in Action::ALL {
            if other != action && same_keys(&binding(settings, other), &candidate) {
                return Err(format!(
                    "{} is already used for {}.",
                    label(&candidate),
                    other.label()
                ));
            }
        }
    }
    settings.shortcuts.insert(action.id().into(), candidate);
    Ok(())
}
#[derive(Default)]
pub struct Editor {
    pub open: bool,
    pub capturing: Option<Action>,
    pub error: Option<String>,
}
impl Editor {
    /// Called instead of dispatch while open; capture can never execute an action.
    pub fn capture(&mut self, settings: &mut Settings, events: &[egui::Event]) -> bool {
        let Some(action) = self.capturing else {
            return false;
        };
        for event in events {
            if let egui::Event::Key {
                key,
                modifiers,
                pressed: true,
                repeat: false,
                ..
            } = event
            {
                if *key == Key::Escape {
                    self.capturing = None;
                    self.error = None;
                    return false;
                }
                if modifiers.mac_cmd {
                    continue;
                }
                match assign(settings, action, from_key(*key, *modifiers)) {
                    Ok(()) => {
                        self.capturing = None;
                        self.error = None;
                        return true;
                    }
                    Err(e) => self.error = Some(e),
                }
                break;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remapping_changes_dispatch_and_distinguishes_modifiers() {
        let mut s = Settings::default();
        assert_eq!(resolve(&s, Key::Z, Modifiers::CTRL), Some(Action::Undo));
        assert_eq!(resolve(&s, Key::Z, Modifiers::NONE), Some(Action::Zoom));
        assert_eq!(resolve(&s, Key::K, Modifiers::ALT), None);
        assign(&mut s, Action::Keep, from_key(Key::A, Modifiers::CTRL)).unwrap();
        assert_eq!(resolve(&s, Key::K, Modifiers::NONE), None);
        assert_eq!(resolve(&s, Key::A, Modifiers::CTRL), Some(Action::Keep));
        assert_eq!(resolve(&s, Key::A, Modifiers::NONE), None);
        let before = s.shortcuts.clone();
        assert!(assign(&mut s, Action::Reject, from_key(Key::A, Modifiers::CTRL)).is_err());
        assert_eq!(s.shortcuts, before);
        assign(&mut s, Action::Keep, Shortcut::default()).unwrap();
        assert_eq!(resolve(&s, Key::A, Modifiers::CTRL), None);
    }
    #[test]
    fn capture_rejects_conflicts_ignores_repeats_and_escape_cancels() {
        let mut s = Settings::default();
        let mut editor = Editor {
            open: true,
            capturing: Some(Action::Keep),
            ..Default::default()
        };
        let event = |key, repeat| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Modifiers::NONE,
        };
        assert!(!editor.capture(&mut s, &[event(Key::ArrowDown, false)]));
        assert!(editor.error.is_some());
        assert!(!editor.capture(&mut s, &[event(Key::A, true)]));
        assert!(s.shortcuts.is_empty());
        assert!(!editor.capture(&mut s, &[event(Key::Escape, false)]));
        assert_eq!(editor.capturing, None);
        assert!(s.shortcuts.is_empty());
        editor.capturing = Some(Action::Keep);
        assert!(editor.capture(&mut s, &[event(Key::A, false)]));
        assert_eq!(editor.capturing, None);
        assert_eq!(resolve(&s, Key::A, Modifiers::NONE), Some(Action::Keep));
    }
    #[test]
    fn arrows_sort_by_default_and_rows_have_no_key() {
        let s = Settings::default();
        let n = Modifiers::NONE;
        assert_eq!(resolve(&s, Key::ArrowUp, n), Some(Action::Keep));
        assert_eq!(resolve(&s, Key::ArrowDown, n), Some(Action::Reject));
        assert_eq!(resolve(&s, Key::ArrowRight, n), Some(Action::Next));
        assert_eq!(resolve(&s, Key::ArrowLeft, n), Some(Action::Previous));
        assert_eq!(resolve(&s, Key::K, n), None);
        assert_eq!(resolve(&s, Key::X, n), None);
        assert!(binding(&s, Action::Down).key.is_empty());
        assert!(binding(&s, Action::Up).key.is_empty());
    }
    #[test]
    fn saved_choice_wins_over_a_newer_default_without_hidden_conflict() {
        let arrow_up = from_key(Key::ArrowUp, Modifiers::NONE);
        // Saved by an older version, where Up was not yet the keep key.
        let mut s = Settings::default();
        s.shortcuts.insert("up".into(), arrow_up.clone());
        s.shortcuts.insert("down".into(), Shortcut::default());
        s.shortcuts.insert(
            "rotate_right".into(),
            from_key(Key::ArrowDown, Modifiers::NONE),
        );
        assert_eq!(resolve(&s, Key::ArrowUp, Modifiers::NONE), Some(Action::Up));
        assert_eq!(
            resolve(&s, Key::ArrowDown, Modifiers::NONE),
            Some(Action::RotateRight)
        );
        assert_eq!(label(&binding(&s, Action::Keep)), "Not assigned");
        assert_eq!(label(&binding(&s, Action::Reject)), "Not assigned");
        // Shown and editable, never a second silent owner of the same keys.
        assign(&mut s, Action::Keep, from_key(Key::K, Modifiers::NONE)).unwrap();
        assert_eq!(resolve(&s, Key::K, Modifiers::NONE), Some(Action::Keep));
        assert!(assign(&mut s, Action::Reject, arrow_up).is_err());
    }
    #[cfg(feature = "ai")]
    #[test]
    fn sortypix_ai_adds_the_photo_card_on_i() {
        let s = Settings::default();
        assert_eq!(Action::ALL.len(), 19);
        assert_eq!(
            resolve(&s, Key::I, Modifiers::NONE),
            Some(Action::PhotoCard)
        );
        // Reassignable like any other action, and kept apart from normal Sortypix's ids.
        let mut s = s;
        assign(&mut s, Action::PhotoCard, from_key(Key::D, Modifiers::NONE)).unwrap();
        assert_eq!(
            resolve(&s, Key::D, Modifiers::NONE),
            Some(Action::PhotoCard)
        );
        assert_eq!(resolve(&s, Key::I, Modifiers::NONE), None);
        assert_eq!(Action::PhotoCard.id(), "photo_card");
    }
    #[cfg(not(feature = "ai"))]
    #[test]
    fn normal_sortypix_keeps_its_eighteen_actions() {
        assert_eq!(Action::ALL.len(), 18);
        assert_eq!(resolve(&Settings::default(), Key::I, Modifiers::NONE), None);
    }
    #[test]
    fn default_shortcuts_are_unique_and_recognized() {
        let s = Settings::default();
        for action in Action::ALL {
            let b = binding(&s, action);
            if b.key.is_empty() {
                continue;
            }
            let key = Key::from_name(&b.key).expect("default key");
            let modifiers = Modifiers {
                ctrl: b.ctrl,
                shift: b.shift,
                alt: b.alt,
                ..Modifiers::NONE
            };
            assert_eq!(resolve(&s, key, modifiers), Some(action));
        }
    }
}
