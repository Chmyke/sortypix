use sortypix_app::cache::{Cache, accept_result};
#[test]
fn memory_stays_bounded_and_recent_images_survive() {
    let mut cache = Cache::new(64);
    for n in 0..100 {
        cache.insert(n, n, 16);
    }
    assert!(cache.used_bytes() <= 64);
    assert!(cache.get(&0).is_none());
    assert_eq!(cache.get(&98), Some(&98));
    cache.insert(100, 100, 16);
    assert_eq!(cache.get(&98), Some(&98));
    cache.insert(101, 101, 1000);
    assert!(cache.used_bytes() <= 64);
    assert!(cache.get(&101).is_none());
}
#[test]
fn late_results_cannot_enter_another_folder() {
    assert!(!accept_result(2, 1));
    assert!(accept_result(2, 2));
}
#[test]
fn protected_entries_are_never_evicted_to_make_room() {
    let mut cache = Cache::new(64);
    cache.insert(1, 1, 32);
    cache.insert(2, 2, 16);
    cache.insert(3, 3, 16);
    let visible = |key: &i32| *key == 1;

    // Only 32 evictable bytes: an entry needing 48 is refused and nothing moves.
    assert!(!cache.insert_protecting(4, 4, 48, visible));
    assert_eq!(cache.used_bytes(), 64);
    assert!(cache.get(&2).is_some() && cache.get(&3).is_some());

    // The oldest unprotected entries make room; the protected one stays.
    assert!(cache.insert_protecting(5, 5, 32, visible));
    assert_eq!(cache.get(&1), Some(&1));
    assert_eq!(cache.get(&5), Some(&5));
    assert!(cache.get(&2).is_none() && cache.get(&3).is_none());
    assert!(cache.used_bytes() <= 64);
}
