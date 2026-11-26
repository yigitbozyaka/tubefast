use crate::ytm::{Item, LibraryParts, LibrarySource};
use serde::de::{EnumAccess, Error as _, IgnoredAny, VariantAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Deref;

pub const SIDEBAR_WIDTH_DEFAULT: f32 = 244.0;
pub const SIDEBAR_WIDTH_MIN: f32 = 200.0;
pub const SIDEBAR_WIDTH_MAX: f32 = 360.0;
pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 72.0;
pub const FOLDER_COLORS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    Playlist,
    Album,
    Artist,
}

impl ItemKind {
    pub fn of(item: &Item) -> Self {
        if item.is_artist || item.browse_id.starts_with("UC") || item.browse_id.starts_with("MPLAUC") {
            return Self::Artist;
        }
        if item.browse_id.starts_with("MPREb_") || item.browse_id.starts_with("OLAK5uy_") {
            return Self::Album;
        }
        let first = item.subtitle.split(" • ").next().unwrap_or_default().trim();
        if first.eq_ignore_ascii_case("album") || first.eq_ignore_ascii_case("ep") || first.eq_ignore_ascii_case("single") {
            return Self::Album;
        }
        Self::Playlist
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Playlist => "Playlist",
            Self::Album => "Album",
            Self::Artist => "Artist",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LibraryTab {
    #[default]
    All,
    Playlists,
    Albums,
    Artists,
    Liked,
    Folders,
}

impl LibraryTab {
    pub const EVERY: [Self; 6] = [Self::All, Self::Playlists, Self::Albums, Self::Artists, Self::Liked, Self::Folders];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Playlists => "Playlists",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Liked => "Liked songs",
            Self::Folders => "Folders",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum LibraryView {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum LibrarySort {
    #[default]
    Default,
    TitleAsc,
    TitleDesc,
    PinnedFirst,
}

impl LibrarySort {
    pub const EVERY: [Self; 4] = [Self::Default, Self::TitleAsc, Self::TitleDesc, Self::PinnedFirst];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Recently added",
            Self::TitleAsc => "Title (A–Z)",
            Self::TitleDesc => "Title (Z–A)",
            Self::PinnedFirst => "Pinned first",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum SidebarMode {
    #[default]
    Pinned,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum SidebarEntry {
    Item(String),
    Folder(String),
}

struct VariantName(String);

impl<'de> Deserialize<'de> for VariantName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reader;

        impl<'de> Visitor<'de> for Reader {
            type Value = VariantName;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a variant name")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<VariantName, E> {
                Ok(VariantName(value.to_owned()))
            }

            fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<VariantName, E> {
                Ok(VariantName(String::from_utf8_lossy(value).into_owned()))
            }
        }

        deserializer.deserialize_identifier(Reader)
    }
}

macro_rules! unit_enum_falling_back_to_default {
    ($name:ident, [$($variant:ident),+]) => {
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Reader;

                impl<'de> Visitor<'de> for Reader {
                    type Value = $name;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str(stringify!($name))
                    }

                    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<$name, A::Error> {
                        let (VariantName(name), variant) = data.variant()?;
                        variant.unit_variant()?;
                        Ok(match name.as_str() {
                            $(stringify!($variant) => $name::$variant,)+
                            _ => $name::default(),
                        })
                    }
                }

                deserializer.deserialize_enum(stringify!($name), &[$(stringify!($variant)),+], Reader)
            }
        }
    };
}

unit_enum_falling_back_to_default!(LibraryView, [Grid, List]);
unit_enum_falling_back_to_default!(LibrarySort, [Default, TitleAsc, TitleDesc, PinnedFirst]);
unit_enum_falling_back_to_default!(SidebarMode, [Pinned, All]);

enum StoredEntry {
    Known(SidebarEntry),
    Unknown,
}

impl<'de> Deserialize<'de> for StoredEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reader;

        impl<'de> Visitor<'de> for Reader {
            type Value = StoredEntry;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sidebar entry")
            }

            fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<StoredEntry, A::Error> {
                let (VariantName(name), variant) = data.variant()?;
                match name.as_str() {
                    "Item" => Ok(StoredEntry::Known(SidebarEntry::Item(variant.newtype_variant()?))),
                    "Folder" => Ok(StoredEntry::Known(SidebarEntry::Folder(variant.newtype_variant()?))),
                    _ => {
                        variant.newtype_variant::<IgnoredAny>().map_err(A::Error::custom)?;
                        Ok(StoredEntry::Unknown)
                    }
                }
            }
        }

        deserializer.deserialize_enum("SidebarEntry", &["Item", "Folder"], Reader)
    }
}

