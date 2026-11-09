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
//! A dungeon also has a [`DungeonLocation`]: where it goes on the world map
//! and how it appears there, so it can be added to the game as a new location.
//!
//! The file is JSON. Rooms keep their id in the file so dungeons can refer to them.

use std::path::Path;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::arena2::{region_name, DungeonDefinition, LocationType, MapPixel, TextureKey, WorldMap};
use crate::encounters::DungeonType;
use crate::music::DungeonSong;
use crate::scene::{
    FlatData, LightData, ModelData, ObjectAction, ObjectId, Scene, SceneObject, SceneObjectKind,
    ADDED_OBJECTS_GROUP,
};

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
    /// Where the dungeon goes in the world when it is added to the game.
    pub location: DungeonLocation,
}

/// Where a project dungeon goes in the world, as a new location of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct DungeonLocation {
    /// Its spot on the world map, or `None` until the user places it. The
    /// dungeon belongs to the region of that spot.
    pub map_pixel: Option<MapPixel>,
    /// Its icon on the travel map (large dungeon, graveyard, …).
    pub location_type: LocationType,
    /// Whether the travel map shows it from the start of the game, before the
    /// player hears of it.
    pub discovered: bool,
    /// The Daggerfall dungeon whose above-ground part (the ruin or building
    /// around the way down) this dungeon uses, or `None` if not chosen yet.
    pub entrance: Option<DungeonReference>,
    /// The Daggerfall dungeon this one takes the place of, at its spot on the
    /// map; `None` for a new location of its own.
    pub replaces: Option<DungeonReference>,
    /// The song played inside; `None` leaves the choice to the game (for a
    /// replacement, the song the replaced dungeon has).
    pub music: Option<DungeonSong>,
    /// Whether the dungeon plays its own music file ([`Self::custom_music_file`])
    /// instead of a game song. Only a `.dfmod` can do that (it carries a
    /// script that plays it); loose files fall back to [`Self::music`].
    pub custom_music: bool,
    /// The music file (`.ogg` or `.mp3`) the dungeon plays with [`Self::custom_music`].
    pub custom_music_file: String,
}

/// Extensions of the music files a dungeon can play as custom music.
pub const CUSTOM_MUSIC_EXTENSIONS: [&str; 2] = ["ogg", "mp3"];

impl Default for DungeonLocation {
    fn default() -> Self {
        Self {
            map_pixel: None,
            location_type: LocationType::MediumDungeon,
            discovered: false,
            entrance: None,
            replaces: None,
            music: None,
            custom_music: false,
            custom_music_file: String::new(),
        }
    }
}

/// Names one of Daggerfall's own dungeons. (Several regions have dungeons of the same name.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DungeonReference {
    pub region_index: usize,
    pub name: String,
}

impl DungeonReference {
    /// Refers to `definition`.
    pub fn to(definition: &DungeonDefinition) -> Self {
        Self { region_index: definition.region_index, name: definition.name.clone() }
    }

    /// Whether this refers to `definition`.
    pub fn is(&self, definition: &DungeonDefinition) -> bool {
        self.region_index == definition.region_index && self.name == definition.name
    }

    /// The name and region, for example "Privateer's Hold (Daggerfall)".
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, region_name(self.region_index))
    }
}

impl DungeonLocation {
    /// The location of a copy of `definition`: the same kind of location with
    /// the same entrance, not yet placed (the original's spot is taken).
    pub fn copied_from(definition: &DungeonDefinition) -> Self {
        Self {
            map_pixel: None,
            location_type: definition
                .location_type
                .filter(|location_type| LocationType::DUNGEON_TYPES.contains(location_type))
                .unwrap_or(LocationType::MediumDungeon),
            discovered: false,
            entrance: Some(DungeonReference::to(definition)),
            replaces: None,
            music: None,
            custom_music: false,
            custom_music_file: String::new(),
        }
    }

