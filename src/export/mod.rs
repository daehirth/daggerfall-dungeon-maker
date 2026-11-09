//! Exporting the project as a Daggerfall Unity mod.
//!
//! Daggerfall Unity reads new locations and dungeon blocks from JSON files
//! ("world data"), either loose in `StreamingAssets/WorldData` or packed in a
//! `.dfmod`. An export writes, for every placed project dungeon:
//!
//! * `WorldData/locationnew-<name>-<region>.json`, the new location: where it
//!   is, its entrance above ground and its grid of dungeon blocks;
//! * `WorldData/<BLOCK>.RDB.json` for each project cell it uses, under a new
//!   block name (Daggerfall's own cells are used by their own names).
//!
//! A dungeon that replaces one of Daggerfall's is written instead as
//! `WorldData/location-<region>-<index>.json`, the file Daggerfall Unity reads
//! in place of that location. It keeps what identifies the location (its
//! index, ids, map position and name) so quests and saved games still find it.
//!
//! A dungeon with its own music (custom music) also gets its music file in
//! `Music/`, and the mod a C# script that plays it there ([`music_script`]).
//! Only a `.dfmod` runs the script: with the files used loose, those dungeons
//! play the game's music instead.
//!
//! It also writes the mod's manifest, `<name>.dfmod.json`. The files are
//! laid out as [`unity`] builds them into a `.dfmod`. The JSON follows the
//! layout of Daggerfall Unity's `DFLocation` and `DFBlock` structures (see
//! [`json`]).

pub mod json;
pub mod music_script;
pub mod unity;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::arena2::{
    region_name, world_to_native, Arena2Library, DungeonDefinition, MapPixel, WorldMap,
    NATIVE_ROTATION_UNITS_PER_DEGREE, REGION_NAMES, WORLD_SCALE,
};
use crate::project::{ItemId, ModDetails, Project, ProjectDungeon, ProjectRoom};
use crate::scene::{Scene, SceneObject, SceneObjectKind, ADDED_OBJECTS_GROUP};

/// Folder inside the mod that holds the world data files. Daggerfall Unity
/// looks for new locations in a folder of exactly this name.
pub const WORLD_DATA_FOLDER: &str = "WorldData";
/// Where the mod's files go in the Unity project that builds it; the
/// manifest lists every file by its path there.
pub const MOD_ASSET_ROOT: &str = "Assets/DungeonMaker";
/// Version of Daggerfall Unity the mod is made for, written in the manifest.
pub const DAGGERFALL_UNITY_VERSION: &str = "1.1.1";
/// Extension of a mod's manifest.
pub const MANIFEST_EXTENSION: &str = ".dfmod.json";

/// Model tag used where Daggerfall's cells never use a model.
const UNKNOWN_MODEL_TAG: &str = "XXX";
/// Most models one RDB block can list.
const MAX_MODELS_PER_BLOCK: usize = 750;
/// First number given to new block names (`N9000001.RDB` and so on): far
/// above Daggerfall's own, so they cannot be mistaken for them.
const FIRST_NEW_BLOCK_NUMBER: u64 = 9_000_000;
/// How many new block numbers there are.
const NEW_BLOCK_NUMBERS: u64 = 1_000_000;
/// Block letters (types) a new block can keep from the cell it was copied from.
const BLOCK_LETTERS: [char; 6] = ['N', 'W', 'L', 'S', 'B', 'M'];
/// Daggerfall Unity identifies saved objects (opened doors, killed enemies)
/// by their block's position plus their own. Daggerfall's blocks sit below
/// 30 million; new blocks are spaced from here so none overlap.
const NEW_BLOCK_POSITION_BASE: i64 = 10_000_000_000;
const NEW_BLOCK_POSITION_STRIDE: i64 = 1_000_000;
/// Gap between the positions of objects in a new block.
const OBJECT_POSITION_STRIDE: i32 = 16;
/// Location ids for new locations are picked from this range (two each:
/// one for the location and one for its dungeon), well above Daggerfall's own.
const NEW_LOCATION_IDS: std::ops::Range<u32> = 40_000..65_000;

/// One file of an exported mod.
#[derive(Debug, Clone)]
pub struct ExportedFile {
    /// Path inside the mod's folder, with `/` separators, for example `WorldData/N9000001.RDB.json`.
    pub path: String,
    pub contents: String,
}

