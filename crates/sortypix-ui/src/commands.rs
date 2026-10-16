use sortypix_core::{Decision, PhotoId};
#[derive(Debug, PartialEq)]
pub enum UiCommand {
    Decide(PhotoId, Decision),
    RejectThrough(PhotoId),
}
pub fn click(
    id: PhotoId,
    right: bool,
    inverted: bool,
    control: bool,
    middle: bool,
    dragged: bool,
) -> Option<UiCommand> {
    if dragged {
        return None;
    }
    if control || middle {
        return Some(UiCommand::RejectThrough(id));
    }
    Some(UiCommand::Decide(
        id,
        if right != inverted {
            Decision::Keep
        } else {
            Decision::Reject
        },
    ))
}
