use sortypix_core::{Decision, PhotoId};
use sortypix_ui::commands::{UiCommand, click};
#[test]
fn mouse_commands_preserve_sorting_contract() {
    let id = PhotoId(1);
    assert_eq!(
        click(id, false, false, false, false, false),
        Some(UiCommand::Decide(id, Decision::Reject))
    );
    assert_eq!(
        click(id, true, false, false, false, false),
        Some(UiCommand::Decide(id, Decision::Keep))
    );
    assert_eq!(
        click(id, false, true, false, false, false),
        Some(UiCommand::Decide(id, Decision::Keep))
    );
    assert_eq!(
        click(id, true, true, false, false, false),
        Some(UiCommand::Decide(id, Decision::Reject))
    );
    assert_eq!(
        click(id, false, false, true, false, false),
        Some(UiCommand::RejectThrough(id))
    );
    assert_eq!(
        click(id, false, false, false, true, false),
        Some(UiCommand::RejectThrough(id))
    );
    assert_eq!(click(id, false, false, false, false, true), None);
}