/// A dungeon an export adds to the game.
#[derive(Debug, Clone)]
pub struct ExportedDungeon {
    pub name: String,
    pub region_index: usize,
    pub map_pixel: MapPixel,
    /// The Daggerfall dungeon it replaces, if it replaces one.
    pub replaces: Option<String>,
    /// The music file it plays, if it has its own music.
    pub custom_music: Option<String>,
}

/// A file copied into the mod as it is, such as a dungeon's music.
#[derive(Debug, Clone)]
pub struct CopiedFile {
    /// Path inside the mod's folder, with `/` separators, for example `Music/mymod_music_0.ogg`.
    pub path: String,
    /// The file to copy.
    pub source: PathBuf,
}

/// Folder inside the mod that holds the dungeons' music files.
pub const MUSIC_FOLDER: &str = "Music";

/// A project cell an export adds to the game, and the block name it gets.
#[derive(Debug, Clone)]
pub struct ExportedCell {
    pub name: String,
    pub block_name: String,
}

/// Everything an export writes.
#[derive(Debug, Clone)]
pub struct ModFiles {
    /// The mod's title, as shown in Daggerfall Unity.
    pub title: String,
    /// File-safe form of the title: the name of the mod's folder, manifest and `.dfmod`.
    pub file_name: String,
    pub files: Vec<ExportedFile>,
    /// Files copied as they are: the dungeons' music.
    pub copied_files: Vec<CopiedFile>,
    pub dungeons: Vec<ExportedDungeon>,
    pub cells: Vec<ExportedCell>,
}

impl ModFiles {
    /// Path of the manifest inside the mod's folder.
    pub fn manifest_path(&self) -> String {
        format!("{}{MANIFEST_EXTENSION}", self.file_name)
    }
}

/// Whether a project dungeon can be exported, and why not.
#[derive(Debug, Clone)]
pub struct DungeonCheck {
    pub id: ItemId,
    pub name: String,
    /// Whether it has a spot on the world map. Dungeons without one are left out of the export.
    pub placed: bool,
    /// What stops it being exported; empty when it can be.
    pub problems: Vec<String>,
    /// The Daggerfall dungeon it replaces, by name and region, if it replaces one.
    pub replaces: Option<String>,
    /// Whether it plays its own music file, which only a `.dfmod` can do.
    pub custom_music: bool,
}

impl DungeonCheck {
    /// Whether the dungeon goes into the export.
    pub fn is_ready(&self) -> bool {
        self.placed && self.problems.is_empty()
    }
}

/// Checks every project dungeon for export.
///
/// Dungeons that are not placed on the world map are listed but left out;
/// placed dungeons must have no problems for the export to go ahead.
pub fn check_project(project: &Project, library: &mut Arena2Library) -> Vec<DungeonCheck> {
    let world = library.world_map();
    let dungeons = library.dungeons();
    let placed = project.placed_dungeons(None);
    project
        .dungeons()
        .into_iter()
        .map(|dungeon| {
            let others: Vec<(&str, MapPixel)> = project.placed_dungeons(Some(dungeon.id));
            let is_placed = dungeon.location.map_pixel.is_some();
            let mut problems = Vec::new();
            if is_placed {
                problems = dungeon.location.problems(world.as_deref(), &others);
                problems.extend(dungeon_problems(project, dungeon, library, world.as_deref(), &dungeons));
                problems.extend(name_problems(dungeon, world.as_deref(), &placed));
                for room in used_project_rooms(project, dungeon) {
                    let models = distinct_models(&room.objects);
                    if models > MAX_MODELS_PER_BLOCK {
                        problems.push(format!(
                            "Cell {} uses {models} different models; a cell can use at most {MAX_MODELS_PER_BLOCK}",
                            room.name
                        ));
                    }
                }
            }
            DungeonCheck {
                id: dungeon.id,
                name: dungeon.name.clone(),
                placed: is_placed,
                problems,
                replaces: dungeon.location.replaces.as_ref().map(|replaced| replaced.label()),
                custom_music: dungeon.location.custom_music,
            }
        })
        .collect()
}