    /// What stops the dungeon from being added to the game at this location.
    ///
    /// `other_dungeons` are the project's other placed dungeons, by name. The
    /// spot must be on the map, in a region (not the sea) and free: Daggerfall
    /// Unity finds locations by their map pixel, so two cannot share one. A
    /// dungeon that replaces a Daggerfall dungeon takes that dungeon's spot.
    pub fn problems(&self, world: Option<&WorldMap>, other_dungeons: &[(&str, MapPixel)]) -> Vec<String> {
        let mut problems = Vec::new();
        match self.map_pixel {
            None => problems.push("It is not placed on the world map yet".to_string()),
            Some(pixel) if !pixel.is_on_map() => problems.push(format!("{pixel} is off the world map")),
            Some(pixel) => {
                if let Some(world) = world {
                    if world.has_regions() && world.region_at(pixel).is_none() && self.replaces.is_none() {
                        problems.push("Its spot is in the sea, outside every region".to_string());
                    }
                    let replaced = |location: &&crate::arena2::WorldLocation| {
                        self.replaces.as_ref().is_some_and(|replaced| {
                            replaced.region_index == location.region_index && replaced.name == location.name
                        })
                    };
                    for location in world.locations_at(pixel).filter(|location| !replaced(location)) {
                        problems.push(format!(
                            "Daggerfall's {} ({}) is already at its spot",
                            location.name,
                            region_name(location.region_index)
                        ));
                    }
                }
                for (name, _) in other_dungeons.iter().filter(|(_, other)| *other == pixel) {
                    problems.push(format!("{name}, in this project, is already at its spot"));
                }
            }
        }
        if self.entrance.is_none() {
            problems.push("No entrance is chosen".to_string());
        }
        problems
    }
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
    /// How the project is described when exported as a Daggerfall Unity mod.
    pub mod_details: ModDetails,
    next_id: ItemId,
}

/// What a Daggerfall Unity mod made from the project says about itself, in
/// its manifest. Saved with the project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModDetails {
    /// Name shown in Daggerfall Unity's mod list; the project's name when empty.
    pub title: String,
    pub version: String,
    pub author: String,
    /// Where players can reach the author (a web page, an e-mail address, …).
    pub contact: String,
    pub description: String,
    /// Identifies the mod to Daggerfall Unity; made once, on the first export.
    pub guid: String,
}

impl ModDetails {
    /// Whether nothing has been filled in.
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
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

    /// Whether a room or dungeon of the project is called `name` (ignoring case
    /// and surrounding spaces).
    pub fn has_item_named(&self, name: &str) -> bool {
        let mut names = Vec::new();
        collect_item_names(&self.entries, &mut names);
        let name = name.trim();
        names.iter().any(|existing| existing.trim().eq_ignore_ascii_case(name))
    }

    /// Adds an empty room called `name` at the top level and returns its id.
    pub fn add_empty_room(&mut self, name: &str) -> ItemId {
        let id = self.new_id();
        let room = ProjectRoom { id, name: name.trim().to_string(), based_on: None, objects: Vec::new() };
        self.add(ProjectEntry::Room(room), None);
        id
    }

