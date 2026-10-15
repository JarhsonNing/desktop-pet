use desktop_pet::{
    content::{Entry, FileItem, Item, Snapshot, Source, State},
    delivery::{Delivery, entry_picks},
};
use std::path::PathBuf;

fn entry(id: &str, names: &[&str]) -> Entry {
    Entry {
        id: id.into(),
        source: Source::Transfer,
        timestamp: 1,
        favorite: false,
        title: id.into(),
        preview: String::new(),
        kind: "files".into(),
        state: State::Ready,
        errors: vec![],
        origin: None,
        items: vec![Item::Files(
            names
                .iter()
                .map(|name| FileItem {
                    name: (*name).into(),
                    path: PathBuf::from(name),
                    managed: true,
                    directory: false,
                    bytes: 1,
                })
                .collect(),
        )],
    }
}

#[test]
fn all_dropped_batches_accumulate_per_file_and_manual_choices_override() {
    let a = entry("a", &["甲.txt", "乙.txt"]);
    let b = entry("b", &["丙.txt"]);
    let mut state = Delivery::default();
    state.capture("a".into());
    state.completed("a", &a);
    state.capture("b".into());
    state.completed("b", &b);
    assert_eq!(state.selected.len(), 3);
    assert_eq!(state.recent.len(), 3);
    let snapshot = Snapshot {
        transfer: vec![a.clone(), b],
        ..Default::default()
    };
    state.set(vec![entry_picks(&a)[1].clone()], &snapshot);
    assert_eq!(state.selected.len(), 1);
    assert_eq!(
        state
            .choices(&snapshot)
            .iter()
            .find(|choice| choice.selected)
            .unwrap()
            .name,
        "乙.txt"
    );
    state.set(vec![], &snapshot);
    assert!(state.selected.is_empty());
}

#[test]
fn begin_sets_boundary_and_late_old_copies_do_not_join_new_batch() {
    let a = entry("a", &["a"]);
    let late = entry("late", &["large-folder"]);
    let next = entry("next", &["x", "y"]);
    let mut state = Delivery::default();
    state.capture("a".into());
    state.completed("a", &a);
    state.capture("late".into());
    state.begin().unwrap();
    assert!(state.recent.is_empty());
    state.capture("next".into());
    state.completed("next", &next);
    state.completed("late", &late);
    state.finish(true);
    assert_eq!(state.selected.len(), 2);
    assert!(state.selected.iter().all(|pick| pick.entry == "next"));
    assert_eq!(state.recent.len(), 2);
}

#[test]
fn cancellation_and_restart_keep_retry_selection_without_replaying_historical_records() {
    let a = entry("a", &["a", "b"]);
    let mut state = Delivery::default();
    state.capture("a".into());
    state.completed("a", &a);
    state.begin().unwrap();
    state.finish(false);
    assert_eq!(state.selected.len(), 2);
    assert!(state.recent.is_empty());
    let mut restored: Delivery =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    restored.reconcile(&Snapshot {
        transfer: vec![a],
        ..Default::default()
    });
    assert_eq!(restored.selected.len(), 2);
    restored.begin().unwrap();
    restored.finish(true);
    assert!(restored.selected.is_empty());
    restored.reconcile(&Snapshot::default());
    assert!(restored.recent.is_empty());
}

#[test]
fn deleting_records_prunes_selection_and_unknown_state_format_is_preserved() {
    let a = entry("a", &["a"]);
    let mut state = Delivery::default();
    state.capture("a".into());
    state.completed("a", &a);
    state.reconcile(&Snapshot::default());
    assert!(state.selected.is_empty());
    let directory = std::env::temp_dir().join(format!("mochi-delivery-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("state.json");
    std::fs::write(&path, b"{\"version\":999}").unwrap();
    assert!(Delivery::load(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"version\":999}");
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
