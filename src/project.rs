//! Project files (`.ddm`): the user's own rooms and dungeons, organised in folders.
//!
//! Rooms and dungeons from Daggerfall itself are never stored in a project;
//! they are read from the game data and cannot be edited. Editing one makes a
//! copy, which belongs to the project. A project holds a tree of
//! [`ProjectEntry`]s: rooms and dungeons, and folders the user creates to
//! organise them. Entries outside any folder are "loose".
//!
//! A dungeon is a layout of rooms. Each room in it is either a Daggerfall room,
//! read from the game data when the dungeon is opened (and read-only like any
//! other Daggerfall room), or a project room the dungeon uses. Changing a
//! Daggerfall room in a dungeon makes a project room copy of it for that
//! dungeon. Each such room belongs to one dungeon: duplicating the dungeon
//! duplicates its rooms too, so editing a room never changes another dungeon.
//!
//! The file is JSON. Rooms keep their id in the file so dungeons can refer to them.

use std::path::Path;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::arena2::{DungeonDefinition, TextureKey};
use crate::encounters::DungeonType;
use crate::scene::{FlatData, SceneObject, SceneObjectKind, ADDED_OBJECTS_GROUP};

/// File extension of project files, without the dot.
pub const PROJECT_FILE_EXTENSION: &str = "ddm";
/// Value of the `format` field that identifies a project file.
const FORMAT_NAME: &str = "daggerfall-dungeon-maker-project";
/// Version of the file layout written by this program.
const FORMAT_VERSION: u32 = 1;
/// Appended to the name of a copied room or dungeon.
const COPY_SUFFIX: &str = "-Copy";

/// Identifies an entry in the project. Room ids are saved, so dungeons can refer
/// to their rooms; other entries get new ids each time the project is loaded.
pub type ItemId = u64;

/// A room belonging to the project.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRoom {
    pub id: ItemId,
    pub name: String,
    /// The Daggerfall room this one was copied from, for example `N0000004.RDB`.
    pub based_on: Option<String>,
    pub objects: Vec<SceneObject>,
}

/// A dungeon belonging to the project.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDungeon {
    pub id: ItemId,
    pub name: String,
    /// Region shown next to the name.
    pub region_name: String,
    /// The kind of dungeon, which decides the enemies at random enemy markers.
    pub dungeon_type: Option<DungeonType>,
    /// The Daggerfall dungeon this one was copied from.
    pub based_on: Option<String>,
    pub rooms: Vec<ProjectDungeonRoom>,
}

/// One room placed in a project dungeon.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDungeonRoom {
    /// RDB block the room is made from, for example `N0000012.RDB`.
    pub block_name: String,
    /// Grid column; each step is one block width.
    pub grid_x: i32,
    /// Grid row; each step is one block width.
    pub grid_z: i32,
    /// Whether the player enters the dungeon in this room.
    pub is_starting_block: bool,
    /// The project room used here, or `None` for the Daggerfall room `block_name`.
    pub room: Option<ItemId>,
}

impl ProjectDungeonRoom {
    /// The rooms of a Daggerfall dungeon, all unchanged.
    pub fn layout_of(definition: &DungeonDefinition) -> Vec<Self> {
        definition
            .blocks
            .iter()
            .map(|block| Self {
                block_name: block.block_name.clone(),
                grid_x: block.grid_x,
                grid_z: block.grid_z,
                is_starting_block: block.is_starting_block,
                room: None,
            })
            .collect()
    }
}

/// A folder the user created to organise the project.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectFolder {
    pub id: ItemId,
    pub name: String,
    pub entries: Vec<ProjectEntry>,
}

/// An entry in the project tree.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectEntry {
    Folder(ProjectFolder),
    Room(ProjectRoom),
    Dungeon(ProjectDungeon),
}

impl ProjectEntry {
    /// The entry's id.
    pub fn id(&self) -> ItemId {
        match self {
            Self::Folder(folder) => folder.id,
            Self::Room(room) => room.id,
            Self::Dungeon(dungeon) => dungeon.id,
        }
    }

    /// The entry's name.
    pub fn name(&self) -> &str {
        match self {
            Self::Folder(folder) => &folder.name,
            Self::Room(room) => &room.name,
            Self::Dungeon(dungeon) => &dungeon.name,
        }
    }

