#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhotoId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Keep,
    Reject,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Kept,
    Rejected,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Photo {
    pub id: PhotoId,
    pub status: Status,
    pub quarter_turns: u8,
}
use std::collections::{BTreeMap, BTreeSet};
/// Owns pure photo state and gesture history; no filesystem or UI dependencies.
#[derive(Default)]
pub struct Session {
    photos: BTreeMap<PhotoId, Photo>,
    journal: Vec<Vec<Photo>>,
}
impl Session {
    pub fn register(&mut self, photo: Photo) {
        self.photos.entry(photo.id).or_insert(photo);
    }
    pub fn new(photos: Vec<Photo>) -> Result<Self, String> {
        let count = photos.len();
        let photos: BTreeMap<_, _> = photos.into_iter().map(|p| (p.id, p)).collect();
        if count != photos.len() {
            return Err("Duplicate photo identity".into());
        }
        Ok(Self {
            photos,
            journal: Vec::new(),
        })
    }
    pub fn decide(&mut self, ids: &[PhotoId], decision: Decision) -> Result<(), String> {
        if ids.is_empty() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
            return Err("Empty or repeated selection".into());
        }
        let before: Vec<Photo> = ids
            .iter()
            .map(|id| {
                self.photos
                    .get(id)
                    .filter(|p| p.status == Status::Pending)
                    .cloned()
                    .ok_or("Photo is unavailable".to_owned())
            })
            .collect::<Result<_, _>>()?;
        for previous in &before {
            let mut after = previous.clone();
            after.status = match decision {
                Decision::Keep => Status::Kept,
                Decision::Reject => Status::Rejected,
            };
            self.photos.insert(after.id, after);
        }
        self.journal.push(before);
        Ok(())
    }
    pub fn rotate(&mut self, id: PhotoId, clockwise: bool) -> Result<(), String> {
        let photo = self.photos.get_mut(&id).ok_or("Photo is unavailable")?;
        self.journal.push(vec![photo.clone()]);
        photo.quarter_turns = (photo.quarter_turns + if clockwise { 1 } else { 3 }) % 4;
        Ok(())
    }
    pub fn undo(&mut self) -> Result<(), String> {
        let before = self.journal.pop().ok_or("Nothing to undo")?;
        for photo in before {
            self.photos.insert(photo.id, photo);
        }
        Ok(())
    }
    pub fn photo(&self, id: PhotoId) -> Option<&Photo> {
        self.photos.get(&id)
    }
}
