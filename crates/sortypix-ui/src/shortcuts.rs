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
}
impl Action {
    pub const ALL: [Self; 18] = [
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
        }
    }
    pub fn default_binding(self) -> Shortcut {
        let key = match self {
            Self::Keep => "K",
            Self::Reject => "X",
            Self::Undo | Self::Zoom => "Z",
            Self::Expand => "Enter",
            Self::Collapse => "Escape",
            Self::RotateRight | Self::RotateLeft => "R",
            Self::Next => "ArrowRight",
            Self::Previous => "ArrowLeft",
            Self::Down => "ArrowDown",
            Self::Up => "ArrowUp",
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
pub fn binding(settings: &Settings, action: Action) -> Shortcut {
    settings
        .shortcuts
        .get(action.id())
        .cloned()
        .unwrap_or_else(|| action.default_binding())
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
    Action::ALL.into_iter().find(|action| {
        let b = binding(settings, *action);
        Key::from_name(&b.key) == Some(key)
            && b.ctrl == modifiers.ctrl
            && b.shift == modifiers.shift
            && b.alt == modifiers.alt
    })
}
pub fn assign(settings: &mut Settings, action: Action, candidate: Shortcut) -> Result<(), String> {
    if !candidate.key.is_empty() {
        let key = Key::from_name(&candidate.key).ok_or("Unknown key")?;
        for other in Action::ALL {
            let b = binding(settings, other);
            if other != action
                && Key::from_name(&b.key) == Some(key)
                && b.ctrl == candidate.ctrl
                && b.shift == candidate.shift
                && b.alt == candidate.alt
            {
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
        assert!(!editor.capture(&mut s, &[event(Key::X, false)]));
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