/// What is wrong with the custom music of `location`, if it has custom music.
pub fn custom_music_problem(location: &crate::project::DungeonLocation) -> Option<String> {
    if !location.custom_music {
        return None;
    }
    let file = location.custom_music_file.trim();
    if file.is_empty() {
        return Some("Custom music is ticked but no music file is chosen".to_string());
    }
    let path = Path::new(file);
    let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or("").to_lowercase();
    if !crate::project::CUSTOM_MUSIC_EXTENSIONS.contains(&extension.as_str()) {
        return Some(format!("Its custom music, {file}, is not an OGG or MP3 file"));
    }
    if !path.is_file() {
        return Some(format!("Its custom music file, {file}, was not found"));
    }
    None
}

/// What, besides its location, stops `dungeon` being exported.
fn dungeon_problems(
    project: &Project,
    dungeon: &ProjectDungeon,
    library: &mut Arena2Library,
    world: Option<&WorldMap>,
    dungeons: &[DungeonDefinition],
) -> Vec<String> {
    let mut problems = Vec::new();
    if world.is_none() {
        problems.push("MAPS.BSA was not found, so the world map cannot be read".to_string());
    }
    if let Some(problem) = custom_music_problem(&dungeon.location) {
        problems.push(problem);
    }
    if dungeon.dungeon_type.is_none() {
        problems.push("No dungeon type is chosen (it decides the enemies)".to_string());
    }
    if let Some(replaced) = &dungeon.location.replaces {
        match dungeons.iter().find(|definition| replaced.is(definition)) {
            Some(definition) if definition.classic.is_some() => {}
            _ => problems
                .push(format!("The dungeon it replaces, {}, is not in the game data", replaced.label())),
        }
    }
    if let Some(entrance) = &dungeon.location.entrance {
        match dungeons.iter().find(|definition| entrance.is(definition)) {
            Some(definition) if definition.exterior.is_some() => {}
            _ => problems.push(format!("Its entrance, {}, is not in the game data", entrance.label())),
        }
    }
    if dungeon.rooms.is_empty() {
        problems.push("It has no cells".to_string());
    }
    let starts: Vec<_> = dungeon.rooms.iter().filter(|room| room.is_starting_block).collect();
    match starts.as_slice() {
        [] if !dungeon.rooms.is_empty() => problems.push("No cell is the start cell".to_string()),
        [start] => {
            let has_exit = match start.room {
                Some(room_id) => project.room(room_id).is_some_and(|room| has_exit_door(&room.objects)),
                None => library
                    .rdb_block(&start.block_name)
                    .is_ok_and(|block| has_exit_door(&Scene::from_rdb_block(&block).objects)),
            };
            if !has_exit {
                problems.push("Its start cell has no exit door".to_string());
            }
        }
        [_, ..] => problems.push("More than one cell is the start cell".to_string()),
        [] => {}
    }
    let mut seen = HashSet::new();
    for room in &dungeon.rooms {
        if !seen.insert((room.grid_x, room.grid_z)) {
            problems.push(format!("Two cells share the grid spot {}, {}", room.grid_x, room.grid_z));
        }
        if !(-128..128).contains(&room.grid_x) || !(-128..128).contains(&room.grid_z) {
            problems.push(format!("A cell at {}, {} is too far from the start", room.grid_x, room.grid_z));
        }
        match room.room {
            Some(room_id) if project.room(room_id).is_none() => {
                problems
                    .push(format!("A cell at {}, {} is missing from the project", room.grid_x, room.grid_z));
            }
            None if !library.has_block(&room.block_name) => {
                problems.push(format!("Cell {} is not in the game data", room.block_name));
            }
            _ => {}
        }
    }
    problems
}