    /// Adds an empty dungeon (no cells, not placed) called `name` at the top level and returns its id.
    pub fn add_empty_dungeon(&mut self, name: &str) -> ItemId {
        let id = self.new_id();
        let dungeon = ProjectDungeon {
            id,
            name: name.trim().to_string(),
            region_name: String::new(),
            dungeon_type: None,
            based_on: None,
            rooms: Vec::new(),
            location: DungeonLocation::default(),
        };
        self.add(ProjectEntry::Dungeon(dungeon), None);
        id
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
                // Two dungeons cannot share a spot on the map.
                dungeon.location.map_pixel = None;
                dungeon.location.replaces = None;
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

    /// Every project dungeon, in tree order.
    pub fn dungeons(&self) -> Vec<&ProjectDungeon> {
        let mut dungeons = Vec::new();
        collect_dungeons(&self.entries, &mut dungeons);
        dungeons
    }

    /// The name and map spot of every placed project dungeon except `except`.
    pub fn placed_dungeons(&self, except: Option<ItemId>) -> Vec<(&str, MapPixel)> {
        self.dungeons()
            .into_iter()
            .filter(|dungeon| Some(dungeon.id) != except)
            .filter_map(|dungeon| Some((dungeon.name.as_str(), dungeon.location.map_pixel?)))
            .collect()
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

    /// The entries saving the room or dungeon `id` saves: the item itself and,
    /// for a dungeon, the project rooms it uses.
    pub fn item_and_its_rooms(&self, id: ItemId) -> Vec<ItemId> {
        let mut ids = vec![id];
        if let Some(dungeon) = self.dungeon(id) {
            for room_id in dungeon.rooms.iter().filter_map(|placement| placement.room) {
                if !ids.contains(&room_id) {
                    ids.push(room_id);
                }
            }
        }
        ids
    }

    /// Whether the room or dungeon `id` (with, for a dungeon, the rooms it
    /// uses) differs from its copy in `saved`, or is not in `saved` at all.
    pub fn item_differs_from(&self, saved: &Project, id: ItemId) -> bool {
        self.item_and_its_rooms(id).into_iter().any(|id| self.find(id) != saved.find(id))
    }

    /// Whether the project's contents (everything saved in its file) are those of `other`.
    pub fn same_contents(&self, other: &Project) -> bool {
        self.entries == other.entries && self.mod_details == other.mod_details
    }

    /// `self` (the project as saved) with the room or dungeon `id` of `current`
    /// saved into it, and for a dungeon the rooms it uses. Everything else stays
    /// as it was saved. An entry new since then goes into its folder, if that
    /// folder was saved, or at the top level.
    pub fn with_item_from(&self, current: &Project, id: ItemId) -> Project {
        let mut saved = self.clone();
        saved.next_id = saved.next_id.max(current.next_id);
        for id in current.item_and_its_rooms(id) {
            let Some(entry) = current.find(id) else { continue };
            if let Some(existing) = saved.find_mut(id) {
                *existing = entry.clone();
                continue;
            }
            let parent = current
                .parent_of(id)
                .flatten()
                .filter(|parent| saved.find(*parent).is_some_and(ProjectEntry::is_folder));
            saved.add(entry.clone(), parent);
        }
        saved
    }

    /// Writes the project as the text of a `.ddm` file.
    pub fn to_file_text(&self) -> String {
        let file = ProjectFile {
            format: FORMAT_NAME.to_string(),
            version: FORMAT_VERSION,
            entries: self.entries.iter().map(EntryFile::from_entry).collect(),
            mod_details: (!self.mod_details.is_empty()).then(|| self.mod_details.clone()),
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
        project.mod_details = file.mod_details.unwrap_or_default();
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
    /// Absent until the project is described for export.
    #[serde(default, rename = "mod", skip_serializing_if = "Option::is_none")]
    mod_details: Option<ModDetails>,
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
        /// Absent in files saved before dungeons had a location.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<LocationFile>,
    },
}

#[derive(Serialize, Deserialize)]
struct LocationFile {
    /// Map pixel `[x, y]`; absent until the dungeon is placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    map_pixel: Option<[i32; 2]>,
    /// The Daggerfall dungeon it replaces; absent for a new location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replaces: Option<DungeonReferenceFile>,
    /// The song played inside, by Daggerfall Unity's name (for example
    /// "dungeon5"); absent when the game chooses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    music: Option<String>,
    /// Whether the dungeon plays its own music file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    custom_music: bool,
    /// The music file it plays; absent when none was chosen.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    custom_music_file: String,
    /// Location type name, for example "Large dungeon".
    location_type: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    discovered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entrance: Option<DungeonReferenceFile>,
}

#[derive(Serialize, Deserialize)]
struct DungeonReferenceFile {
    region: usize,
    name: String,
}

impl LocationFile {
    fn from_location(location: &DungeonLocation) -> Self {
        Self {
            map_pixel: location.map_pixel.map(|pixel| [pixel.x, pixel.y]),
            location_type: location.location_type.name().to_string(),
            discovered: location.discovered,
            entrance: location.entrance.as_ref().map(|entrance| DungeonReferenceFile {
                region: entrance.region_index,
                name: entrance.name.clone(),
            }),
            replaces: location.replaces.as_ref().map(|replaced| DungeonReferenceFile {
                region: replaced.region_index,
                name: replaced.name.clone(),
            }),
            music: location.music.map(|song| song.name.to_string()),
            custom_music: location.custom_music,
            custom_music_file: location.custom_music_file.clone(),
        }
    }

    fn into_location(self) -> DungeonLocation {
        DungeonLocation {
            map_pixel: self.map_pixel.map(|[x, y]| MapPixel::new(x, y)),
            location_type: LocationType::from_name(&self.location_type)
                .unwrap_or(DungeonLocation::default().location_type),
            discovered: self.discovered,
            entrance: self
                .entrance
                .map(|entrance| DungeonReference { region_index: entrance.region, name: entrance.name }),
            replaces: self
                .replaces
                .map(|replaced| DungeonReference { region_index: replaced.region, name: replaced.name }),
            music: self.music.as_deref().and_then(DungeonSong::from_name),
            custom_music: self.custom_music,
            custom_music_file: self.custom_music_file,
        }
    }
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
    /// Identifies the object within its cell, for action links.
    #[serde(default)]
    id: ObjectId,
    name: String,
    /// The RDB object list the object came from; absent for objects the user placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<usize>,
    kind: ObjectKindFile,
    position: [f32; 3],
    rotation: [f32; 3],
    #[serde(default = "default_visible", skip_serializing_if = "Clone::clone")]
    visible: bool,
    /// Id of the object this one's action triggers next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next: Option<ObjectId>,
}

#[derive(Serialize, Deserialize)]
struct ActionFile {
    #[serde(rename = "type")]
    action_type: u8,
    axis: u8,
    duration: u16,
    magnitude: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ObjectKindFile {
    Model {
        model_id: u32,
        /// Written by earlier versions instead of `action`.
        #[serde(default, skip_serializing)]
        has_action: bool,
        #[serde(default, skip_serializing_if = "is_zero_u32")]
        trigger_and_lock: u32,
        #[serde(default, skip_serializing_if = "is_zero_u8")]
        sound: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<ActionFile>,
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
        #[serde(default, skip_serializing_if = "is_zero_u32")]
        unknown_1: u32,
        #[serde(default, skip_serializing_if = "is_zero_u32")]
        unknown_2: u32,
    },
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
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
                location: Some(LocationFile::from_location(&dungeon.location)),
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
                objects: {
                    let mut objects: Vec<SceneObject> =
                        objects.into_iter().map(ObjectFile::into_object).collect();
                    Scene::ensure_object_ids(&mut objects);
                    objects
                },
            }),
            Self::Dungeon { name, region, dungeon_type, based_on, rooms, location } => {
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
                    location: location.map(LocationFile::into_location).unwrap_or_default(),
                })
            }
        }
    }
}

