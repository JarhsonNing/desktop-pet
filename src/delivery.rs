//! Pet delivery selection. Stable content references, independent of Explorer/UI.
use crate::content::{Entry, Item, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pick {
    pub entry: String,
    pub item: usize,
    pub file: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Choice {
    pub pick: Pick,
    pub name: String,
    pub favorite: bool,
    pub directory: bool,
    pub selected: bool,
    pub recent: bool,
}

pub fn entry_picks(entry: &Entry) -> Vec<Pick> {
    entry
        .items
        .iter()
        .enumerate()
        .flat_map(|(item, value)| {
            let files = match value {
                Item::Files(files) => (0..files.len()).map(Some).collect(),
                _ => vec![None],
            };
            files.into_iter().map(move |file| Pick {
                entry: entry.id.clone(),
                item,
                file,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Delivery {
    pub version: u32,
    pub epoch: u64,
    pub selected: BTreeSet<Pick>,
    pub recent: BTreeSet<Pick>,
    incoming: BTreeMap<String, u64>,
    #[serde(skip)]
    active: Option<BTreeSet<Pick>>,
}
impl Default for Delivery {
    fn default() -> Self {
        Self {
            version: 1,
            epoch: 0,
            selected: BTreeSet::new(),
            recent: BTreeSet::new(),
            incoming: BTreeMap::new(),
            active: None,
        }
    }
}
impl Delivery {
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let state: Self = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        if state.version != 1 {
            return Err(std::io::Error::other("投递状态格式较新，未覆盖该文件"));
        }
        Ok(state)
    }
    pub fn capture(&mut self, task: String) {
        self.incoming.insert(task, self.epoch);
    }
    pub fn completed(&mut self, task: &str, entry: &Entry) {
        if self.incoming.remove(task) == Some(self.epoch) {
            for pick in entry_picks(entry) {
                self.selected.insert(pick.clone());
                self.recent.insert(pick);
            }
        }
    }
    pub fn forget_task(&mut self, task: &str) -> bool {
        self.incoming.remove(task).is_some()
    }
    pub fn reconcile(&mut self, snapshot: &Snapshot) {
        let valid: BTreeSet<_> = snapshot
            .transfer
            .iter()
            .chain(&snapshot.clipboard)
            .flat_map(entry_picks)
            .collect();
        self.selected.retain(|pick| valid.contains(pick));
        self.recent.retain(|pick| valid.contains(pick));
    }
    pub fn set(&mut self, picks: Vec<Pick>, snapshot: &Snapshot) {
        self.selected = picks.into_iter().collect();
        self.reconcile(snapshot);
    }
    pub fn begin(&mut self) -> Result<(), &'static str> {
        if self.active.is_some() {
            return Err("正在投递，请稍候");
        }
        if self.selected.is_empty() {
            return Err("请先选择文件");
        }
        self.active = Some(self.selected.clone());
        self.epoch += 1;
        self.recent.clear();
        Ok(())
    }
    pub fn finish(&mut self, accepted: bool) {
        if let Some(active) = self.active.take()
            && accepted
        {
            self.selected.retain(|pick| !active.contains(pick));
        }
    }
    pub fn choices(&self, snapshot: &Snapshot) -> Vec<Choice> {
        snapshot
            .transfer
            .iter()
            .chain(&snapshot.clipboard)
            .flat_map(|entry| {
                entry_picks(entry)
                    .into_iter()
                    .map(|pick| {
                        let (name, directory) = match &entry.items[pick.item] {
                            Item::Files(files) => {
                                let file = &files[pick.file.unwrap()];
                                (file.name.clone(), file.directory)
                            }
                            Item::Image { .. } => (format!("{}.png", entry.title), false),
                            Item::Text { .. } => (
                                format!("{}.txt", entry.title.chars().take(45).collect::<String>()),
                                false,
                            ),
                        };
                        Choice {
                            selected: self.selected.contains(&pick),
                            recent: self.recent.contains(&pick),
                            favorite: entry.favorite,
                            pick,
                            name,
                            directory,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}