/// What is wrong with the dungeon's name: Daggerfall Unity finds locations
/// by name within their region, so no two there may share one.
fn name_problems(
    dungeon: &ProjectDungeon,
    world: Option<&WorldMap>,
    placed: &[(&str, MapPixel)],
) -> Vec<String> {
    let mut problems = Vec::new();
    let name = dungeon.name.trim();
    if name.is_empty() {
        problems.push("It has no name".to_string());
        return problems;
    }
    // A replacement keeps the name of the dungeon it replaces.
    if dungeon.location.replaces.is_some() {
        return problems;
    }
    let (Some(world), Some(pixel)) = (world, dungeon.location.map_pixel) else { return problems };
    let Some(region_index) = world.region_at(pixel) else { return problems };
    if world
        .locations
        .iter()
        .any(|location| location.region_index == region_index && location.name.eq_ignore_ascii_case(name))
    {
        problems.push(format!("{} already has a location called {name}", region_name(region_index)));
    }
    let same_name_here = placed
        .iter()
        .filter(|(other_name, other_pixel)| {
            other_name.trim().eq_ignore_ascii_case(name)
                && world.region_at(*other_pixel) == Some(region_index)
        })
        .count();
    if same_name_here > 1 {
        problems
            .push(format!("Another project dungeon in {} is also called {name}", region_name(region_index)));
    }
    problems
}

/// Whether `objects` include the door that leads out of a dungeon.
fn has_exit_door(objects: &[SceneObject]) -> bool {
    objects.iter().any(SceneObject::is_exit_door)
}

/// How many different models `objects` use.
fn distinct_models(objects: &[SceneObject]) -> usize {
    objects
        .iter()
        .filter_map(|object| match object.kind {
            SceneObjectKind::Model { model_id, .. } => Some(model_id),
            _ => None,
        })
        .collect::<HashSet<_>>()
        .len()
}

/// The project cells `dungeon` uses, each once.
fn used_project_rooms<'project>(
    project: &'project Project,
    dungeon: &ProjectDungeon,
) -> Vec<&'project ProjectRoom> {
    let mut seen = HashSet::new();
    dungeon
        .rooms
        .iter()
        .filter_map(|placement| placement.room)
        .filter(|room_id| seen.insert(*room_id))
        .filter_map(|room_id| project.room(room_id))
        .collect()
}