impl ObjectFile {
    fn from_object(object: &SceneObject) -> Self {
        let kind = match object.kind {
            SceneObjectKind::Model { model_id, data } => ObjectKindFile::Model {
                model_id,
                has_action: false,
                trigger_and_lock: data.trigger_and_lock,
                sound: data.sound,
                action: data.action.map(|action| ActionFile {
                    action_type: action.action_type,
                    axis: action.axis,
                    duration: action.duration,
                    magnitude: action.magnitude,
                }),
            },
            SceneObjectKind::Flat { texture, data } => ObjectKindFile::Flat {
                archive: texture.archive,
                record: texture.record,
                flags: data.flags,
                faction_or_mobile_id: data.faction_or_mobile_id,
                action: data.action,
            },
            SceneObjectKind::Light { radius, data } => {
                ObjectKindFile::Light { radius, unknown_1: data.unknown_1, unknown_2: data.unknown_2 }
            }
        };
        Self {
            id: object.id,
            next: object.next,
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
            ObjectKindFile::Model { model_id, has_action, trigger_and_lock, sound, action } => {
                let action = action
                    .map(|action| ObjectAction {
                        action_type: action.action_type,
                        axis: action.axis,
                        duration: action.duration,
                        magnitude: action.magnitude,
                    })
                    // Earlier versions only recorded that there was an action.
                    .or(has_action.then(ObjectAction::default));
                SceneObjectKind::Model { model_id, data: ModelData { trigger_and_lock, sound, action } }
            }
            ObjectKindFile::Flat { archive, record, flags, faction_or_mobile_id, action } => {
                SceneObjectKind::Flat {
                    texture: TextureKey::new(archive, record),
                    data: FlatData { flags, faction_or_mobile_id, action },
                }
            }
            ObjectKindFile::Light { radius, unknown_1, unknown_2 } => {
                SceneObjectKind::Light { radius, data: LightData { unknown_1, unknown_2 } }
            }
        };
        SceneObject {
            id: self.id,
            next: self.next,
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
                kind: SceneObjectKind::Model {
                    model_id: 55000,
                    data: ModelData {
                        trigger_and_lock: 0x22,
                        sound: 7,
                        action: Some(ObjectAction { action_type: 8, axis: 5, duration: 40, magnitude: 205 }),
                    },
                },
                position: Vec3::new(1.25, -0.5, 30.1),
                rotation_degrees: Vec3::new(0.0, 90.0, 0.0),
                visible: true,
                id: 1,
                next: Some(3),
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
                id: 2,
                next: None,
            },
            SceneObject {
                name: "Light".to_string(),
                group_index: 2,
                kind: SceneObjectKind::Light { radius: 7.5, data: LightData { unknown_1: 3, unknown_2: 0 } },
                position: Vec3::ZERO,
                rotation_degrees: Vec3::ZERO,
                visible: true,
                id: 3,
                next: None,
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
    fn saving_one_item_keeps_other_changes_unsaved() {
        let mut project = Project::default();
        let folder = project.add_folder("Crypts", None);
        let hall = room(&mut project, "Hall");
        let hall_id = hall.id();
        project.add(hall, Some(folder));
        let other = room(&mut project, "Other");
        let other_id = other.id();
        project.add(other, None);
        let dungeon_id = project.add_empty_dungeon("Hold");
        let saved = project.clone();

        // The dungeon gains a new room, the hall and the other room are edited.
        let new_room = room(&mut project, "New");
        let new_room_id = new_room.id();
        project.add(new_room, Some(folder));
        project.dungeon_mut(dungeon_id).unwrap().rooms.push(ProjectDungeonRoom {
            block_name: "N0000004.RDB".to_string(),
            grid_x: 0,
            grid_z: 0,
            is_starting_block: true,
            room: Some(new_room_id),
        });
        project.room_mut(hall_id).unwrap().objects.clear();
        project.room_mut(other_id).unwrap().objects.clear();
        assert!(project.item_differs_from(&saved, dungeon_id));
        assert!(project.item_differs_from(&saved, hall_id));
        assert!(!project.same_contents(&saved));

        // Saving the dungeon saves it and its new room (into its folder), nothing else.
        let after = saved.with_item_from(&project, dungeon_id);
        assert!(!project.item_differs_from(&after, dungeon_id));
        assert_eq!(after.parent_of(new_room_id), Some(Some(folder)));
        assert!(project.item_differs_from(&after, hall_id));
        assert!(project.item_differs_from(&after, other_id));

        // Saving the remaining rooms leaves nothing unsaved.
        let after = after.with_item_from(&project, hall_id).with_item_from(&project, other_id);
        assert!(project.same_contents(&after));
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
                location: DungeonLocation {
                    map_pixel: Some(MapPixel::new(204, 312)),
                    location_type: LocationType::Graveyard,
                    discovered: true,
                    entrance: Some(DungeonReference {
                        region_index: 17,
                        name: "Privateer's Hold".to_string(),
                    }),
                    replaces: Some(DungeonReference {
                        region_index: 17,
                        name: "Castle Necromoghan".to_string(),
                    }),
                    music: Some(crate::music::DUNGEON_SONGS[3]),
                    custom_music: true,
                    custom_music_file: "C:/Music/Hold.ogg".to_string(),
                },
            }),
            None,
        );

