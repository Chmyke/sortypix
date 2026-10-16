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