/// Makes the mod's files from the `selected` project dungeons.
///
/// Fails if one of them is not placed or has problems (see
/// [`check_project`]), or if none is selected.
pub fn build_mod(
    project: &Project,
    library: &mut Arena2Library,
    details: &ModDetails,
    fallback_title: &str,
    selected: &[ItemId],
) -> Result<ModFiles, String> {
    let checks: Vec<DungeonCheck> =
        check_project(project, library).into_iter().filter(|check| selected.contains(&check.id)).collect();
    let blocked: Vec<String> = checks
        .iter()
        .filter(|check| !check.is_ready())
        .map(|check| match check.placed {
            true => format!("{}: {}", check.name, check.problems.join("; ")),
            false => format!("{}: it is not placed on the world map", check.name),
        })
        .collect();
    if !blocked.is_empty() {
        return Err(format!("Some dungeons cannot be exported yet:\n{}", blocked.join("\n")));
    }
    let ready: Vec<&ProjectDungeon> = checks.iter().filter_map(|check| project.dungeon(check.id)).collect();
    if ready.is_empty() {
        return Err("No dungeon is chosen for export".to_string());
    }

    let world = library.world_map().ok_or("MAPS.BSA was not found, so the world map cannot be read")?;
    let definitions = library.dungeons();
    let model_tags = library.model_tags();
    let mut used_location_ids = library.location_ids();

    let title = match details.title.trim() {
        "" => fallback_title.trim().to_string(),
        title => title.to_string(),
    };
    let title = if title.is_empty() { "Dungeon Maker Mod".to_string() } else { title };
    let file_name = file_safe_name(&title, "DungeonMakerMod");

    // New block names for the project cells, stable as long as the cells keep their ids.
    let mut block_names: HashMap<ItemId, String> = HashMap::new();
    let mut taken_names = HashSet::new();
    let mut cells = Vec::new();
    let mut files = Vec::new();
    for dungeon in &ready {
        for room in used_project_rooms(project, dungeon) {
            if block_names.contains_key(&room.id) {
                continue;
            }
            let block_name = new_block_name(room, library, &mut taken_names);
            let block =
                json::Block::from_cell(&block_name, block_position(room.id), &room.objects, &model_tags);
            files.push(ExportedFile {
                path: format!("{WORLD_DATA_FOLDER}/{block_name}.json"),
                contents: to_json(&block),
            });
            cells.push(ExportedCell { name: room.name.clone(), block_name: block_name.clone() });
            block_names.insert(room.id, block_name);
        }
    }

    let mut exported = Vec::new();
    let mut taken_location_files = HashSet::new();
    let mut copied_files = Vec::new();
    let mut music_entries = Vec::new();
    for dungeon in &ready {
        let pixel = dungeon.location.map_pixel.ok_or("a dungeon lost its place")?;
        let replaced = match &dungeon.location.replaces {
            Some(replaced) => Some(
                definitions
                    .iter()
                    .find(|definition| replaced.is(definition))
                    .filter(|definition| definition.classic.is_some())
                    .ok_or_else(|| format!("{} is not in the game data", replaced.label()))?,
            ),
            None => None,
        };
        let region_index = match replaced {
            Some(definition) => definition.region_index,
            None => world.region_at(pixel).ok_or_else(|| format!("{} is in the sea", dungeon.name))?,
        };
        let entrance = dungeon
            .location
            .entrance
            .as_ref()
            .and_then(|entrance| definitions.iter().find(|definition| entrance.is(definition)))
            .and_then(|definition| definition.exterior.clone())
            .ok_or_else(|| format!("the entrance of {} is not in the game data", dungeon.name))?;
        let classic = replaced.and_then(|definition| definition.classic.as_ref());
        let location_id = match classic {
            Some(classic) => classic.location_id,
            None => {
                new_location_id(pixel, &mut used_location_ids).ok_or("there are no free location ids left")?
            }
        };
        // The song: the one chosen, or else the replaced dungeon's (a new
        // dungeon gets one picked from its location id, so they differ).
        let music_seed = match (dungeon.location.music, classic) {
            (Some(song), _) => {
                let start = classic.map_or(location_id, |classic| classic.dungeon_music_seed as u16);
                crate::music::seed_for_song(song, region_index, start)
            }
            (None, Some(classic)) => classic.dungeon_music_seed,
            (None, None) => u32::from(location_id),
        };
        // A replacement keeps the replaced dungeon's name, which the game knows it by.
        let name = replaced.map_or(dungeon.name.trim(), |definition| definition.name.as_str());
        let blocks: Vec<json::DungeonBlock> = dungeon
            .rooms
            .iter()
            .map(|placement| json::DungeonBlock {
                x: placement.grid_x as i8,
                z: placement.grid_z as i8,
                is_starting_block: placement.is_starting_block,
                block_name: match placement.room.and_then(|room_id| block_names.get(&room_id)) {
                    Some(name) => name.clone(),
                    None => placement.block_name.clone(),
                },
                water_level: 0,
                castle_block: false,
            })
            .collect();
        let location = json::Location::new_dungeon(json::NewDungeon {
            name,
            region_index,
            pixel,
            location_type: dungeon.location.location_type,
            dungeon_type: dungeon.dungeon_type.map_or(0, |dungeon_type| dungeon_type.raw()),
            discovered: dungeon.location.discovered,
            location_id,
            replaces: classic,
            music_seed,
            // A replaced location keeps the climate the game gave it.
            climate: match classic {
                Some(_) => world.location_climate_at(pixel),
                None => world.climate_at(pixel),
            },
            exterior: &entrance,
            blocks,
        });
        let file_stem = match classic {
            Some(classic) => format!("location-{region_index}-{}", classic.location_index),
            None => {
                let base = file_safe_name(dungeon.name.trim(), "Dungeon");
                let mut file_stem = format!("locationnew-{base}-{region_index}");
                let mut number = 2;
                while !taken_location_files.insert(file_stem.clone()) {
                    file_stem = format!("locationnew-{base}{number}-{region_index}");
                    number += 1;
                }
                file_stem
            }
        };
        files.push(ExportedFile {
            path: format!("{WORLD_DATA_FOLDER}/{file_stem}.json"),
            contents: to_json(&location),
        });
        // Its own music: the file goes into the mod, and the script plays it here.
        let custom_music =
            dungeon.location.custom_music.then(|| PathBuf::from(dungeon.location.custom_music_file.trim()));
        if let Some(source) = &custom_music {
            let extension =
                source.extension().and_then(|extension| extension.to_str()).unwrap_or("ogg").to_lowercase();
            let asset_name =
                format!("{}_music_{}.{extension}", file_name.to_lowercase(), music_entries.len());
            copied_files
                .push(CopiedFile { path: format!("{MUSIC_FOLDER}/{asset_name}"), source: source.clone() });
            music_entries.push(music_script::DungeonMusic {
                region_index,
                location_name: name.to_string(),
                asset_name,
            });
        }
        exported.push(ExportedDungeon {
            name: dungeon.name.clone(),
            region_index,
            map_pixel: pixel,
            replaces: replaced.map(|definition| definition.name.clone()),
            custom_music: custom_music.map(|source| source.display().to_string()),
        });
    }
    if !music_entries.is_empty() {
        files.push(ExportedFile {
            path: format!("{file_name}DungeonMusic.cs.txt"),
            contents: music_script::music_script(&file_name, &music_entries),
        });
    }

    files.sort_by(|first, second| first.path.cmp(&second.path));
    let asset_folder = format!("{MOD_ASSET_ROOT}/{file_name}");
    let mut asset_paths: Vec<String> = files
        .iter()
        .map(|file| &file.path)
        .chain(copied_files.iter().map(|file| &file.path))
        .map(|path| format!("{asset_folder}/{path}"))
        .collect();
    asset_paths.push(format!("{asset_folder}/{file_name}{MANIFEST_EXTENSION}"));
    let manifest = json::Manifest {
        mod_title: title.clone(),
        mod_version: if details.version.trim().is_empty() {
            "1.0".to_string()
        } else {
            details.version.trim().to_string()
        },
        mod_author: details.author.trim().to_string(),
        contact_info: details.contact.trim().to_string(),
        dfunity_version: DAGGERFALL_UNITY_VERSION.to_string(),
        mod_description: details.description.trim().to_string(),
        guid: if details.guid.is_empty() { new_guid(&title) } else { details.guid.clone() },
        files: asset_paths,
        dependencies: Vec::new(),
    };
    files.push(ExportedFile {
        path: format!("{file_name}{MANIFEST_EXTENSION}"),
        contents: to_json(&manifest),
    });

    Ok(ModFiles { title, file_name, files, copied_files, dungeons: exported, cells })
}