    /// The entry's name, for renaming.
    pub fn name_mut(&mut self) -> &mut String {
        match self {
            Self::Folder(folder) => &mut folder.name,
            Self::Room(room) => &mut room.name,
            Self::Dungeon(dungeon) => &mut dungeon.name,
        }
    }

    /// Whether the entry is a folder.
    pub fn is_folder(&self) -> bool {
        matches!(self, Self::Folder(_))
    }
}

/// A folder listed for "Move to" menus.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderListing {
    pub id: ItemId,
    /// Folder names from the top level down, joined with " / ".
    pub path: String,
}

/// The open project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Project {
    /// Top-level entries: folders and loose rooms and dungeons, in display order.
    pub entries: Vec<ProjectEntry>,
    next_id: ItemId,
}

impl Project {
    /// Returns a fresh id for a new entry.
    pub fn new_id(&mut self) -> ItemId {
        self.next_id += 1;
        self.next_id
    }

    /// Whether the project has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Finds the entry with `id` anywhere in the tree.
    pub fn find(&self, id: ItemId) -> Option<&ProjectEntry> {
        find_in(&self.entries, id)
    }

    /// Finds the entry with `id` anywhere in the tree, for changing it.
    pub fn find_mut(&mut self, id: ItemId) -> Option<&mut ProjectEntry> {
        find_in_mut(&mut self.entries, id)
    }

    /// The room with `id`, if there is one.
    pub fn room(&self, id: ItemId) -> Option<&ProjectRoom> {
        match self.find(id)? {
            ProjectEntry::Room(room) => Some(room),
            _ => None,
        }
    }

    /// The room with `id`, for changing it.
    pub fn room_mut(&mut self, id: ItemId) -> Option<&mut ProjectRoom> {
        match self.find_mut(id)? {
            ProjectEntry::Room(room) => Some(room),
            _ => None,
        }
    }

    /// The dungeon with `id`, if there is one.
    pub fn dungeon(&self, id: ItemId) -> Option<&ProjectDungeon> {
        match self.find(id)? {
            ProjectEntry::Dungeon(dungeon) => Some(dungeon),
            _ => None,
        }
    }

    /// The dungeon with `id`, for changing it.
    pub fn dungeon_mut(&mut self, id: ItemId) -> Option<&mut ProjectDungeon> {
        match self.find_mut(id)? {
            ProjectEntry::Dungeon(dungeon) => Some(dungeon),
            _ => None,
        }
    }

    /// Adds `entry` at the end of the folder `parent`, or at the top level if
    /// `parent` is `None` or not a folder.
    pub fn add(&mut self, entry: ProjectEntry, parent: Option<ItemId>) {
        match parent.and_then(|parent| self.find_mut(parent)) {
            Some(ProjectEntry::Folder(folder)) => folder.entries.push(entry),
            _ => self.entries.push(entry),
        }
    }

    /// Creates an empty folder called `name` inside `parent` and returns its id.
    pub fn add_folder(&mut self, name: &str, parent: Option<ItemId>) -> ItemId {
        let id = self.new_id();
        self.add(
            ProjectEntry::Folder(ProjectFolder { id, name: name.to_string(), entries: Vec::new() }),
            parent,
        );
        id
    }

    /// Removes the entry with `id` (with everything inside it, for a folder) and returns it.
    pub fn remove(&mut self, id: ItemId) -> Option<ProjectEntry> {
        remove_from(&mut self.entries, id)
    }

    /// Removes the folder `id`, moving its contents to where the folder was.
    pub fn remove_folder_keeping_contents(&mut self, id: ItemId) -> bool {
        let Some(list) = list_containing_mut(&mut self.entries, id) else { return false };
        let Some(position) = list.iter().position(|entry| entry.id() == id) else { return false };
        let ProjectEntry::Folder(folder) = list.remove(position) else {
            unreachable!("only folders are passed here")
        };
        list.splice(position..position, folder.entries);
        true
    }

    /// Moves the entry `id` into the folder `new_parent` (or the top level).
    ///
    /// Returns `false` without changing anything if the entry does not exist,
    /// `new_parent` is not a folder, or a folder would be moved into itself.
    pub fn move_entry(&mut self, id: ItemId, new_parent: Option<ItemId>) -> bool {
        if let Some(parent) = new_parent {
            let Some(target) = self.find(parent) else { return false };
            if !target.is_folder() || contains_id(self.find(id), parent) {
                return false;
            }
        }
        match self.remove(id) {
            Some(entry) => {
                self.add(entry, new_parent);
                true
            }
            None => false,
        }
    }