fn known_entries<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<SidebarEntry>, D::Error> {
    let stored = Vec::<StoredEntry>::deserialize(deserializer)?;
    Ok(stored
        .into_iter()
        .filter_map(|entry| match entry {
            StoredEntry::Known(known) => Some(known),
            StoredEntry::Unknown => None,
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub items: Vec<String>,
    pub open: bool,
    pub color: u8,
}

impl Default for Folder {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            items: Vec::new(),
            open: true,
            color: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarSlot {
    Folder {
        folder_id: String,
        root_index: usize,
    },
    RootItem {
        browse_id: String,
        root_index: usize,
    },
    FolderItem {
        folder_id: String,
        browse_id: String,
        item_index: usize,
    },
}

impl SidebarSlot {
    pub fn is_folder(&self) -> bool {
        matches!(self, Self::Folder { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropTarget {
    Root(usize),
    IntoFolder(String),
    InFolder(String, usize),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidebarLayout {
    pub mode: SidebarMode,
    #[serde(deserialize_with = "known_entries")]
    pub entries: Vec<SidebarEntry>,
    pub folders: Vec<Folder>,
    pub tags: HashMap<String, Vec<String>>,
    pub items: HashMap<String, Item>,
    pub compact: bool,
    pub collapsed: bool,
    pub width: f32,
    pub show_liked: bool,
    pub show_recent: bool,
    pub show_filters: bool,
    pub library_view: LibraryView,
    pub library_sort: LibrarySort,
    pub next_folder: u64,
    pub migrated: bool,
    pub hidden: Vec<String>,
}

impl Default for SidebarLayout {
    fn default() -> Self {
        Self {
            mode: SidebarMode::Pinned,
            entries: Vec::new(),
            folders: Vec::new(),
            tags: HashMap::new(),
            items: HashMap::new(),
            compact: false,
            collapsed: false,
            width: SIDEBAR_WIDTH_DEFAULT,
            show_liked: true,
            show_recent: true,
            show_filters: false,
            library_view: LibraryView::Grid,
            library_sort: LibrarySort::Default,
            next_folder: 1,
            migrated: false,
            hidden: Vec::new(),
        }
    }
}

pub fn usable_sidebar_width(width: f32) -> f32 {
    if width.is_nan() {
        SIDEBAR_WIDTH_DEFAULT
    } else {
        width.clamp(SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    items: Vec<Item>,
    index: HashMap<String, usize>,
}

impl Catalog {
    #[cfg(test)]
    pub fn build(pinned: &[Item], remote: &[Item]) -> Self {
        Self::with_cache(pinned, remote, &[])
    }

    pub fn with_cache(pinned: &[Item], remote: &[Item], cached: &[Item]) -> Self {
        let mut index = HashMap::new();
        let mut items = Vec::new();
        for item in pinned.iter().chain(remote).chain(cached) {
            if item.browse_id.is_empty() || item.browse_id == "VLLM" || item.is_song() {
                continue;
            }
            if !index.contains_key(&item.browse_id) {
                index.insert(item.browse_id.clone(), items.len());
                items.push(item.clone());
            }
        }
        Self { items, index }
    }

    pub fn get(&self, browse_id: &str) -> Option<&Item> {
        self.index.get(browse_id).map(|at| &self.items[*at])
    }
}

impl Deref for Catalog {
    type Target = [Item];

    fn deref(&self) -> &[Item] {
        &self.items
    }
}

fn source_of(kind: ItemKind) -> LibrarySource {
    match kind {
        ItemKind::Playlist => LibrarySource::Playlists,
        ItemKind::Album => LibrarySource::Albums,
        ItemKind::Artist => LibrarySource::Artists,
    }
}

pub fn merge_library(previous: &[Item], fetched: LibraryParts) -> Vec<Item> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for source in [LibrarySource::Playlists, LibrarySource::Albums, LibrarySource::Artists] {
        let fresh = fetched
            .iter()
            .find(|(known, _)| *known == source)
            .and_then(|(_, items)| items.as_ref());
        let kept: Vec<&Item> = match fresh {
            Some(items) => items.iter().collect(),
            None => previous.iter().filter(|item| source_of(ItemKind::of(item)) == source).collect(),
        };
        for item in kept {
            if seen.insert(item.browse_id.clone()) {
                merged.push(item.clone());
            }
        }
    }
    merged
}

impl SidebarLayout {
    pub fn migrate(&mut self, pinned: &[Item], returning_user: bool) {
        if self.migrated {
            return;
        }
        if returning_user {
            self.mode = SidebarMode::All;
        }
        for item in pinned {
            if item.browse_id.is_empty() || item.is_song() {
                continue;
            }
            if !self.is_in_sidebar(&item.browse_id) {
                self.entries.push(SidebarEntry::Item(item.browse_id.clone()));
            }
            self.items.insert(item.browse_id.clone(), item.clone());
        }
        self.migrated = true;
    }

    pub fn cached_items(&self, loaded: &[LibrarySource]) -> Vec<Item> {
        self.sidebar_ids()
            .into_iter()
            .filter_map(|id| self.items.get(id))
            .filter(|item| !loaded.contains(&source_of(ItemKind::of(item))))
            .cloned()
            .collect()
    }

    pub fn reconcile(&mut self, pinned: &[Item], remote: &[Item], loaded: &[LibrarySource]) {
        let known: HashSet<&str> = pinned.iter().chain(remote).map(|item| item.browse_id.as_str()).collect();
        let stale: Vec<String> = self
            .sidebar_ids()
            .into_iter()
            .filter(|id| !known.contains(id))
            .filter(|id| {
                self.items
                    .get(*id)
                    .is_none_or(|item| loaded.contains(&source_of(ItemKind::of(item))))
            })
            .map(str::to_owned)
            .collect();
        for id in &stale {
            self.unpin_item(id);
        }
        let catalog = Catalog::with_cache(pinned, remote, &self.cached_items(loaded));
        self.refresh_metadata(&catalog);
        self.forget_missing(&catalog);
    }

    pub fn refresh_metadata(&mut self, source: &[Item]) {
        let lookup: HashMap<&str, &Item> = source.iter().map(|item| (item.browse_id.as_str(), item)).collect();
        let referenced: HashSet<String> = self
            .entries
            .iter()
            .filter_map(|entry| match entry {
                SidebarEntry::Item(id) => Some(id.clone()),
                SidebarEntry::Folder(_) => None,
            })
            .chain(self.folders.iter().flat_map(|folder| folder.items.iter().cloned()))
            .collect();
        for id in &referenced {
            if let Some(fresh) = lookup.get(id.as_str()) {
                self.items.insert(id.clone(), (*fresh).clone());
            }
        }
        self.items.retain(|id, _| referenced.contains(id));
    }

    pub fn forget_missing(&mut self, catalog: &Catalog) {
        let referenced: HashSet<String> = self.sidebar_ids().into_iter().map(str::to_owned).collect();
        let known = |id: &str| catalog.get(id).is_some() || referenced.contains(id);
        self.tags.retain(|id, _| known(id));
        self.hidden.retain(|id| catalog.get(id).is_some());
    }

    pub fn is_in_sidebar(&self, browse_id: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, SidebarEntry::Item(id) if id == browse_id))
            || self.folders.iter().any(|folder| folder.items.iter().any(|id| id == browse_id))
    }

    pub fn folder(&self, folder_id: &str) -> Option<&Folder> {
        self.folders.iter().find(|folder| folder.id == folder_id)
    }

    pub fn folder_of(&self, browse_id: &str) -> Option<&str> {
        self.folders
            .iter()
            .find(|folder| folder.items.iter().any(|id| id == browse_id))
            .map(|folder| folder.id.as_str())
    }

    pub fn pin_item(&mut self, item: &Item) {
        if item.browse_id.is_empty() || item.is_song() {
            return;
        }
        self.items.insert(item.browse_id.clone(), item.clone());
        if !self.is_in_sidebar(&item.browse_id) {
            self.entries.push(SidebarEntry::Item(item.browse_id.clone()));
        }
    }

    pub fn unpin_item(&mut self, browse_id: &str) {
        self.entries
            .retain(|entry| !matches!(entry, SidebarEntry::Item(id) if id == browse_id));
        for folder in &mut self.folders {
            folder.items.retain(|id| id != browse_id);
        }
        self.items.remove(browse_id);
    }

    pub fn is_hidden(&self, browse_id: &str) -> bool {
        self.hidden.iter().any(|id| id == browse_id)
    }

    pub fn shows_in_sidebar(&self, browse_id: &str) -> bool {
        match self.mode {
            SidebarMode::Pinned => self.is_in_sidebar(browse_id),
            SidebarMode::All => !self.is_hidden(browse_id),
        }
    }

    pub fn toggle_sidebar_item(&mut self, item: &Item) -> bool {
        if self.shows_in_sidebar(&item.browse_id) {
            self.unpin_item(&item.browse_id);
            if self.mode == SidebarMode::All {
                self.hidden.push(item.browse_id.clone());
            }
            false
        } else {
            if self.mode == SidebarMode::All {
                self.hidden.retain(|id| id != &item.browse_id);
            } else {
                self.pin_item(item);
            }
            true
        }
    }

    pub fn create_folder(&mut self, name: &str) -> String {
        let label = name.trim();
        let title = if label.is_empty() {
            format!("Folder {}", self.folders.len() + 1)
        } else {
            label.to_owned()
        };
        let id = loop {
            let candidate = format!("folder-{}", self.next_folder);
            self.next_folder += 1;
            if self.folders.iter().all(|folder| folder.id != candidate) {
                break candidate;
            }
        };
        self.folders.push(Folder {
            id: id.clone(),
            name: title,
            items: Vec::new(),
            open: true,
            color: 0,
        });
        self.entries.insert(0, SidebarEntry::Folder(id.clone()));
        id
    }

    pub fn rename_folder(&mut self, folder_id: &str, name: &str) {
        let clean = name.trim();
        if clean.is_empty() {
            return;
        }
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == folder_id) {
            folder.name = clean.to_owned();
        }
    }

    pub fn set_folder_color(&mut self, folder_id: &str, color: u8) {
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == folder_id) {
            folder.color = color % FOLDER_COLORS as u8;
        }
    }

    pub fn toggle_folder_open(&mut self, folder_id: &str) {
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == folder_id) {
            folder.open ^= true;
        }
    }

    pub fn delete_folder(&mut self, folder_id: &str) {
        let Some(pos) = self.folders.iter().position(|folder| folder.id == folder_id) else {
            return;
        };
        let removed = self.folders.remove(pos);
        let insert_at = self
            .entries
            .iter()
            .position(|entry| matches!(entry, SidebarEntry::Folder(id) if id == folder_id));
        self.entries
            .retain(|entry| !matches!(entry, SidebarEntry::Folder(id) if id == folder_id));
        let mut at = insert_at.unwrap_or(self.entries.len()).min(self.entries.len());
        for item_id in removed.items {
            if !self.is_in_sidebar(&item_id) {
                self.entries.insert(at, SidebarEntry::Item(item_id));
                at += 1;
            }
        }
    }

    pub fn add_to_folder(&mut self, folder_id: &str, item: &Item) {
        if item.browse_id.is_empty() || item.is_song() {
            return;
        }
        if !self.folders.iter().any(|folder| folder.id == folder_id) {
            return;
        }
        self.items.insert(item.browse_id.clone(), item.clone());
        self.entries
            .retain(|entry| !matches!(entry, SidebarEntry::Item(id) if id == &item.browse_id));
        for folder in &mut self.folders {
            folder.items.retain(|id| id != &item.browse_id);
        }
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == folder_id) {
            folder.items.push(item.browse_id.clone());
            folder.open = true;
        }
        if !self
            .entries
            .iter()
            .any(|entry| matches!(entry, SidebarEntry::Folder(id) if id == folder_id))
        {
            self.entries.push(SidebarEntry::Folder(folder_id.to_owned()));
        }
    }

    pub fn remove_from_folder(&mut self, folder_id: &str, browse_id: &str) {
        let mut removed = false;
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == folder_id)
            && let Some(pos) = folder.items.iter().position(|id| id == browse_id)
        {
            folder.items.remove(pos);
            removed = true;
        }
        if !removed || self.is_in_sidebar(browse_id) {
            return;
        }
        let after = self
            .entries
            .iter()
            .position(|entry| matches!(entry, SidebarEntry::Folder(id) if id == folder_id))
            .map_or(self.entries.len(), |at| at + 1);
        self.entries.insert(after, SidebarEntry::Item(browse_id.to_owned()));
    }

    pub fn materialize_all(&mut self, items: &[Item]) {
        for item in items {
            if item.browse_id.is_empty() || item.is_song() {
                continue;
            }
            if !self.is_hidden(&item.browse_id) && !self.is_in_sidebar(&item.browse_id) {
                self.entries.push(SidebarEntry::Item(item.browse_id.clone()));
                self.items.insert(item.browse_id.clone(), item.clone());
            }
        }
    }

    pub fn reorder(&mut self, dragged: &SidebarSlot, target: DropTarget, catalog: &[Item]) {
        if self.mode == SidebarMode::All {
            self.materialize_all(catalog);
        }
        match (dragged, target) {
            (SidebarSlot::Folder { folder_id, .. }, DropTarget::Root(mut dest)) => {
                let Some(from) = self
                    .entries
                    .iter()
                    .position(|entry| matches!(entry, SidebarEntry::Folder(id) if id == folder_id))
                else {
                    return;
                };
                if from < dest {
                    dest = dest.saturating_sub(1);
                }
                let entry = self.entries.remove(from);
                let at = dest.min(self.entries.len());
                self.entries.insert(at, entry);
            }
            (SidebarSlot::Folder { .. }, DropTarget::IntoFolder(_) | DropTarget::InFolder(_, _)) => {}
            (SidebarSlot::RootItem { browse_id, .. }, DropTarget::Root(mut dest)) => {
                let Some(from) = self
                    .entries
                    .iter()
                    .position(|entry| matches!(entry, SidebarEntry::Item(id) if id == browse_id))
                else {
                    return;
                };
                if from < dest {
                    dest = dest.saturating_sub(1);
                }
                let entry = self.entries.remove(from);
                let at = dest.min(self.entries.len());
                self.entries.insert(at, entry);
            }
            (SidebarSlot::RootItem { browse_id, .. }, DropTarget::IntoFolder(folder_id)) => {
                if !self.folders.iter().any(|folder| folder.id == folder_id) {
                    return;
                }
                self.entries
                    .retain(|entry| !matches!(entry, SidebarEntry::Item(id) if id == browse_id));
                if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == folder_id) {
                    folder.items.retain(|id| id != browse_id);
                    folder.items.push(browse_id.clone());
                    folder.open = true;
                }
            }
            (SidebarSlot::RootItem { browse_id, .. }, DropTarget::InFolder(folder_id, dest)) => {
                if !self.folders.iter().any(|folder| folder.id == folder_id) {
                    return;
                }
                self.entries
                    .retain(|entry| !matches!(entry, SidebarEntry::Item(id) if id == browse_id));
                if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == folder_id) {
                    folder.items.retain(|id| id != browse_id);
                    let at = dest.min(folder.items.len());
                    folder.items.insert(at, browse_id.clone());
                    folder.open = true;
                }
            }
            (SidebarSlot::FolderItem { folder_id, browse_id, .. }, DropTarget::Root(dest)) => {
                let Some(source) = self.folders.iter_mut().find(|folder| folder.id == *folder_id) else {
                    return;
                };
                let before = source.items.len();
                source.items.retain(|id| id != browse_id);
                if source.items.len() == before {
                    return;
                }
                let at = dest.min(self.entries.len());
                self.entries.insert(at, SidebarEntry::Item(browse_id.clone()));
            }
            (SidebarSlot::FolderItem { folder_id, browse_id, .. }, DropTarget::IntoFolder(dest_folder)) => {
                if *folder_id == dest_folder || !self.folders.iter().any(|folder| folder.id == dest_folder) {
                    return;
                }
                if let Some(source) = self.folders.iter_mut().find(|folder| folder.id == *folder_id) {
                    source.items.retain(|id| id != browse_id);
                }
                if let Some(target) = self.folders.iter_mut().find(|folder| folder.id == dest_folder) {
                    target.items.retain(|id| id != browse_id);
                    target.items.push(browse_id.clone());
                    target.open = true;
                }
            }
            (SidebarSlot::FolderItem { folder_id, browse_id, .. }, DropTarget::InFolder(dest_folder, mut dest)) => {
                if !self.folders.iter().any(|folder| folder.id == dest_folder) {
                    return;
                }
                if *folder_id == dest_folder {
                    let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == *folder_id) else {
                        return;
                    };
                    let Some(from) = folder.items.iter().position(|id| id == browse_id) else {
                        return;
                    };
                    if from < dest {
                        dest = dest.saturating_sub(1);
                    }
                    let id = folder.items.remove(from);
                    let at = dest.min(folder.items.len());
                    folder.items.insert(at, id);
                } else {
                    if let Some(source) = self.folders.iter_mut().find(|folder| folder.id == *folder_id) {
                        source.items.retain(|id| id != browse_id);
                    }
                    if let Some(target) = self.folders.iter_mut().find(|folder| folder.id == dest_folder) {
                        target.items.retain(|id| id != browse_id);
                        let at = dest.min(target.items.len());
                        target.items.insert(at, browse_id.clone());
                        target.open = true;
                    }
                }
            }
        }
    }

    pub fn move_step(&mut self, slot: &SidebarSlot, delta: isize, catalog: &[Item]) {
        if self.mode == SidebarMode::All {
            self.materialize_all(catalog);
        }
        match slot {
            SidebarSlot::Folder { root_index, .. } | SidebarSlot::RootItem { root_index, .. } => {
                let from = *root_index;
                let Some(to) = from.checked_add_signed(delta) else { return };
                if from < self.entries.len() && to < self.entries.len() {
                    self.entries.swap(from, to);
                }
            }
            SidebarSlot::FolderItem { folder_id, item_index, .. } => {
                let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == *folder_id) else {
                    return;
                };
                let from = *item_index;
                let Some(to) = from.checked_add_signed(delta) else { return };
                if from < folder.items.len() && to < folder.items.len() {
                    folder.items.swap(from, to);
                }
            }
        }
    }

    pub fn add_tag(&mut self, browse_id: &str, raw: &str) {
        let clean = raw.trim().trim_start_matches('#').trim();
        if browse_id.is_empty() || clean.is_empty() {
            return;
        }
        let list = self.tags.entry(browse_id.to_owned()).or_default();
        if list.iter().all(|existing| !existing.eq_ignore_ascii_case(clean)) {
            list.push(clean.to_owned());
        }
    }

    pub fn remove_tag(&mut self, browse_id: &str, tag: &str) {
        let Some(list) = self.tags.get_mut(browse_id) else { return };
        list.retain(|existing| !existing.eq_ignore_ascii_case(tag));
        if list.is_empty() {
            self.tags.remove(browse_id);
        }
    }

    pub fn tags_of(&self, browse_id: &str) -> &[String] {
        self.tags.get(browse_id).map_or(&[], Vec::as_slice)
    }

    pub fn all_tags(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut tags = Vec::new();
        for list in self.tags.values() {
            for tag in list {
                if seen.insert(tag.to_ascii_lowercase()) {
                    tags.push(tag.clone());
                }
            }
        }
        tags.sort_by_key(|tag| tag.to_ascii_lowercase());
        tags
    }

    pub fn resolve<'a>(&'a self, browse_id: &str, catalog: &'a Catalog) -> Option<&'a Item> {
        catalog.get(browse_id)
    }

    pub fn sidebar_ids(&self) -> HashSet<&str> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                SidebarEntry::Item(id) => Some(id.as_str()),
                SidebarEntry::Folder(_) => None,
            })
            .chain(self.folders.iter().flat_map(|folder| folder.items.iter().map(String::as_str)))
            .collect()
    }

    pub fn matches_item(&self, item: &Item, query: &str, kind: Option<ItemKind>, tag: Option<&str>) -> bool {
        if let Some(wanted) = kind
            && ItemKind::of(item) != wanted
        {
            return false;
        }
        if let Some(wanted_tag) = tag
            && !self
                .tags_of(&item.browse_id)
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(wanted_tag))
        {
            return false;
        }
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return true;
        }
        item.title.to_ascii_lowercase().contains(&q)
            || item.subtitle.to_ascii_lowercase().contains(&q)
            || self
                .tags_of(&item.browse_id)
                .iter()
                .any(|existing| existing.to_ascii_lowercase().contains(&q))
    }

    pub fn visible_slots(&self, catalog: &Catalog, query: &str, kind: Option<ItemKind>) -> Vec<SidebarSlot> {
        let q = query.trim().to_ascii_lowercase();
        let filtering = !q.is_empty() || kind.is_some();
        let mut slots = Vec::new();
        let mut emitted = HashSet::new();

        for (root_index, entry) in self.entries.iter().enumerate() {
            match entry {
                SidebarEntry::Item(browse_id) => {
                    if let Some(item) = self.resolve(browse_id, catalog)
                        && self.matches_item(item, &q, kind, None)
                    {
                        emitted.insert(browse_id.as_str());
                        slots.push(SidebarSlot::RootItem {
                            browse_id: browse_id.clone(),
                            root_index,
                        });
                    }
                }
                SidebarEntry::Folder(folder_id) => {
                    let Some(folder) = self.folder(folder_id) else { continue };
                    let name_matches = !q.is_empty() && folder.name.to_ascii_lowercase().contains(&q);
                    let matching_children: Vec<(usize, &String)> = folder
                        .items
                        .iter()
                        .enumerate()
                        .filter(|(_, id)| {
                            self.resolve(id, catalog).is_some_and(|item| {
                                if name_matches {
                                    kind.is_none_or(|wanted| ItemKind::of(item) == wanted)
                                } else {
                                    self.matches_item(item, &q, kind, None)
                                }
                            })
                        })
                        .collect();
                    if filtering && !name_matches && matching_children.is_empty() {
                        continue;
                    }
                    slots.push(SidebarSlot::Folder {
                        folder_id: folder.id.clone(),
                        root_index,
                    });
                    if folder.open || (filtering && !matching_children.is_empty()) {
                        for (item_index, browse_id) in matching_children {
                            emitted.insert(browse_id.as_str());
                            slots.push(SidebarSlot::FolderItem {
                                folder_id: folder.id.clone(),
                                browse_id: browse_id.clone(),
                                item_index,
                            });
                        }
                    } else {
                        for browse_id in &folder.items {
                            emitted.insert(browse_id.as_str());
                        }
                    }
                }
            }
        }

        if self.mode == SidebarMode::All {
            let in_sidebar = self.sidebar_ids();
            let mut next_root = self.entries.len();
            for item in catalog.iter() {
                if emitted.contains(item.browse_id.as_str())
                    || in_sidebar.contains(item.browse_id.as_str())
                    || self.is_hidden(&item.browse_id)
                {
                    continue;
                }
                if self.matches_item(item, &q, kind, None) {
                    slots.push(SidebarSlot::RootItem {
                        browse_id: item.browse_id.clone(),
                        root_index: next_root,
                    });
                }
                next_root += 1;
            }
        }

        slots
    }

    pub fn sort_items(&self, items: &mut [&Item]) {
        match self.library_sort {
            LibrarySort::Default => {}
            LibrarySort::TitleAsc => items.sort_by_cached_key(|item| item.title.to_ascii_lowercase()),
            LibrarySort::TitleDesc => items.sort_by_cached_key(|item| Reverse(item.title.to_ascii_lowercase())),
            LibrarySort::PinnedFirst => {
                let pinned = self.sidebar_ids();
                items.sort_by_key(|item| !pinned.contains(item.browse_id.as_str()));
            }
        }
    }

    pub fn reset_sidebar(&mut self) {
        self.entries.clear();
        self.folders.clear();
        self.items.clear();
        self.hidden.clear();
        self.compact = false;
        self.collapsed = false;
        self.width = SIDEBAR_WIDTH_DEFAULT;
        self.show_liked = true;
        self.show_recent = true;
        self.show_filters = false;
        self.mode = SidebarMode::Pinned;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str, subtitle: &str, is_artist: bool) -> Item {
        Item {
            browse_id: id.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            is_artist,
            ..Item::default()
        }
    }

    #[test]
    fn classifies_playlists_albums_and_artists() {
        assert_eq!(ItemKind::of(&item("UC123", "Artist", "", true)), ItemKind::Artist);
        assert_eq!(ItemKind::of(&item("MPLAUC99", "Artist", "", false)), ItemKind::Artist);
        assert_eq!(ItemKind::of(&item("MPREb_abc", "Album", "Tarkan", false)), ItemKind::Album);
        assert_eq!(ItemKind::of(&item("VL123", "EP Name", "EP • 2024", false)), ItemKind::Album);
        assert_eq!(
            ItemKind::of(&item("VLPL1", "Workout", "Playlist • 20 songs", false)),
            ItemKind::Playlist
        );
    }

    #[test]
    fn migrates_existing_pinned_items_once() {
        let mut layout = SidebarLayout::default();
        let saved = vec![item("VL1", "Mix", "Playlist", false), item("MPREb_1", "LP", "Album", false)];
        layout.migrate(&saved, false);
        assert!(layout.migrated);
        assert_eq!(
            layout.entries,
            [SidebarEntry::Item("VL1".into()), SidebarEntry::Item("MPREb_1".into())]
        );
        layout.unpin_item("VL1");
        layout.migrate(&saved, false);
        assert_eq!(layout.entries, [SidebarEntry::Item("MPREb_1".into())]);
    }

    #[test]
    fn drag_and_drop_reorders_items_and_folders() {
        let mut layout = SidebarLayout::default();
        let a = item("VL1", "A", "Playlist", false);
        let b = item("VL2", "B", "Playlist", false);
        let c = item("MPREb_3", "C", "Album", false);
        let all = vec![a.clone(), b.clone(), c.clone()];
        layout.pin_item(&a);
        layout.pin_item(&b);
        layout.pin_item(&c);
        let folder = layout.create_folder("Chill");

        layout.reorder(
            &SidebarSlot::RootItem {
                browse_id: "VL2".into(),
                root_index: 2,
            },
            DropTarget::IntoFolder(folder.clone()),
            &all,
        );
        assert_eq!(layout.folder(&folder).unwrap().items, ["VL2"]);
        assert_eq!(
            layout.entries,
            [
                SidebarEntry::Folder(folder.clone()),
                SidebarEntry::Item("VL1".into()),
                SidebarEntry::Item("MPREb_3".into()),
            ]
        );

        layout.reorder(
            &SidebarSlot::RootItem {
                browse_id: "MPREb_3".into(),
                root_index: 2,
            },
            DropTarget::InFolder(folder.clone(), 0),
            &all,
        );
        assert_eq!(layout.folder(&folder).unwrap().items, ["MPREb_3", "VL2"]);

        layout.reorder(
            &SidebarSlot::FolderItem {
                folder_id: folder.clone(),
                browse_id: "VL2".into(),
                item_index: 1,
            },
            DropTarget::Root(0),
            &all,
        );
        assert_eq!(layout.folder(&folder).unwrap().items, ["MPREb_3"]);
        assert_eq!(
            layout.entries,
            [
                SidebarEntry::Item("VL2".into()),
                SidebarEntry::Folder(folder.clone()),
                SidebarEntry::Item("VL1".into()),
            ]
        );

        layout.delete_folder(&folder);
        assert_eq!(
            layout.entries,
            [
                SidebarEntry::Item("VL2".into()),
                SidebarEntry::Item("MPREb_3".into()),
                SidebarEntry::Item("VL1".into()),
            ]
        );
    }

    #[test]
    fn tags_filter_and_deduplicate_case_insensitively() {
        let mut layout = SidebarLayout::default();
        let a = item("VL1", "Late Night", "Playlist", false);
        layout.add_tag("VL1", "#Focus");
        layout.add_tag("VL1", "focus");
        layout.add_tag("VL1", "chill");
        assert_eq!(layout.tags_of("VL1"), ["Focus", "chill"]);
        assert!(layout.matches_item(&a, "foc", None, None));
        assert!(layout.matches_item(&a, "", None, Some("CHILL")));
        assert!(!layout.matches_item(&a, "", Some(ItemKind::Album), None));
        layout.remove_tag("VL1", "FOCUS");
        assert_eq!(layout.tags_of("VL1"), ["chill"]);
    }

    fn catalog_of(items: &[Item]) -> Catalog {
        Catalog::build(items, &[])
    }

    fn five_items() -> Vec<Item> {
        vec![
            item("VL1", "Alpha", "Playlist", false),
            item("VL2", "Bravo", "Playlist", false),
            item("MPREb_3", "Charlie", "Album", false),
            item("UC4", "Delta", "", true),
            item("VL5", "Echo", "Playlist", false),
        ]
    }

    fn root_ids(layout: &SidebarLayout) -> Vec<String> {
        layout
            .entries
            .iter()
            .map(|entry| match entry {
                SidebarEntry::Item(id) | SidebarEntry::Folder(id) => id.clone(),
            })
            .collect()
    }

    #[test]
    fn a_returning_user_with_no_pins_keeps_seeing_the_whole_library() {
        let mut layout = SidebarLayout::default();
        layout.migrate(&[], true);
        assert_eq!(layout.mode, SidebarMode::All);
        assert!(layout.migrated);
        let items = five_items();
        let slots = layout.visible_slots(&catalog_of(&items), "", None);
        assert_eq!(slots.len(), 5);
    }

    #[test]
    fn a_new_user_starts_with_an_empty_whitelist() {
        let mut layout = SidebarLayout::default();
        layout.migrate(&[], false);
        assert_eq!(layout.mode, SidebarMode::Pinned);
        assert!(layout.visible_slots(&catalog_of(&five_items()), "", None).is_empty());
    }

    #[test]
    fn migration_does_not_override_a_choice_made_after_it() {
        let mut layout = SidebarLayout::default();
        layout.migrate(&[], true);
        layout.mode = SidebarMode::Pinned;
        layout.migrate(&[], true);
        assert_eq!(layout.mode, SidebarMode::Pinned);
    }

    #[test]
    fn a_failed_category_keeps_its_known_items_and_a_fresh_empty_one_clears_them() {
        let previous = vec![
            item("VL1", "Old list", "Playlist", false),
            item("MPREb_2", "Old album", "Album", false),
            item("UC3", "Old artist", "", true),
        ];
        let fetched: LibraryParts = vec![
            (LibrarySource::Playlists, Some(vec![item("VL9", "New list", "Playlist", false)])),
            (LibrarySource::Albums, None),
            (LibrarySource::Artists, Some(Vec::new())),
        ];
        let merged: Vec<String> = merge_library(&previous, fetched).into_iter().map(|item| item.browse_id).collect();
        assert_eq!(merged, ["VL9", "MPREb_2"]);
    }

    #[test]
    fn merging_the_library_drops_duplicates_across_categories() {
        let fetched: LibraryParts = vec![
            (LibrarySource::Playlists, Some(vec![item("VL1", "A", "Playlist", false)])),
            (LibrarySource::Albums, Some(vec![item("VL1", "A", "Playlist", false)])),
            (LibrarySource::Artists, None),
        ];
        assert_eq!(merge_library(&[], fetched).len(), 1);
    }

    #[test]
    fn filtered_slots_carry_real_root_indices_so_a_drop_lands_in_the_real_list() {
        let items = five_items();
        let mut layout = SidebarLayout::default();
        for entry in &items {
            layout.pin_item(entry);
        }
        let catalog = catalog_of(&items);
        let slots = layout.visible_slots(&catalog, "echo", None);
        assert_eq!(slots.len(), 1);
        let SidebarSlot::RootItem { root_index, .. } = &slots[0] else {
            panic!("expected a root item")
        };
        assert_eq!(*root_index, 4);
        layout.reorder(&slots[0], DropTarget::Root(1), &catalog);
        assert_eq!(root_ids(&layout), ["VL1", "VL5", "VL2", "MPREb_3", "UC4"]);
    }

    #[test]
    fn a_kind_filter_keeps_folder_children_addressable_by_their_real_index() {
        let items = five_items();
        let mut layout = SidebarLayout::default();
        let folder = layout.create_folder("Mixed");
        for entry in &items {
            layout.add_to_folder(&folder, entry);
        }
        let catalog = catalog_of(&items);
        let slots = layout.visible_slots(&catalog, "", Some(ItemKind::Album));
        let child = slots
            .iter()
            .find(|slot| matches!(slot, SidebarSlot::FolderItem { .. }))
            .expect("the album stays visible inside its folder");
        let SidebarSlot::FolderItem { item_index, browse_id, .. } = child else {
            panic!("expected a folder item")
        };
        assert_eq!((*item_index, browse_id.as_str()), (2, "MPREb_3"));
        layout.move_step(child, -1, &catalog);
        assert_eq!(layout.folder(&folder).unwrap().items, ["VL1", "MPREb_3", "VL2", "UC4", "VL5"]);
    }

    #[test]
    fn all_mode_lists_unpinned_items_after_pinned_ones_and_reorder_materializes_them() {
        let items = five_items();
        let mut layout = SidebarLayout {
            mode: SidebarMode::All,
            ..SidebarLayout::default()
        };
        layout.pin_item(&items[3]);
        let catalog = catalog_of(&items);
        let slots = layout.visible_slots(&catalog, "", None);
        let ids: Vec<&str> = slots
            .iter()
            .map(|slot| match slot {
                SidebarSlot::RootItem { browse_id, .. } => browse_id.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(ids, ["UC4", "VL1", "VL2", "MPREb_3", "VL5"]);
        let SidebarSlot::RootItem { root_index, .. } = &slots[4] else {
            panic!("expected a root item")
        };
        assert_eq!(*root_index, 4);
        layout.reorder(&slots[4], DropTarget::Root(0), &catalog);
        assert_eq!(root_ids(&layout), ["VL5", "UC4", "VL1", "VL2", "MPREb_3"]);
    }

    #[test]
    fn slots_that_went_stale_after_a_delete_change_nothing() {
        let items = five_items();
        let mut layout = SidebarLayout::default();
        let folder = layout.create_folder("Gone");
        layout.add_to_folder(&folder, &items[0]);
        layout.pin_item(&items[1]);
        let catalog = catalog_of(&items);
        let stale = SidebarSlot::FolderItem {
            folder_id: folder.clone(),
            browse_id: "VL1".into(),
            item_index: 0,
        };
        layout.delete_folder(&folder);
        let before = layout.entries.clone();
        layout.reorder(&stale, DropTarget::Root(0), &catalog);
        layout.reorder(&stale, DropTarget::InFolder(folder.clone(), 0), &catalog);
        layout.move_step(&stale, 1, &catalog);
        layout.move_step(
            &SidebarSlot::RootItem {
                browse_id: "VL2".into(),
                root_index: 99,
            },
            -1,
            &catalog,
        );
        assert_eq!(layout.entries, before);
        assert_eq!(root_ids(&layout), ["VL1", "VL2"]);
    }

    #[test]
    fn a_folder_cannot_be_dropped_into_a_folder() {
        let mut layout = SidebarLayout::default();
        let outer = layout.create_folder("Outer");
        let inner = layout.create_folder("Inner");
        let before = layout.entries.clone();
        let slot = SidebarSlot::Folder {
            folder_id: inner.clone(),
            root_index: 0,
        };
        layout.reorder(&slot, DropTarget::IntoFolder(outer.clone()), &[]);
        layout.reorder(&slot, DropTarget::IntoFolder(inner), &[]);
        layout.reorder(&slot, DropTarget::InFolder(outer, 0), &[]);
        assert_eq!(layout.entries, before);
    }

    #[test]
    fn deleting_a_folder_keeps_its_items_in_place_and_never_duplicates_them() {
        let items = five_items();
        let mut layout = SidebarLayout::default();
        layout.pin_item(&items[0]);
        let folder = layout.create_folder("Temp");
        layout.add_to_folder(&folder, &items[1]);
        layout.add_to_folder(&folder, &items[2]);
        layout.delete_folder(&folder);
        assert_eq!(root_ids(&layout), ["VL2", "MPREb_3", "VL1"]);
        assert!(layout.folders.is_empty());
    }

    #[test]
    fn the_catalog_skips_liked_songs_and_duplicates_and_resolves_by_id() {
        let pinned = vec![item("VL1", "Mine", "Playlist", false), item("VLLM", "Liked", "", false)];
        let remote = vec![
            item("VL1", "Mine again", "Playlist", false),
            item("MPREb_2", "Album", "Album", false),
        ];
        let catalog = Catalog::build(&pinned, &remote);
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.get("VL1").unwrap().title, "Mine");
        assert!(catalog.get("VLLM").is_none());
        assert_eq!(catalog.get("MPREb_2").unwrap().title, "Album");
    }

    #[test]
    fn an_item_missing_from_the_catalog_is_hidden_but_stays_in_the_layout() {
        let gone = item("VL7", "Remembered", "Playlist", false);
        let mut layout = SidebarLayout::default();
        layout.pin_item(&gone);
        let catalog = Catalog::default();
        assert!(layout.resolve("VL7", &catalog).is_none());
        assert!(layout.visible_slots(&catalog, "", None).is_empty());
        assert_eq!(root_ids(&layout), ["VL7"]);
    }

    const EVERY_SOURCE: [LibrarySource; 3] = [LibrarySource::Playlists, LibrarySource::Albums, LibrarySource::Artists];

    fn cached_layout() -> SidebarLayout {
        let mut layout = SidebarLayout::default();
        layout.pin_item(&item("VL1", "One", "Playlist", false));
        layout.pin_item(&item("VL2", "Two", "Playlist", false));
        layout.pin_item(&item("MPREb_3", "Three", "Album", false));
        let folder = layout.create_folder("Mine");
        layout.add_to_folder(&folder, &item("VL4", "Four", "Playlist", false));
        layout
    }

    #[test]
    fn reconciling_drops_cached_items_the_library_no_longer_lists() {
        let mut layout = cached_layout();
        let remote = [item("VL1", "One", "Playlist", false), item("MPREb_3", "Three", "Album", false)];
        layout.reconcile(&[], &remote, &EVERY_SOURCE);
        assert_eq!(root_ids(&layout), ["folder-1", "VL1", "MPREb_3"]);
        assert!(layout.folders[0].items.is_empty());
        assert!(!layout.items.contains_key("VL2"));
        assert!(!layout.items.contains_key("VL4"));
    }

    #[test]
    fn reconciling_keeps_locally_pinned_items_and_refreshes_titles() {
        let mut layout = cached_layout();
        let pinned = [item("VL2", "Two", "Playlist", false)];
        let remote = [item("VL1", "Renamed", "Playlist", false)];
        layout.reconcile(&pinned, &remote, &EVERY_SOURCE);
        assert_eq!(root_ids(&layout), ["folder-1", "VL1", "VL2"]);
        assert_eq!(layout.items["VL1"].title, "Renamed");
    }

    #[test]
    fn reconciling_keeps_order_folders_and_tags_of_survivors_and_lists_new_items() {
        let mut layout = cached_layout();
        layout.mode = SidebarMode::All;
        layout.tags.insert("VL4".into(), vec!["calm".into()]);
        layout.tags.insert("VL2".into(), vec!["gone".into()]);
        let remote = [
            item("MPREb_3", "Three", "Album", false),
            item("VL4", "Four", "Playlist", false),
            item("VL1", "One", "Playlist", false),
            item("VL9", "New", "Playlist", false),
        ];
        layout.reconcile(&[], &remote, &EVERY_SOURCE);
        assert_eq!(root_ids(&layout), ["folder-1", "VL1", "MPREb_3"]);
        assert_eq!(layout.folders[0].items, ["VL4"]);
        assert!(layout.tags.contains_key("VL4"));
        assert!(!layout.tags.contains_key("VL2"));
        let catalog = Catalog::build(&[], &remote);
        let shown = layout.visible_slots(&catalog, "", None);
        assert!(
            shown
                .iter()
                .any(|slot| matches!(slot, SidebarSlot::RootItem { browse_id, .. } if browse_id == "VL9"))
        );
    }

    #[test]
    fn a_signed_out_start_hides_cached_items_but_keeps_the_layout() {
        let mut layout = cached_layout();
        layout.tags.insert("VL1".into(), vec!["calm".into()]);
        let before = root_ids(&layout);
        let catalog = Catalog::with_cache(&[], &[], &[]);
        assert!(catalog.is_empty());
        assert!(
            layout
                .visible_slots(&catalog, "", None)
                .iter()
                .all(|slot| matches!(slot, SidebarSlot::Folder { .. }))
        );
        assert_eq!(root_ids(&layout), before);
        assert_eq!(layout.folders[0].items, ["VL4"]);
        assert!(layout.tags.contains_key("VL1"));
    }

    #[test]
    fn signing_back_in_restores_order_folders_and_tags() {
        let mut layout = cached_layout();
        layout.tags.insert("VL1".into(), vec!["calm".into()]);
        let before = root_ids(&layout);
        let remote = [
            item("VL1", "One", "Playlist", false),
            item("VL2", "Two", "Playlist", false),
            item("MPREb_3", "Three", "Album", false),
            item("VL4", "Four", "Playlist", false),
        ];
        let loading = Catalog::with_cache(&[], &[], &layout.cached_items(&[]));
        assert_eq!(loading.len(), 4);
        layout.reconcile(&[], &remote, &EVERY_SOURCE);
        assert_eq!(root_ids(&layout), before);
        assert_eq!(layout.folders[0].items, ["VL4"]);
        assert!(layout.tags.contains_key("VL1"));
        let catalog = Catalog::build(&[], &remote);
        assert!(layout.visible_slots(&catalog, "", None).len() >= 4);
    }

    #[test]
    fn until_a_source_loads_its_cached_items_stay_in_the_catalog() {
        let mut layout = cached_layout();
        layout.reconcile(&[], &[], &[LibrarySource::Albums]);
        assert_eq!(root_ids(&layout), ["folder-1", "VL1", "VL2"]);
        let cached = layout.cached_items(&[LibrarySource::Albums]);
        assert_eq!(cached.len(), 3);
        assert_eq!(Catalog::with_cache(&[], &[], &cached).len(), 3);
    }

    #[test]
    fn refreshing_metadata_updates_known_items_and_forgets_unreferenced_ones() {
        let mut layout = SidebarLayout::default();
        layout.pin_item(&item("VL1", "Old title", "Playlist", false));
        layout.items.insert("VL2".into(), item("VL2", "Orphan", "Playlist", false));
        layout.refresh_metadata(&[item("VL1", "New title", "Playlist", false)]);
        assert_eq!(layout.items["VL1"].title, "New title");
        assert!(!layout.items.contains_key("VL2"));
    }

    #[test]
    fn sorting_by_title_ignores_case_and_pinned_first_is_stable() {
        let items = [
            item("VL1", "bravo", "Playlist", false),
            item("VL2", "Alpha", "Playlist", false),
            item("VL3", "charlie", "Playlist", false),
            item("VL4", "Delta", "Playlist", false),
        ];
        let mut layout = SidebarLayout::default();
        let titles = |sorted: &[&Item]| sorted.iter().map(|item| item.title.clone()).collect::<Vec<_>>();
        let mut view: Vec<&Item> = items.iter().collect();
        layout.library_sort = LibrarySort::TitleAsc;
        layout.sort_items(&mut view);
        assert_eq!(titles(&view), ["Alpha", "bravo", "charlie", "Delta"]);
        layout.library_sort = LibrarySort::TitleDesc;
        layout.sort_items(&mut view);
        assert_eq!(titles(&view), ["Delta", "charlie", "bravo", "Alpha"]);
        layout.pin_item(&items[2]);
        layout.pin_item(&items[0]);
        layout.library_sort = LibrarySort::PinnedFirst;
        let mut view: Vec<&Item> = items.iter().collect();
        layout.sort_items(&mut view);
        assert_eq!(titles(&view), ["bravo", "charlie", "Alpha", "Delta"]);
    }

    #[test]
    fn a_saved_layout_round_trips_and_an_older_one_loads_with_defaults() {
        let items = five_items();
        let mut layout = SidebarLayout::default();
        layout.pin_item(&items[0]);
        let folder = layout.create_folder("Chill");
        layout.add_to_folder(&folder, &items[1]);
        layout.add_tag("VL1", "focus");
        layout.mode = SidebarMode::All;
        layout.library_sort = LibrarySort::TitleDesc;
        let text = serde_json::to_string(&layout).unwrap();
        let back: SidebarLayout = serde_json::from_str(&text).unwrap();
        assert_eq!(back, layout);
        let older: SidebarLayout = serde_json::from_str(r#"{"mode":"All","entries":[{"Item":"VL1"}]}"#).unwrap();
        assert_eq!(older.mode, SidebarMode::All);
        assert_eq!(root_ids(&older), ["VL1"]);
        assert!(older.show_liked && older.width == SIDEBAR_WIDTH_DEFAULT);
    }

    #[test]
    fn unknown_values_fall_back_for_that_field_only() {
        let text = r#"{"mode":"Sideways","library_sort":"Shuffle","library_view":"Mosaic","width":300.0,
            "entries":[{"Item":"VL1"},{"Divider":"x"},{"Folder":"f"}]}"#;
        let layout: SidebarLayout = serde_json::from_str(text).unwrap();
        assert_eq!(layout.mode, SidebarMode::Pinned);
        assert_eq!(layout.library_sort, LibrarySort::Default);
        assert_eq!(layout.library_view, LibraryView::Grid);
        assert_eq!(layout.width, 300.0);
        assert_eq!(root_ids(&layout), ["VL1", "f"]);
    }

    #[test]
    fn hiding_in_all_mode_really_removes_the_item_until_it_is_added_back() {
        let items = five_items();
        let catalog = catalog_of(&items);
        let mut layout = SidebarLayout {
            mode: SidebarMode::All,
            ..SidebarLayout::default()
        };
        layout.pin_item(&items[1]);
        assert!(layout.shows_in_sidebar("VL2"));
        assert!(!layout.toggle_sidebar_item(&items[1]));
        assert!(!layout.shows_in_sidebar("VL2"));
        let ids = |layout: &SidebarLayout| -> Vec<String> {
            layout
                .visible_slots(&catalog, "", None)
                .into_iter()
                .filter_map(|slot| match slot {
                    SidebarSlot::RootItem { browse_id, .. } => Some(browse_id),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(ids(&layout), ["VL1", "MPREb_3", "UC4", "VL5"]);
        layout.materialize_all(&catalog);
        assert!(!layout.is_in_sidebar("VL2"));
        assert!(layout.toggle_sidebar_item(&items[1]));
        assert!(ids(&layout).contains(&"VL2".to_owned()));
        layout.toggle_sidebar_item(&items[1]);
        layout.reset_sidebar();
        assert!(layout.hidden.is_empty());
    }

    #[test]
    fn pinning_in_pinned_mode_still_adds_and_removes_entries() {
        let items = five_items();
        let mut layout = SidebarLayout::default();
        assert!(layout.toggle_sidebar_item(&items[0]));
        assert!(layout.shows_in_sidebar("VL1"));
        assert!(!layout.toggle_sidebar_item(&items[0]));
        assert!(!layout.shows_in_sidebar("VL1") && layout.hidden.is_empty());
    }

    #[test]
    fn tags_of_removed_items_are_forgotten_but_kept_for_a_category_that_failed_to_load() {
        let previous = vec![item("VL1", "List", "Playlist", false), item("MPREb_2", "Album", "Album", false)];
        let mut layout = SidebarLayout::default();
        layout.add_tag("VL1", "focus");
        layout.add_tag("MPREb_2", "chill");
        layout.add_tag("VL9", "gone");
        layout.hidden.push("VL9".into());
        let fetched: LibraryParts = vec![
            (LibrarySource::Playlists, Some(vec![item("VL1", "List", "Playlist", false)])),
            (LibrarySource::Albums, None),
            (LibrarySource::Artists, Some(Vec::new())),
        ];
        let merged = merge_library(&previous, fetched);
        layout.forget_missing(&Catalog::build(&[], &merged));
        assert_eq!(layout.tags_of("VL1"), ["focus"]);
        assert_eq!(layout.tags_of("MPREb_2"), ["chill"]);
        assert!(layout.tags_of("VL9").is_empty());
        assert!(layout.hidden.is_empty());
    }

    #[test]
    fn tags_of_items_the_sidebar_still_references_survive_a_library_refresh() {
        let mut layout = SidebarLayout::default();
        layout.pin_item(&item("VL7", "Kept", "Playlist", false));
        layout.add_tag("VL7", "keep");
        layout.forget_missing(&Catalog::default());
        assert_eq!(layout.tags_of("VL7"), ["keep"]);
    }

    #[test]
    fn clamps_sidebar_width_safely() {
        assert_eq!(usable_sidebar_width(f32::NAN), SIDEBAR_WIDTH_DEFAULT);
        assert_eq!(usable_sidebar_width(50.0), SIDEBAR_WIDTH_MIN);
        assert_eq!(usable_sidebar_width(900.0), SIDEBAR_WIDTH_MAX);
        assert_eq!(usable_sidebar_width(280.0), 280.0);
    }
}
