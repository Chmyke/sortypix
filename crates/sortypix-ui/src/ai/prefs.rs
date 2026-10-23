//! ROLE: Sortypix AI only. Its choices in the settings file shared with Sortypix.
//! OWNER: Gallery.settings; these fields live in `Settings.extra`, which normal Sortypix reads
//! and writes back unchanged.
//! INVARIANTS: A missing or malformed value reads as the default; the capture order stays in
//! the shared `newest_first`, so both editions agree on oldest or newest first.
use super::filters::{Kind, Kinds};
use super::suggest::{PERCENT_DEFAULT, clamp_percent};
use super::view::Order;
use sortypix_app::settings::Settings;

const PERCENT_KEY: &str = "ai_suggest_percent";
const ORDER_KEY: &str = "ai_order";
/// "Show" entry of the menu: `all` or a mark kind (`low`, `off_topic`, `burst`).
const SHOW_KEY: &str = "ai_show";
/// Badges hidden by choice: a list of mark kinds.
const HIDDEN_KEY: &str = "ai_hidden_badges";

pub(crate) fn suggest_percent(settings: &Settings) -> u32 {
    settings
        .extra
        .get(PERCENT_KEY)
        .and_then(serde_json::Value::as_u64)
        .map_or(PERCENT_DEFAULT, clamp_percent)
}

pub(crate) fn set_suggest_percent(settings: &mut Settings, percent: u32) {
    let percent = clamp_percent(u64::from(percent));
    settings
        .extra
        .insert(PERCENT_KEY.into(), serde_json::Value::from(percent));
}

pub(crate) fn order(settings: &Settings) -> Order {
    match settings.extra.get(ORDER_KEY).and_then(|v| v.as_str()) {
        Some("score") => Order::LowestScore,
        Some("score-high") => Order::HighestScore,
        Some("varied") => Order::MostVaried,
        _ if settings.newest_first => Order::Newest,
        _ => Order::Oldest,
    }
}

pub(crate) fn set_order(settings: &mut Settings, order: Order) {
    let value = match order {
        Order::Oldest | Order::Newest => {
            settings.newest_first = order == Order::Newest;
            "capture"
        }
        Order::LowestScore => "score",
        Order::HighestScore => "score-high",
        Order::MostVaried => "varied",
    };
    settings.extra.insert(ORDER_KEY.into(), value.into());
}

/// Photos shown: all (`None`) or only those with one kind of mark.
pub(crate) fn show(settings: &Settings) -> Option<Kind> {
    settings
        .extra
        .get(SHOW_KEY)
        .and_then(|v| v.as_str())
        .and_then(Kind::from_key)
}

pub(crate) fn set_show(settings: &mut Settings, show: Option<Kind>) {
    let value = show.map_or("all", Kind::key);
    settings.extra.insert(SHOW_KEY.into(), value.into());
}

/// Badges hidden by choice (unknown names are ignored).
pub(crate) fn hidden(settings: &Settings) -> Kinds {
    let mut hidden = Kinds::default();
    if let Some(list) = settings.extra.get(HIDDEN_KEY).and_then(|v| v.as_array()) {
        for kind in list
            .iter()
            .filter_map(|v| v.as_str())
            .filter_map(Kind::from_key)
        {
            hidden.set(kind, true);
        }
    }
    hidden
}

pub(crate) fn set_hidden(settings: &mut Settings, hidden: Kinds) {
    let list: Vec<serde_json::Value> = Kind::ALL
        .into_iter()
        .filter(|&k| hidden.has(k))
        .map(|k| k.key().into())
        .collect();
    settings.extra.insert(HIDDEN_KEY.into(), list.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_malformed_values() {
        let mut settings = Settings::default();
        assert_eq!(suggest_percent(&settings), PERCENT_DEFAULT);
        assert_eq!(order(&settings), Order::Oldest);
        settings.extra.insert(PERCENT_KEY.into(), "lots".into());
        settings.extra.insert(ORDER_KEY.into(), 7.into());
        assert_eq!(suggest_percent(&settings), PERCENT_DEFAULT);
        assert_eq!(order(&settings), Order::Oldest);
        settings.extra.insert(PERCENT_KEY.into(), 999.into());
        assert_eq!(suggest_percent(&settings), 50);
        assert_eq!(show(&settings), None);
        assert_eq!(hidden(&settings), Kinds::default());
        settings.extra.insert(SHOW_KEY.into(), "everything".into());
        settings
            .extra
            .insert(HIDDEN_KEY.into(), serde_json::json!(["burst", 3, "colour"]));
        assert_eq!(show(&settings), None);
        assert_eq!(
            hidden(&settings),
            Kinds {
                burst: true,
                ..Kinds::default()
            }
        );
        settings.extra.insert(HIDDEN_KEY.into(), "burst".into());
        assert_eq!(hidden(&settings), Kinds::default());
    }

    #[test]
    fn choices_survive_a_save_by_normal_sortypix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::default();
        set_suggest_percent(&mut settings, 35);
        set_order(&mut settings, Order::MostVaried);
        set_show(&mut settings, Some(Kind::OffTopic));
        let mut hide = Kinds::default();
        hide.set(Kind::Low, true);
        hide.set(Kind::Burst, true);
        set_hidden(&mut settings, hide);
        settings.save(&path).unwrap();
        // Normal Sortypix knows none of these fields: it loads, flips its own order, saves.
        let mut normal = Settings::load(&path);
        normal.newest_first = true;
        normal.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(suggest_percent(&back), 35);
        assert_eq!(order(&back), Order::MostVaried);
        assert_eq!(show(&back), Some(Kind::OffTopic));
        assert_eq!(hidden(&back), hide);
        let mut back = back;
        set_order(&mut back, Order::Oldest);
        assert!(!back.newest_first);
        assert_eq!(order(&back), Order::Oldest);
        set_order(&mut back, Order::Newest);
        assert!(back.newest_first);
        assert_eq!(order(&back), Order::Newest);
        set_order(&mut back, Order::HighestScore);
        assert_eq!(order(&back), Order::HighestScore);
        assert!(back.newest_first, "the capture order stays as it was");
        set_show(&mut back, None);
        set_hidden(&mut back, Kinds::default());
        assert_eq!(show(&back), None);
        assert_eq!(hidden(&back), Kinds::default());
    }
}