    /// Moves the entry `id` to just before the entry `target`, in `target`'s folder.
    ///
    /// Returns `false` without changing anything if either entry does not exist,
    /// they are the same entry, or a folder would be moved into itself.
    pub fn move_before(&mut self, id: ItemId, target: ItemId) -> bool {
        if id == target || self.find(target).is_none() || contains_id(self.find(id), target) {
            return false;
        }
        let Some(entry) = self.remove(id) else { return false };
        let list = list_containing_mut(&mut self.entries, target).expect("the target was found above");
        let position = list.iter().position(|entry| entry.id() == target).expect("the list holds the target");
        list.insert(position, entry);
        true
    }

    /// The folder containing the entry `id`: `Some(None)` at the top level, `None` if it does not exist.
    pub fn parent_of(&self, id: ItemId) -> Option<Option<ItemId>> {
        if self.entries.iter().any(|entry| entry.id() == id) {
            return Some(None);
        }
        parent_in(&self.entries, id).map(Some)
    }

    /// The folders containing `id`, from its own folder up to the top level.
    pub fn parent_chain(&self, id: ItemId) -> Vec<ItemId> {
        let mut chain = Vec::new();
        let mut current = id;
        while let Some(Some(parent)) = self.parent_of(current) {
            chain.push(parent);
            current = parent;
        }
        chain
    }

    /// Every folder, in tree order, for "Move to" menus.
    pub fn folders(&self) -> Vec<FolderListing> {
        let mut listings = Vec::new();
        collect_folders(&self.entries, "", &mut listings);
        listings
    }

    /// Name for a copy of something called `name`: `name-Copy`, or `name-Copy 2`
    /// and so on if a room or dungeon already has that name.
    pub fn copy_name(&self, name: &str) -> String {
        let mut names = Vec::new();
        collect_item_names(&self.entries, &mut names);
        let base = format!("{name}{COPY_SUFFIX}");
        std::iter::once(base.clone())
            .chain((2..).map(|number| format!("{base} {number}")))
            .find(|candidate| !names.contains(&candidate.as_str()))
            .expect("an unused name always exists")
    }

    /// Copies the room or dungeon `id` and adds the copy just after it. Returns the copy's id.
    ///
    /// A dungeon's project rooms are copied too (at the top level), so the
    /// two dungeons can be edited separately.
    pub fn duplicate(&mut self, id: ItemId) -> Option<ItemId> {
        let mut copy = self.find(id).filter(|entry| !entry.is_folder())?.clone();
        let copy_id = self.new_id();
        *copy.name_mut() = self.copy_name(copy.name());
        match &mut copy {
            ProjectEntry::Room(room) => room.id = copy_id,
            ProjectEntry::Dungeon(dungeon) => {
                dungeon.id = copy_id;
                for placement in &mut dungeon.rooms {
                    let Some(mut room) = placement.room.and_then(|room_id| self.room(room_id)).cloned()
                    else {
                        continue;
                    };
                    room.id = self.new_id();
                    room.name = self.copy_name(&room.name);
                    placement.room = Some(room.id);
                    self.entries.push(ProjectEntry::Room(room));
                }
            }
            ProjectEntry::Folder(_) => unreachable!("folders are filtered out above"),
        }
        let list = list_containing_mut(&mut self.entries, id)?;
        let position = list.iter().position(|entry| entry.id() == id)?;
        list.insert(position + 1, copy);
        Some(copy_id)
    }

    /// Names of the dungeons that use the project room `room_id`.
    pub fn dungeons_using(&self, room_id: ItemId) -> Vec<&str> {
        let mut dungeons = Vec::new();
        collect_dungeons(&self.entries, &mut dungeons);
        dungeons
            .into_iter()
            .filter(|dungeon| dungeon.rooms.iter().any(|placement| placement.room == Some(room_id)))
            .map(|dungeon| dungeon.name.as_str())
            .collect()
    }

