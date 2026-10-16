//! ROLE: Couples physical transactions to pure session state across folders.
//! OWNER: One engine lives on the application worker, never on the UI thread.
use crate::{
    catalog::{PhotoMetadata, PhysicalPhoto, read_metadata, scan_names},
    storage::Storage,
};
use sortypix_core::{Decision, Photo, PhotoId, Session, Status};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
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
}
impl Engine {
    pub fn open(&mut self, root: &Path) -> Result<(), String> {
        self.open_provisional(root)?;
        let metadata = read_metadata(&self.catalog);
        self.apply_metadata(metadata);
        Ok(())
    }
    pub fn open_provisional(&mut self, root: &Path) -> Result<(), String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("Cannot open {}: {e}", root.display()))?;
        let mut photos = scan_names(&root)?;
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
            .map(|value| (value.key, (value.timestamp, value.aspect_ratio)))
            .collect();
        for photo in &mut self.catalog {
            if let Some((_, aspect_ratio)) = metadata.get(&photo.members[0]) {
                photo.aspect_ratio = *aspect_ratio;
            }
        }
        self.catalog.sort_by_cached_key(|photo| {
            (
                metadata
                    .get(&photo.members[0])
                    .map_or(0, |(timestamp, _)| *timestamp),
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
        if report.errors.is_empty() {
            Ok(())
        } else {
            Err(report.errors.join("\n"))
        }
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
