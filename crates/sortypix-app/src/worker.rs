//! ROLE: Owns the engine off the UI thread; publishes staged catalogs and rejects stale work.
use crate::{
    catalog::{PhotoMetadata, PhysicalPhoto, read_metadata},
    duplicates::exact_duplicate_groups,
    engine::Engine,
};
use sortypix_core::{Decision, PhotoId};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};
pub enum Command {
    Open(PathBuf),
    VisibleReady,
    Decide(Vec<PhotoId>, Decision),
    Rotate(PhotoId, bool),
    Undo,
}
pub struct Snapshot {
    pub photos: Vec<PhysicalPhoto>,
    pub turns: Vec<u8>,
    pub root: Option<PathBuf>,
    pub undo_count: usize,
    pub error: Option<String>,
    pub duplicates_scanning: bool,
    pub catalog_final: bool,
}
struct MetadataJob {
    generation: u64,
    photos: Vec<PhysicalPhoto>,
}
struct MetadataResult {
    generation: u64,
    metadata: Vec<PhotoMetadata>,
}
struct DuplicateJob {
    revision: u64,
    photos: Vec<PhysicalPhoto>,
}
struct DuplicateResult {
    revision: u64,
    ids: Vec<PhotoId>,
    stamps: Vec<(PathBuf, FileStamp)>,
    error: Option<String>,
}
#[derive(PartialEq, Eq)]
struct FileStamp {
    bytes: u64,
    modified: std::time::SystemTime,
}
fn stamp(path: &std::path::Path) -> Result<FileStamp, String> {
    let metadata = path.metadata().map_err(|e| e.to_string())?;
    Ok(FileStamp {
        bytes: metadata.len(),
        modified: metadata.modified().map_err(|e| e.to_string())?,
    })
}
impl DuplicateResult {
    fn unchanged(&self) -> bool {
        self.stamps
            .iter()
            .all(|(path, before)| stamp(path).is_ok_and(|now| now == *before))
    }
}
fn analyse_duplicates(job: DuplicateJob) -> Result<DuplicateResult, String> {
    // Content hashing and byte confirmation stay entirely on the analysis thread.
    // Only cheap size/mtime guards run on the command thread before moving files.
    let stamps = job
        .photos
        .iter()
        .flat_map(|p| &p.members)
        .map(|p| Ok((p.clone(), stamp(p)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let ids = exact_duplicate_groups(&job.photos)?;
    let result = DuplicateResult {
        revision: job.revision,
        stamps: if ids.is_empty() { Vec::new() } else { stamps },
        ids,
        error: None,
    };
    if !result.unchanged() {
        return Err("Files changed during duplicate analysis; no automatic move was made".into());
    }
    Ok(result)
}
pub struct Worker {
    pub commands: SyncSender<Command>,
    pub events: Receiver<Snapshot>,
}
impl Worker {
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self::start_with_jobs(wake, analyse_duplicates, |photos| read_metadata(&photos))
    }
    #[cfg(test)]
    fn start_with_analysis(
        wake: Arc<dyn Fn() + Send + Sync>,
        analyse: impl FnMut(DuplicateJob) -> Result<DuplicateResult, String> + Send + 'static,
    ) -> Self {
        Self::start_with_jobs(wake, analyse, |photos| read_metadata(&photos))
    }
    fn start_with_jobs(
        wake: Arc<dyn Fn() + Send + Sync>,
        mut analyse: impl FnMut(DuplicateJob) -> Result<DuplicateResult, String> + Send + 'static,
        mut metadata_reader: impl FnMut(Vec<PhysicalPhoto>) -> Vec<PhotoMetadata> + Send + 'static,
    ) -> Self {
        let (send, receive) = mpsc::sync_channel(32);
        let (events, results) = mpsc::channel();
        let (jobs, work) = mpsc::sync_channel::<DuplicateJob>(1);
        let (ready, duplicates) = mpsc::sync_channel(1);
        let (metadata_jobs, metadata_work) = mpsc::channel::<MetadataJob>();
        let (metadata_ready, metadata_results) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(mut job) = metadata_work.recv() {
                while let Ok(newer) = metadata_work.try_recv() {
                    job = newer;
                }
                let metadata = metadata_reader(job.photos);
                if metadata_ready
                    .send(MetadataResult {
                        generation: job.generation,
                        metadata,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            while let Ok(job) = work.recv() {
                let revision = job.revision;
                let result = match analyse(job) {
                    Ok(result) => result,
                    Err(e) => DuplicateResult {
                        revision,
                        ids: Vec::new(),
                        stamps: Vec::new(),
                        error: Some(e),
                    },
                };
                if ready.send(result).is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut engine = Engine::default();
            let mut revision = 0;
            let mut automatic = false;
            let mut scanning = false;
            let mut needs_job = false;
            let mut folder_generation = 0;
            let mut catalog_final = true;
            let mut visible_ready = false;
            loop {
                match receive.recv_timeout(Duration::from_millis(15)) {
                    Ok(command) => {
                        revision += 1;
                        let result = match command {
                            Command::Open(root) => {
                                let result = engine.open_provisional(&root);
                                if result.is_ok() {
                                    folder_generation += 1;
                                    catalog_final = false;
                                    visible_ready = false;
                                    automatic = true;
                                    needs_job = false;
                                    let _ = metadata_jobs.send(MetadataJob {
                                        generation: folder_generation,
                                        photos: engine.photos(),
                                    });
                                }
                                result
                            }
                            Command::VisibleReady => {
                                visible_ready = true;
                                Ok(())
                            }
                            Command::Decide(ids, decision) => engine.decide(&ids, decision),
                            Command::Rotate(id, clockwise) => engine.rotate(id, clockwise),
                            Command::Undo => {
                                automatic = false;
                                needs_job = false;
                                engine.undo()
                            }
                        };
                        if automatic && catalog_final && visible_ready {
                            needs_job = true;
                        }
                        if events
                            .send(snapshot(
                                &engine,
                                result.err(),
                                scanning || needs_job,
                                catalog_final,
                            ))
                            .is_err()
                        {
                            break;
                        }
                        wake();
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if let Ok(result) = metadata_results.try_recv()
                    && result.generation == folder_generation
                {
                    engine.apply_metadata(result.metadata);
                    catalog_final = true;
                    if automatic && visible_ready {
                        needs_job = true;
                    }
                    if events
                        .send(snapshot(
                            &engine,
                            None,
                            scanning || needs_job,
                            catalog_final,
                        ))
                        .is_err()
                    {
                        break;
                    }
                    wake();
                }
                if let Ok(result) = duplicates.try_recv() {
                    scanning = false;
                    if automatic && result.revision == revision {
                        needs_job = false;
                        let unchanged = result.unchanged();
                        let error = if let Some(error) = result.error {
                            Some(error)
                        } else if unchanged && !result.ids.is_empty() {
                            engine.decide(&result.ids, Decision::Reject).err()
                        } else {
                            None
                        };
                        if events
                            .send(snapshot(&engine, error, false, catalog_final))
                            .is_err()
                        {
                            break;
                        }
                        wake();
                    }
                }
                if automatic
                    && catalog_final
                    && visible_ready
                    && needs_job
                    && !scanning
                    && jobs
                        .try_send(DuplicateJob {
                            revision,
                            photos: engine.photos(),
                        })
                        .is_ok()
                {
                    scanning = true;
                    needs_job = false;
                }
            }
        });
        Self {
            commands: send,
            events: results,
        }
    }
}
fn snapshot(
    engine: &Engine,
    error: Option<String>,
    duplicates_scanning: bool,
    catalog_final: bool,
) -> Snapshot {
    let photos = engine.photos();
    let turns = photos.iter().map(|p| engine.turns(p.id)).collect();
    Snapshot {
        photos,
        turns,
        root: engine.root().map(PathBuf::from),
        undo_count: engine.undo_count(),
        error,
        duplicates_scanning,
        catalog_final,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_publishes_names_before_metadata_and_defers_duplicates_until_visible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, b"photo").unwrap();
        let (metadata_started, metadata_waiting) = mpsc::channel();
        let (release_metadata, metadata_gate) = mpsc::channel();
        let (duplicates_started, duplicates_waiting) = mpsc::channel();
        let worker = Worker::start_with_jobs(
            Arc::new(|| {}),
            move |job| {
                duplicates_started.send(()).unwrap();
                analyse_duplicates(job)
            },
            move |photos| {
                metadata_started.send(()).unwrap();
                metadata_gate.recv().unwrap();
                photos
                    .into_iter()
                    .map(|photo| crate::catalog::PhotoMetadata {
                        key: photo.members[0].clone(),
                        timestamp: 1,
                        aspect_ratio: 1.5,
                        capture_time: None,
                    })
                    .collect()
            },
        );

        worker
            .commands
            .send(Command::Open(dir.path().to_owned()))
            .unwrap();
        let provisional = worker.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!provisional.catalog_final);
        assert!(!provisional.duplicates_scanning);
        metadata_waiting
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(duplicates_waiting.try_recv().is_err());

        release_metadata.send(()).unwrap();
        let final_catalog = worker.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(final_catalog.catalog_final);
        assert!(!final_catalog.duplicates_scanning);
        assert!(duplicates_waiting.try_recv().is_err());

        worker.commands.send(Command::VisibleReady).unwrap();
        assert!(
            worker
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .duplicates_scanning
        );
        duplicates_waiting
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
    }
    #[test]
    fn no_candidates_need_no_final_file_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single.jpg");
        std::fs::write(&path, b"photo").unwrap();
        let result = analyse_duplicates(DuplicateJob {
            revision: 7,
            photos: vec![PhysicalPhoto {
                aspect_ratio: 1.5,
                capture_time: None,
                id: PhotoId(1),
                members: vec![path.clone()],
            }],
        })
        .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(result.ids.is_empty());
        assert!(result.stamps.is_empty());
        assert!(result.unchanged());
    }
    #[test]
    fn modified_candidate_invalidates_background_result() {
        let dir = tempfile::tempdir().unwrap();
        let paths = [dir.path().join("a.jpg"), dir.path().join("a-copy.jpg")];
        for path in &paths {
            std::fs::write(path, b"same").unwrap();
        }
        let result = analyse_duplicates(DuplicateJob {
            revision: 2,
            photos: paths
                .iter()
                .enumerate()
                .map(|(i, path)| PhysicalPhoto {
                    aspect_ratio: 1.5,
                    capture_time: None,
                    id: PhotoId(i as u64),
                    members: vec![path.clone()],
                })
                .collect(),
        })
        .unwrap();
        assert_eq!(result.ids, vec![PhotoId(1)]);
        assert!(result.unchanged());
        std::fs::write(&paths[0], b"changed survivor").unwrap();
        assert!(!result.unchanged());
    }
    #[test]
    fn commands_and_undo_continue_while_analysis_waits_and_stale_results_do_not_move() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"same").unwrap();
        std::fs::write(dir.path().join("a-copy.jpg"), b"same").unwrap();
        let (started, waiting) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let worker = Worker::start_with_analysis(Arc::new(|| {}), move |job| {
            let result = analyse_duplicates(job);
            started.send(()).unwrap();
            gate.recv().unwrap();
            result
        });
        let next = || worker.events.recv_timeout(Duration::from_secs(2)).unwrap();
        worker
            .commands
            .send(Command::Open(dir.path().to_owned()))
            .unwrap();
        let mut opened = next();
        while !opened.catalog_final {
            opened = next();
        }
        worker.commands.send(Command::VisibleReady).unwrap();
        assert!(next().duplicates_scanning);
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        worker
            .commands
            .send(Command::Decide(vec![opened.photos[0].id], Decision::Keep))
            .unwrap();
        assert_eq!(next().photos.len(), 1);
        worker.commands.send(Command::Undo).unwrap();
        assert_eq!(next().photos.len(), 2);
        release.send(()).unwrap();
        // A subsequent command must still see both files, even as the old analysis finishes.
        for _ in 0..4 {
            worker
                .commands
                .send(Command::Rotate(opened.photos[0].id, true))
                .unwrap();
            let state = next();
            assert_eq!(state.photos.len(), 2);
            assert!(state.error.is_none());
        }
        assert!(!dir.path().join("rejected").exists());
    }
}