    /// Makes every dungeon that uses the project room `room_id` use the Daggerfall room again.
    pub fn stop_using_room(&mut self, room_id: ItemId) {
        for_each_dungeon_mut(&mut self.entries, &mut |dungeon| {
            for placement in &mut dungeon.rooms {
                if placement.room == Some(room_id) {
                    placement.room = None;
                }
            }
        });
    }

    /// Writes the project as the text of a `.ddm` file.
    pub fn to_file_text(&self) -> String {
        let file = ProjectFile {
            format: FORMAT_NAME.to_string(),
            version: FORMAT_VERSION,
            entries: self.entries.iter().map(EntryFile::from_entry).collect(),
        };
        serde_json::to_string_pretty(&file).expect("project data always serialises")
    }

    /// Reads a project from the text of a `.ddm` file.
    pub fn from_file_text(text: &str) -> Result<Self, String> {
        let file: ProjectFile = serde_json::from_str(text)
            .map_err(|error| format!("this is not a valid project file ({error})"))?;
        if file.format != FORMAT_NAME {
            return Err("this is not a Daggerfall Dungeon Maker project".to_string());
        }
        if file.version > FORMAT_VERSION {
            return Err("this project was saved by a newer version of Daggerfall Dungeon Maker".to_string());
        }
        let mut project = Project::default();
        // Rooms keep their saved ids; everything else is numbered after them.
        project.next_id = file.entries.iter().map(EntryFile::largest_room_id).max().unwrap_or(0);
        project.entries = file.entries.into_iter().map(|entry| entry.into_entry(&mut project)).collect();
        // A dungeon whose room is missing from the file falls back to the Daggerfall room.
        let mut room_ids = Vec::new();
        collect_room_ids(&project.entries, &mut room_ids);
        for_each_dungeon_mut(&mut project.entries, &mut |dungeon| {
            for placement in &mut dungeon.rooms {
                if placement.room.is_some_and(|room_id| !room_ids.contains(&room_id)) {
                    placement.room = None;
                }
            }
        });
        Ok(project)
    }

    /// Saves the project to `path`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_file_text())
            .map_err(|error| format!("Could not save '{}': {error}", path.display()))
    }

    /// Loads the project saved at `path`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("Could not open '{}': {error}", path.display()))?;
        Self::from_file_text(&text).map_err(|error| format!("Could not open '{}': {error}", path.display()))
    }
}

fn find_in(entries: &[ProjectEntry], id: ItemId) -> Option<&ProjectEntry> {
    entries.iter().find_map(|entry| match entry {
        _ if entry.id() == id => Some(entry),
        ProjectEntry::Folder(folder) => find_in(&folder.entries, id),
        _ => None,
    })
}

fn find_in_mut(entries: &mut [ProjectEntry], id: ItemId) -> Option<&mut ProjectEntry> {
    for entry in entries {
        if entry.id() == id {
            return Some(entry);
        }
        if let ProjectEntry::Folder(folder) = entry {
            if let Some(found) = find_in_mut(&mut folder.entries, id) {
                return Some(found);
            }
        }
    }
    None
}

/// The list (top level or a folder's entries) that directly contains `id`.
fn list_containing_mut(entries: &mut Vec<ProjectEntry>, id: ItemId) -> Option<&mut Vec<ProjectEntry>> {
    if entries.iter().any(|entry| entry.id() == id) {
        return Some(entries);
    }
    for entry in entries {
        if let ProjectEntry::Folder(folder) = entry {
            if let Some(list) = list_containing_mut(&mut folder.entries, id) {
                return Some(list);
            }
        }
    }
    None
}

fn remove_from(entries: &mut Vec<ProjectEntry>, id: ItemId) -> Option<ProjectEntry> {
    let list = list_containing_mut(entries, id)?;
    let position = list.iter().position(|entry| entry.id() == id)?;
    Some(list.remove(position))
}

/// Whether `entry` is `id` or contains it.
fn contains_id(entry: Option<&ProjectEntry>, id: ItemId) -> bool {
    match entry {
        Some(entry) if entry.id() == id => true,
        Some(ProjectEntry::Folder(folder)) => find_in(&folder.entries, id).is_some(),
        _ => false,
    }
}