/// Writes `mod_files` into a folder named after the mod inside `folder`,
/// replacing what an earlier export left there. Returns the mod's folder.
pub fn write_mod(mod_files: &ModFiles, folder: &Path) -> Result<PathBuf, String> {
    let mod_folder = folder.join(&mod_files.file_name);
    // Files of dungeons, cells and music since removed must not linger.
    for folder in [WORLD_DATA_FOLDER, MUSIC_FOLDER] {
        let folder = mod_folder.join(folder);
        if folder.is_dir() {
            std::fs::remove_dir_all(&folder)
                .map_err(|error| format!("Could not clear '{}': {error}", folder.display()))?;
        }
    }
    if let Ok(entries) = std::fs::read_dir(&mod_folder) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".cs.txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    for file in &mod_files.copied_files {
        let path = mod_folder.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create '{}': {error}", parent.display()))?;
        }
        std::fs::copy(&file.source, &path).map_err(|error| {
            format!("Could not copy '{}' to '{}': {error}", file.source.display(), path.display())
        })?;
    }
    for file in &mod_files.files {
        let path = mod_folder.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create '{}': {error}", parent.display()))?;
        }
        std::fs::write(&path, &file.contents)
            .map_err(|error| format!("Could not write '{}': {error}", path.display()))?;
    }
    let readme = mod_folder.join("README.txt");
    std::fs::write(&readme, readme_text(mod_files))
        .map_err(|error| format!("Could not write '{}': {error}", readme.display()))?;
    Ok(mod_folder)
}