        let text = project.to_file_text();
        let loaded = Project::from_file_text(&text).unwrap();
        assert_eq!(loaded.to_file_text(), text);
        let loaded_dungeon = loaded.dungeon(loaded.entries[2].id()).unwrap();
        assert_eq!(loaded_dungeon.dungeon_type, DungeonType::from_name("Human Stronghold"));
        assert_eq!(loaded_dungeon.location.map_pixel, Some(MapPixel::new(204, 312)));
        assert_eq!(loaded_dungeon.location.location_type, LocationType::Graveyard);
        assert!(loaded_dungeon.location.discovered);
        assert_eq!(loaded_dungeon.location.music, Some(crate::music::DUNGEON_SONGS[3]));
        assert!(loaded_dungeon.location.custom_music);
        assert_eq!(loaded_dungeon.location.custom_music_file, "C:/Music/Hold.ogg");
        assert_eq!(
            loaded_dungeon.location.replaces.as_ref().map(DungeonReference::label).as_deref(),
            Some("Castle Necromoghan (Daggerfall)")
        );
        assert_eq!(
            loaded_dungeon.location.entrance.as_ref().unwrap().label(),
            "Privateer's Hold (Daggerfall)"
        );
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
                location: DungeonLocation {
                    map_pixel: Some(MapPixel::new(5, 6)),
                    ..DungeonLocation::default()
                },
            }),
            None,
        );

        // A duplicated dungeon gets its own copy of the room, and is not placed on the original's spot.
        let copy_id = project.duplicate(dungeon_id).unwrap();
        assert_eq!(project.dungeon(copy_id).unwrap().location.map_pixel, None);
        assert_eq!(project.placed_dungeons(None), [("Hold", MapPixel::new(5, 6))]);
        assert!(project.placed_dungeons(Some(dungeon_id)).is_empty());
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
    fn older_files_load_dungeons_without_a_location() {
        let text = format!(
            r#"{{"format":"{FORMAT_NAME}","version":1,"entries":[
                {{"type":"dungeon","name":"Old","region":"Daggerfall","rooms":[]}}]}}"#
        );
        let loaded = Project::from_file_text(&text).unwrap();
        assert_eq!(loaded.dungeons()[0].location, DungeonLocation::default());
    }

    #[test]
    fn location_problems() {
        let mut location = DungeonLocation::default();
        assert_eq!(
            location.problems(None, &[]),
            ["It is not placed on the world map yet", "No entrance is chosen"]
        );

        location.entrance = Some(DungeonReference { region_index: 17, name: "Privateer's Hold".to_string() });
        location.map_pixel = Some(MapPixel::new(1000, 3));
        assert_eq!(location.problems(None, &[]), ["1000, 3 is off the world map"]);

        location.map_pixel = Some(MapPixel::new(10, 3));
        assert!(location.problems(None, &[("Other", MapPixel::new(10, 4))]).is_empty());
        assert_eq!(
            location.problems(None, &[("Other", MapPixel::new(10, 3))]),
            ["Other, in this project, is already at its spot"]
        );
    }

    #[test]
    fn empty_items_and_taken_names() {
        let mut project = Project::default();
        let room_id = project.add_empty_room(" Great Hall ");
        let dungeon_id = project.add_empty_dungeon("Deep Vault");
        assert_eq!(project.room(room_id).unwrap().name, "Great Hall");
        assert!(project.room(room_id).unwrap().objects.is_empty());
        let dungeon = project.dungeon(dungeon_id).unwrap();
        assert!(dungeon.rooms.is_empty());
        assert_eq!(dungeon.location, DungeonLocation::default());
        assert!(project.has_item_named("great hall"));
        assert!(project.has_item_named("DEEP VAULT "));
        assert!(!project.has_item_named("Deep"));
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
