//! ROLE: Couples physical transactions to pure session state across folders.
//! OWNER: One engine lives on the application worker, never on the UI thread.
use crate::{
    catalog::{PhotoMetadata, PhysicalPhoto, read_metadata, scan_names_with},
    storage::Storage,
};
use sortypix_core::{Decision, Photo, PhotoId, Session, Status};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
/// What is listed so far of a folder being opened, with ids and quarter turns.
pub struct Partial {
    pub root: PathBuf,
    pub photos: Vec<PhysicalPhoto>,
    pub turns: Vec<u8>,
    pub undo_count: usize,
}
struct Gesture {
    physical: bool,
    logical: bool,
}
#[derive(Default)]
pub struct Engine {
    root: Option<PathBuf>,
    catalog: Vec<PhysicalPhoto>,
    ids: BTreeMap<PathBuf, PhotoId>,
    session: Session,
    storage: Storage,
    history: Vec<Gesture>,
    recovery_pending: bool,
    /// The open folder is on a read-only volume (locked card, disc, protected share).
    read_only: bool,
}
/// Shown when the folder opens and when a gesture is refused.
pub const READ_ONLY: &str = "This folder is read-only (locked card, disc or protected drive): its photos can be viewed, not sorted.";
impl Engine {
    pub fn open(&mut self, root: &Path) -> Result<(), String> {
        self.open_provisional(root)?;
        let metadata = read_metadata(&self.catalog);
        self.apply_metadata(metadata);
        Ok(())
    }
    pub fn open_provisional(&mut self, root: &Path) -> Result<(), String> {
        self.open_provisional_with(root, |_| {})
    }
    /// `open_provisional`, handing `partial` the photos listed so far while a large folder is
    /// read.
    pub fn open_provisional_with(
        &mut self,
        root: &Path,
        mut partial: impl FnMut(Partial),
    ) -> Result<(), String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("Cannot open {}: {e}", root.display()))?;
        let (ids, session, undo_count) = (&mut self.ids, &self.session, self.history.len());
        let mut photos = scan_names_with(&root, |mut photos| {
            for photo in &mut photos {
                let next = PhotoId(ids.len() as u64);
                photo.id = *ids.entry(photo.members[0].clone()).or_insert(next);
            }
            let turns = photos
                .iter()
                .map(|p| session.photo(p.id).map_or(0, |p| p.quarter_turns))
                .collect();
            partial(Partial {
                root: root.clone(),
                photos,
                turns,
                undo_count,
            });
        })?;
        self.read_only = crate::storage::read_only(&root);
        for photo in &mut photos {
            let next = PhotoId(self.ids.len() as u64);
            photo.id = *self.ids.entry(photo.members[0].clone()).or_insert(next);
            self.session.register(Photo {
                id: photo.id,
                status: Status::Pending,
                quarter_turns: 0,
            });
        }
        self.root = Some(root);
        self.catalog = photos;
        Ok(())
    }
    pub fn apply_metadata(&mut self, metadata: Vec<PhotoMetadata>) {
        let metadata: BTreeMap<_, _> = metadata
            .into_iter()
            .map(|value| {
                let fields = (value.timestamp, value.aspect_ratio, value.capture_time);
                (value.key, fields)
            })
            .collect();
        for photo in &mut self.catalog {
            if let Some((_, aspect_ratio, capture_time)) = metadata.get(&photo.members[0]) {
                photo.aspect_ratio = *aspect_ratio;
                photo.capture_time = *capture_time;
            }
        }
        self.catalog.sort_by_cached_key(|photo| {
            (
                metadata
                    .get(&photo.members[0])
                    .map_or(0, |(timestamp, _, _)| *timestamp),
                photo.members[0].clone(),
            )
        });
    }
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
    pub fn photos(&self) -> Vec<PhysicalPhoto> {
        self.catalog.clone()
    }
    pub fn decide(&mut self, ids: &[PhotoId], decision: Decision) -> Result<(), String> {
        if self.recovery_pending {
            return Err("Use Undo to finish recovering the previous operation first.".into());
        }
        if ids.is_empty() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
            return Err("Empty or repeated selection".into());
        }
        if self.read_only {
            return Err(READ_ONLY.into());
        }
        let gone = self.forget_vanished(ids);
        let ids: Vec<PhotoId> = ids
            .iter()
            .copied()
            .filter(|id| !gone.contains(id))
            .collect();
        if ids.is_empty() {
            return Err(vanished_message(gone.len()));
        }
        let ids = &ids[..];
        let lookup: BTreeMap<_, _> = self.catalog.iter().map(|p| (p.id, p)).collect();
        let photos = ids
            .iter()
            .map(|id| {
                lookup
                    .get(id)
                    .cloned()
                    .cloned()
                    .ok_or("Photo no longer available".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if ids.iter().any(|id| {
            self.session
                .photo(*id)
                .is_none_or(|p| p.status != Status::Pending)
        }) {
            return Err("Photo already sorted".into());
        }
        let before = self.storage.undo_count();
        let report = self.storage.apply(&photos, decision);
        let logical = !report.completed.is_empty();
        if logical {
            self.session.decide(&report.completed, decision)?;
        }
        if self.storage.undo_count() > before {
            self.history.push(Gesture {
                physical: true,
                logical,
            });
        }
        let completed: BTreeSet<_> = report.completed.into_iter().collect();
        self.catalog.retain(|p| !completed.contains(&p.id));
        self.recovery_pending = !report.errors.is_empty() && self.storage.undo_count() > before;
        let mut errors = report.errors;
        if !gone.is_empty() {
            errors.push(vanished_message(gone.len()));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    /// Files removed by another program since the listing leave their photo; a photo with no
    /// file left leaves the folder (returned), without a gesture to undo.
    fn forget_vanished(&mut self, ids: &[PhotoId]) -> BTreeSet<PhotoId> {
        let wanted: BTreeSet<_> = ids.iter().collect();
        let mut gone = BTreeSet::new();
        for photo in self.catalog.iter_mut().filter(|p| wanted.contains(&p.id)) {
            photo
                .members
                .retain(|m| std::fs::symlink_metadata(m).is_ok());
            if photo.members.is_empty() {
                gone.insert(photo.id);
            }
        }
        self.catalog.retain(|p| !gone.contains(&p.id));
        gone
    }
    pub fn rotate(&mut self, id: PhotoId, clockwise: bool) -> Result<(), String> {
        if self.recovery_pending {
            return Err("Finish recovery with Undo first.".into());
        }
        if !self.catalog.iter().any(|p| p.id == id) {
            return Err("Photo no longer available".into());
        }
        self.session.rotate(id, clockwise)?;
        self.history.push(Gesture {
            physical: false,
            logical: true,
        });
        Ok(())
    }
    pub fn undo(&mut self) -> Result<(), String> {
        let last = self.history.last().ok_or("Nothing to undo")?;
        if last.physical {
            let report = self.storage.undo();
            if !report.errors.is_empty() {
                self.recovery_pending = true;
                return Err(report.errors.join("\n"));
            }
        }
        if last.logical {
            self.session.undo()?;
        }
        self.history.pop();
        self.recovery_pending = false;
        if let Some(root) = self.root.clone() {
            self.open(&root)?;
        }
        Ok(())
    }
    pub fn turns(&self, id: PhotoId) -> u8 {
        self.session.photo(id).map_or(0, |p| p.quarter_turns)
    }
    pub fn undo_count(&self) -> usize {
        self.history.len()
    }
}
fn vanished_message(count: usize) -> String {
    if count == 1 {
        "That photo was removed by another program: it has left the grid.".into()
    } else {
        format!("{count} photos were removed by another program: they have left the grid.")
    }
}