/// Explains what the exported files are and how to use them.
fn readme_text(mod_files: &ModFiles) -> String {
    let mut text = format!(
        "{title}\n{underline}\n\nMade with Daggerfall Dungeon Maker for Daggerfall Unity {DAGGERFALL_UNITY_VERSION}.\n\n",
        title = mod_files.title,
        underline = "=".repeat(mod_files.title.chars().count()),
    );
    text.push_str("Dungeons:\n");
    for dungeon in &mod_files.dungeons {
        text.push_str(&format!(
            "  {} ({}, map pixel {})",
            dungeon.name,
            region_name(dungeon.region_index),
            dungeon.map_pixel
        ));
        if let Some(replaced) = &dungeon.replaces {
            text.push_str(&format!(", replacing {replaced} and keeping its name"));
        }
        if let Some(music) = &dungeon.custom_music {
            text.push_str(&format!(", playing its own music ({music}; only in the .dfmod)"));
        }
        text.push('\n');
    }
    if !mod_files.cells.is_empty() {
        text.push_str("\nCells:\n");
        for cell in &mod_files.cells {
            text.push_str(&format!("  {} = {}\n", cell.name, cell.block_name));
        }
    }
    text.push_str(&format!(
        "\nTo play without building a mod: copy the files in {WORLD_DATA_FOLDER} into\n\
         DaggerfallUnity_Data/StreamingAssets/{WORLD_DATA_FOLDER}/ in your Daggerfall Unity folder.\n\n\
         To build a .dfmod yourself: copy this folder into a Unity project as\n\
         {MOD_ASSET_ROOT}/{name}/, open {name}{MANIFEST_EXTENSION} in Daggerfall Unity's Mod Builder and build.\n\
         The manifest already lists every file by that path.\n",
        name = mod_files.file_name,
    ));
    text
}

/// A new block name for the project cell `room`: the letter (block type) of
/// the cell it was copied from, then a number made from the cell's id, so
/// the name stays the same from one export to the next.
fn new_block_name(room: &ProjectRoom, library: &Arena2Library, taken: &mut HashSet<String>) -> String {
    let letter = room
        .based_on
        .as_deref()
        .and_then(|name| name.chars().next())
        .map(|letter| letter.to_ascii_uppercase())
        .filter(|letter| BLOCK_LETTERS.contains(letter))
        .unwrap_or('N');
    let mut number = room.id % NEW_BLOCK_NUMBERS;
    loop {
        let name = format!("{letter}{:07}.RDB", FIRST_NEW_BLOCK_NUMBER + number);
        if !library.has_block(&name) && taken.insert(name.clone()) {
            return name;
        }
        number = (number + 1) % NEW_BLOCK_NUMBERS;
    }
}

/// Position given to the new block made from the project cell `room_id`.
fn block_position(room_id: ItemId) -> i64 {
    NEW_BLOCK_POSITION_BASE + (room_id % 10_000) as i64 * NEW_BLOCK_POSITION_STRIDE
}

/// A location id for a new location at `pixel`, with the id after it free
/// for its dungeon; both are marked used. Made from the pixel, so it stays
/// the same from one export to the next.
fn new_location_id(pixel: MapPixel, used: &mut HashSet<u16>) -> Option<u16> {
    let slots = (NEW_LOCATION_IDS.end - NEW_LOCATION_IDS.start) / 2;
    let pixel_id = (pixel.y * 1000 + pixel.x).unsigned_abs();
    for step in 0..slots {
        let id = NEW_LOCATION_IDS.start + ((pixel_id + step) % slots) * 2;
        let (id, next) = (id as u16, id as u16 + 1);
        if !used.contains(&id) && !used.contains(&next) {
            used.insert(id);
            used.insert(next);
            return Some(id);
        }
    }
    None
}

/// `name` with only letters, digits and underscores (spaces become
/// underscores), for file names; `fallback` if nothing is left.
pub fn file_safe_name(name: &str, fallback: &str) -> String {
    let safe: String = name
        .chars()
        .filter_map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => Some(character),
            ' ' | '-' => Some('_'),
            _ => None,
        })
        .collect();
    let safe = safe.trim_matches('_').to_string();
    if safe.is_empty() {
        fallback.to_string()
    } else {
        safe
    }
}

/// A GUID for a new mod, in the usual 8-4-4-4-12 form.
pub fn new_guid(seed: &str) -> String {
    use std::hash::{BuildHasher, Hasher};
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let mut halves = [0u64; 2];
    for (index, half) in halves.iter_mut().enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write(seed.as_bytes());
        hasher.write_u128(time);
        hasher.write_usize(index);
        *half = hasher.finish();
    }
    let hex = format!("{:016x}{:016x}", halves[0], halves[1]);
    format!("{}-{}-4{}-{}-{}", &hex[0..8], &hex[8..12], &hex[13..16], &hex[16..20], &hex[20..32])
}

/// `value` as indented JSON.
fn to_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).expect("export data always serialises")
}