fn parent_in(entries: &[ProjectEntry], id: ItemId) -> Option<ItemId> {
    entries.iter().find_map(|entry| match entry {
        ProjectEntry::Folder(folder) if folder.entries.iter().any(|child| child.id() == id) => {
            Some(folder.id)
        }
        ProjectEntry::Folder(folder) => parent_in(&folder.entries, id),
        _ => None,
    })
}

fn collect_folders(entries: &[ProjectEntry], prefix: &str, listings: &mut Vec<FolderListing>) {
    for entry in entries {
        if let ProjectEntry::Folder(folder) = entry {
            let path =
                if prefix.is_empty() { folder.name.clone() } else { format!("{prefix} / {}", folder.name) };
            listings.push(FolderListing { id: folder.id, path: path.clone() });
            collect_folders(&folder.entries, &path, listings);
        }
    }
}

fn collect_dungeons<'project>(
    entries: &'project [ProjectEntry],
    dungeons: &mut Vec<&'project ProjectDungeon>,
) {
    for entry in entries {
        match entry {
            ProjectEntry::Folder(folder) => collect_dungeons(&folder.entries, dungeons),
            ProjectEntry::Dungeon(dungeon) => dungeons.push(dungeon),
            ProjectEntry::Room(_) => {}
        }
    }
}

fn for_each_dungeon_mut(entries: &mut [ProjectEntry], action: &mut impl FnMut(&mut ProjectDungeon)) {
    for entry in entries {
        match entry {
            ProjectEntry::Folder(folder) => for_each_dungeon_mut(&mut folder.entries, action),
            ProjectEntry::Dungeon(dungeon) => action(dungeon),
            ProjectEntry::Room(_) => {}
        }
    }
}

fn collect_room_ids(entries: &[ProjectEntry], ids: &mut Vec<ItemId>) {
    for entry in entries {
        match entry {
            ProjectEntry::Folder(folder) => collect_room_ids(&folder.entries, ids),
            ProjectEntry::Room(room) => ids.push(room.id),
            ProjectEntry::Dungeon(_) => {}
        }
    }
}

fn collect_item_names<'project>(entries: &'project [ProjectEntry], names: &mut Vec<&'project str>) {
    for entry in entries {
        match entry {
            ProjectEntry::Folder(folder) => collect_item_names(&folder.entries, names),
            _ => names.push(entry.name()),
        }
    }
}

// ---------------------------------------------------------------------------
// File layout. Kept separate from the in-memory types so the file format stays
// stable when the editor's own types change.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    format: String,
    version: u32,
    entries: Vec<EntryFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EntryFile {
    Folder {
        name: String,
        entries: Vec<EntryFile>,
    },
    Room {
        /// Referred to by the dungeons that use the room.
        #[serde(default)]
        id: ItemId,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        based_on: Option<String>,
        objects: Vec<ObjectFile>,
    },
    Dungeon {
        name: String,
        #[serde(default)]
        region: String,
        /// Dungeon type name, for example "Orc Stronghold".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dungeon_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        based_on: Option<String>,
        rooms: Vec<DungeonRoomFile>,
    },
}

#[derive(Serialize, Deserialize)]
struct DungeonRoomFile {
    block: String,
    grid_x: i32,
    grid_z: i32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    start: bool,
    /// Id of the project room used here; absent for the Daggerfall room `block`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    room: Option<ItemId>,
}