/// The native (game data) form of an object's position and rotation.
pub(crate) struct NativePlacement {
    pub position: glam::IVec3,
    pub rotation: glam::IVec3,
}

impl NativePlacement {
    pub(crate) fn of(object: &SceneObject) -> Self {
        // World rotations turn the opposite way to native ones.
        let rotation = -object.rotation_degrees * NATIVE_ROTATION_UNITS_PER_DEGREE;
        Self {
            position: world_to_native(object.position),
            rotation: glam::IVec3::new(
                rotation.x.round() as i32,
                rotation.y.round() as i32,
                rotation.z.round() as i32,
            ),
        }
    }
}

/// A light's radius in native units.
pub(crate) fn native_radius(radius: f32) -> u16 {
    (radius / WORLD_SCALE).round().clamp(0.0, f32::from(u16::MAX)) as u16
}

/// The object groups of a cell, as RDB blocks store them: the cell's own
/// lists in order, then one more for the objects the user added.
pub(crate) fn object_groups(objects: &[SceneObject]) -> Vec<Vec<usize>> {
    let own_groups = objects
        .iter()
        .filter(|object| object.group_index != ADDED_OBJECTS_GROUP)
        .map(|object| object.group_index + 1)
        .max()
        .unwrap_or(0);
    let has_added = objects.iter().any(|object| object.group_index == ADDED_OBJECTS_GROUP);
    let mut groups = vec![Vec::new(); own_groups.max(1) + usize::from(has_added)];
    let added_group = groups.len() - 1;
    for (index, object) in objects.iter().enumerate() {
        let group = if object.group_index == ADDED_OBJECTS_GROUP { added_group } else { object.group_index };
        groups[group].push(index);
    }
    groups
}

/// Gap between objects' positions in a new block; see [`OBJECT_POSITION_STRIDE`].
pub(crate) const fn object_position(index: usize) -> i32 {
    (index as i32 + 1) * OBJECT_POSITION_STRIDE
}

/// Tag for `model_id`, as Daggerfall's cells tag it.
pub(crate) fn model_tag(model_id: u32, tags: &HashMap<u32, String>) -> String {
    tags.get(&model_id).cloned().unwrap_or_else(|| UNKNOWN_MODEL_TAG.to_string())
}

/// Region name as Daggerfall Unity keys it (never translated).
pub(crate) fn canonical_region_name(region_index: usize) -> &'static str {
    REGION_NAMES.get(region_index).copied().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_safe_names_keep_letters_and_digits() {
        assert_eq!(file_safe_name("Castle Gloom's Keep-2", "x"), "Castle_Glooms_Keep_2");
        assert_eq!(file_safe_name("???", "Fallback"), "Fallback");
    }

    #[test]
    fn location_ids_are_stable_and_skip_used_ones() {
        let pixel = MapPixel::new(201, 312);
        let mut used = HashSet::new();
        let first = new_location_id(pixel, &mut used).unwrap();
        assert!(used.contains(&first) && used.contains(&(first + 1)));
        assert_eq!(first % 2, 0);
        let mut fresh = HashSet::new();
        assert_eq!(new_location_id(pixel, &mut fresh), Some(first));
        // Taken ids are skipped.
        let second = new_location_id(pixel, &mut used).unwrap();
        assert_ne!(second, first);
    }

    #[test]
    fn guids_have_the_usual_form() {
        let guid = new_guid("My mod");
        let parts: Vec<usize> = guid.split('-').map(str::len).collect();
        assert_eq!(parts, [8, 4, 4, 4, 12]);
    }

    #[test]
    fn added_objects_get_their_own_group() {
        let object = |group_index| SceneObject {
            id: 1,
            name: String::new(),
            group_index,
            kind: SceneObjectKind::Light { radius: 1.0, data: Default::default() },
            position: glam::Vec3::ZERO,
            rotation_degrees: glam::Vec3::ZERO,
            visible: true,
            next: None,
        };
        let objects = [object(1), object(ADDED_OBJECTS_GROUP), object(0)];
        assert_eq!(object_groups(&objects), vec![vec![2], vec![0], vec![1]]);
        assert_eq!(object_groups(&[object(ADDED_OBJECTS_GROUP)]), vec![vec![], vec![0]]);
    }
}