#[derive(Serialize, Deserialize)]
struct ObjectFile {
    name: String,
    /// The RDB object list the object came from; absent for objects the user placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<usize>,
    kind: ObjectKindFile,
    position: [f32; 3],
    rotation: [f32; 3],
    #[serde(default = "default_visible", skip_serializing_if = "Clone::clone")]
    visible: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ObjectKindFile {
    Model {
        model_id: u32,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        has_action: bool,
    },
    Flat {
        archive: u16,
        record: u8,
        /// Extra flat data from the game, written only when set.
        #[serde(default, skip_serializing_if = "is_zero_u16")]
        flags: u16,
        #[serde(default, skip_serializing_if = "is_zero_u16")]
        faction_or_mobile_id: u16,
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        action: u8,
    },
    Light {
        radius: f32,
    },
}

fn default_visible() -> bool {
    true
}

fn is_zero_u16(value: &u16) -> bool {
    *value == 0
}

fn is_zero_u8(value: &u8) -> bool {
    *value == 0
}

impl EntryFile {
    fn from_entry(entry: &ProjectEntry) -> Self {
        match entry {
            ProjectEntry::Folder(folder) => Self::Folder {
                name: folder.name.clone(),
                entries: folder.entries.iter().map(Self::from_entry).collect(),
            },
            ProjectEntry::Room(room) => Self::Room {
                id: room.id,
                name: room.name.clone(),
                based_on: room.based_on.clone(),
                objects: room.objects.iter().map(ObjectFile::from_object).collect(),
            },
            ProjectEntry::Dungeon(dungeon) => Self::Dungeon {
                name: dungeon.name.clone(),
                region: dungeon.region_name.clone(),
                dungeon_type: dungeon.dungeon_type.map(|dungeon_type| dungeon_type.name().to_string()),
                based_on: dungeon.based_on.clone(),
                rooms: dungeon
                    .rooms
                    .iter()
                    .map(|room| DungeonRoomFile {
                        block: room.block_name.clone(),
                        grid_x: room.grid_x,
                        grid_z: room.grid_z,
                        start: room.is_starting_block,
                        room: room.room,
                    })
                    .collect(),
            },
        }
    }

    /// The largest room id saved in this entry, or 0.
    fn largest_room_id(&self) -> ItemId {
        match self {
            Self::Folder { entries, .. } => entries.iter().map(Self::largest_room_id).max().unwrap_or(0),
            Self::Room { id, .. } => *id,
            Self::Dungeon { .. } => 0,
        }
    }

    fn into_entry(self, project: &mut Project) -> ProjectEntry {
        match self {
            Self::Folder { name, entries } => ProjectEntry::Folder(ProjectFolder {
                id: project.new_id(),
                name,
                entries: entries.into_iter().map(|entry| entry.into_entry(project)).collect(),
            }),
            Self::Room { id, name, based_on, objects } => ProjectEntry::Room(ProjectRoom {
                // Rooms saved without an id (id 0) get a new one.
                id: if id == 0 { project.new_id() } else { id },
                name,
                based_on,
                objects: objects.into_iter().map(ObjectFile::into_object).collect(),
            }),
            Self::Dungeon { name, region, dungeon_type, based_on, rooms } => {
                ProjectEntry::Dungeon(ProjectDungeon {
                    id: project.new_id(),
                    name,
                    region_name: region,
                    dungeon_type: dungeon_type.as_deref().and_then(DungeonType::from_name),
                    based_on,
                    rooms: rooms
                        .into_iter()
                        .map(|room| ProjectDungeonRoom {
                            block_name: room.block,
                            grid_x: room.grid_x,
                            grid_z: room.grid_z,
                            is_starting_block: room.start,
                            room: room.room,
                        })
                        .collect(),
                })
            }
        }
    }
}

impl ObjectFile {
    fn from_object(object: &SceneObject) -> Self {
        let kind = match object.kind {
            SceneObjectKind::Model { model_id, has_action } => ObjectKindFile::Model { model_id, has_action },
            SceneObjectKind::Flat { texture, data } => ObjectKindFile::Flat {
                archive: texture.archive,
                record: texture.record,
                flags: data.flags,
                faction_or_mobile_id: data.faction_or_mobile_id,
                action: data.action,
            },
            SceneObjectKind::Light { radius } => ObjectKindFile::Light { radius },
        };
        Self {
            name: object.name.clone(),
            group: (object.group_index != ADDED_OBJECTS_GROUP).then_some(object.group_index),
            kind,
            position: object.position.to_array(),
            rotation: object.rotation_degrees.to_array(),
            visible: object.visible,
        }
    }

    fn into_object(self) -> SceneObject {
        let kind = match self.kind {
            ObjectKindFile::Model { model_id, has_action } => SceneObjectKind::Model { model_id, has_action },
            ObjectKindFile::Flat { archive, record, flags, faction_or_mobile_id, action } => {
                SceneObjectKind::Flat {
                    texture: TextureKey::new(archive, record),
                    data: FlatData { flags, faction_or_mobile_id, action },
                }
            }
            ObjectKindFile::Light { radius } => SceneObjectKind::Light { radius },
        };
        SceneObject {
            name: self.name,
            group_index: self.group.unwrap_or(ADDED_OBJECTS_GROUP),
            kind,
            position: Vec3::from_array(self.position),
            rotation_degrees: Vec3::from_array(self.rotation),
            visible: self.visible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_objects() -> Vec<SceneObject> {
        vec![
            SceneObject {
                name: "Model 55000".to_string(),
                group_index: 0,
                kind: SceneObjectKind::Model { model_id: 55000, has_action: true },
                position: Vec3::new(1.25, -0.5, 30.1),
                rotation_degrees: Vec3::new(0.0, 90.0, 0.0),
                visible: true,
            },
            SceneObject {
                name: "Torch".to_string(),
                group_index: ADDED_OBJECTS_GROUP,
                kind: SceneObjectKind::Flat {
                    texture: TextureKey::new(199, 16),
                    data: FlatData { flags: 2, faction_or_mobile_id: 0x3507, action: 99 },
                },
                position: Vec3::new(0.1, 0.2, 0.3),
                rotation_degrees: Vec3::ZERO,
                visible: false,
            },
            SceneObject {
                name: "Light".to_string(),
                group_index: 2,
                kind: SceneObjectKind::Light { radius: 7.5 },
                position: Vec3::ZERO,
                rotation_degrees: Vec3::ZERO,
                visible: true,
            },
        ]
    }

    fn room(project: &mut Project, name: &str) -> ProjectEntry {
        ProjectEntry::Room(ProjectRoom {
            id: project.new_id(),
            name: name.to_string(),
            based_on: Some("N0000004.RDB".to_string()),
            objects: sample_objects(),
        })
    }

    #[test]
    fn file_round_trip_keeps_everything() {
        let mut project = Project::default();
        let folder = project.add_folder("Crypts", None);
        let entry = room(&mut project, "Hall-Copy");
        project.add(entry, Some(folder));
        let dungeon_room = room(&mut project, "B0000002-Copy");
        let dungeon_room_id = dungeon_room.id();
        project.add(dungeon_room, None);
        let dungeon_id = project.new_id();
        project.add(
            ProjectEntry::Dungeon(ProjectDungeon {
                id: dungeon_id,
                name: "Privateer's Hold-Copy".to_string(),
                region_name: "Daggerfall".to_string(),
                dungeon_type: DungeonType::from_name("Human Stronghold"),
                based_on: Some("Privateer's Hold".to_string()),
                rooms: vec![
                    ProjectDungeonRoom {
                        block_name: "N0000012.RDB".to_string(),
                        grid_x: 0,
                        grid_z: -1,
                        is_starting_block: true,
                        room: None,
                    },
                    ProjectDungeonRoom {
                        block_name: "B0000002.RDB".to_string(),
                        grid_x: 1,
                        grid_z: -1,
                        is_starting_block: false,
                        room: Some(dungeon_room_id),
                    },
                ],
            }),
            None,
        );

        let text = project.to_file_text();
        let loaded = Project::from_file_text(&text).unwrap();
        assert_eq!(loaded.to_file_text(), text);
        let loaded_dungeon = loaded.dungeon(loaded.entries[2].id()).unwrap();
        assert_eq!(loaded_dungeon.dungeon_type, DungeonType::from_name("Human Stronghold"));
        let room_id = loaded_dungeon.rooms[1].room.expect("the dungeon still uses its room");
        assert_eq!(loaded.room(room_id).unwrap().name, "B0000002-Copy");
        assert_eq!(loaded.dungeons_using(room_id), ["Privateer's Hold-Copy"]);
        assert_eq!(loaded.folders().len(), 1);
        let ProjectEntry::Folder(folder) = &loaded.entries[0] else { panic!("folder expected") };
        assert_eq!(folder.entries.len(), 1);
        let ProjectEntry::Room(loaded_room) = &folder.entries[0] else { panic!("room expected") };
        assert_eq!(loaded_room.objects, sample_objects());
    }

    #[test]
    fn dungeon_rooms_belong_to_one_dungeon() {
        let mut project = Project::default();
        let entry = room(&mut project, "S0000999-Copy");
        let room_id = entry.id();
        project.add(entry, None);
        let dungeon_id = project.new_id();
        project.add(
            ProjectEntry::Dungeon(ProjectDungeon {
                id: dungeon_id,
                name: "Hold".to_string(),
                region_name: String::new(),
                dungeon_type: None,
                based_on: None,
                rooms: vec![ProjectDungeonRoom {
                    block_name: "S0000999.RDB".to_string(),
                    grid_x: 0,
                    grid_z: 0,
                    is_starting_block: true,
                    room: Some(room_id),
                }],
            }),
            None,
        );

        // A duplicated dungeon gets its own copy of the room.
        let copy_id = project.duplicate(dungeon_id).unwrap();
        let copied_room = project.dungeon(copy_id).unwrap().rooms[0].room.unwrap();
        assert_ne!(copied_room, room_id);
        assert_eq!(project.room(copied_room).unwrap().name, "S0000999-Copy-Copy");
        assert_eq!(project.dungeons_using(room_id), ["Hold"]);

        // A deleted room's dungeon goes back to the Daggerfall room; a missing room on load does too.
        project.stop_using_room(room_id);
        assert_eq!(project.dungeon(dungeon_id).unwrap().rooms[0].room, None);
        project.remove(copied_room);
        let loaded = Project::from_file_text(&project.to_file_text()).unwrap();
        let loaded_copy = loaded.entries.iter().find(|entry| entry.name() == "Hold-Copy").unwrap();
        assert_eq!(loaded.dungeon(loaded_copy.id()).unwrap().rooms[0].room, None);
    }

    #[test]
    fn rejects_other_files() {
        assert!(Project::from_file_text("not json").is_err());
        assert!(Project::from_file_text(r#"{"format":"other","version":1,"entries":[]}"#).is_err());
        let newer = format!(r#"{{"format":"{FORMAT_NAME}","version":{},"entries":[]}}"#, FORMAT_VERSION + 1);
        assert!(Project::from_file_text(&newer).is_err());
    }

    #[test]
    fn copy_names_are_unique() {
        let mut project = Project::default();
        assert_eq!(project.copy_name("N0000004"), "N0000004-Copy");
        let entry = room(&mut project, "N0000004-Copy");
        project.add(entry, None);
        assert_eq!(project.copy_name("N0000004"), "N0000004-Copy 2");
    }

    #[test]
    fn duplicate_goes_next_to_the_original() {
        let mut project = Project::default();
        let folder = project.add_folder("Crypts", None);
        let entry = room(&mut project, "Hall");
        let room_id = entry.id();
        project.add(entry, Some(folder));

        let copy_id = project.duplicate(room_id).unwrap();
        assert_eq!(project.parent_of(copy_id), Some(Some(folder)));
        assert_eq!(project.room(copy_id).unwrap().name, "Hall-Copy");
        assert!(project.duplicate(folder).is_none(), "folders are not duplicated");
    }

    #[test]
    fn moving_and_removing_folders() {
        let mut project = Project::default();
        let outer = project.add_folder("Outer", None);
        let inner = project.add_folder("Inner", Some(outer));
        let entry = room(&mut project, "Hall");
        let room_id = entry.id();
        project.add(entry, Some(inner));

        assert!(!project.move_entry(outer, Some(inner)), "a folder cannot go inside itself");
        assert!(!project.move_entry(room_id, Some(room_id)), "only folders can hold entries");
        assert_eq!(
            project.folders().iter().map(|folder| folder.path.as_str()).collect::<Vec<_>>(),
            ["Outer", "Outer / Inner"]
        );

        let other = room(&mut project, "Cellar");
        let other_id = other.id();
        project.add(other, None);
        assert!(!project.move_before(outer, room_id), "a folder cannot go before its own contents");
        assert!(project.move_before(other_id, room_id));
        assert_eq!(project.parent_of(other_id), Some(Some(inner)));
        let ProjectEntry::Folder(inner_folder) = project.find(inner).unwrap() else {
            panic!("folder expected")
        };
        assert_eq!(
            inner_folder.entries.iter().map(ProjectEntry::id).collect::<Vec<_>>(),
            [other_id, room_id]
        );
        assert!(project.move_entry(other_id, None));

        assert!(project.remove_folder_keeping_contents(inner));
        assert_eq!(project.parent_of(room_id), Some(Some(outer)));
        assert!(project.move_entry(room_id, None));
        assert_eq!(project.parent_of(room_id), Some(None));
        assert!(project.remove(outer).is_some());
        // Hall and Cellar are left at the top level.
        assert_eq!(project.entries.len(), 2);
    }
}
