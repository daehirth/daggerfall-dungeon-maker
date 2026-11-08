//! Owns the loaded Daggerfall archives and caches decoded assets.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec2;

use super::blocks::{parse_rdb_block, RdbBlock};
use super::bsa::BsaArchive;
use super::error::{read_file, Arena2Error, Arena2Result};
use super::maps::{read_all_dungeons, DungeonDefinition};
use super::mesh::{parse_model, ModelMesh};
use super::texture::{read_archive_name, Palette, RgbaImage, TextureArchive, ARCHIVE_HEADER_SIZE};
use super::{TextureKey, WORLD_SCALE};

/// Archive holding every 3D model.
const MODELS_FILE: &str = "ARCH3D.BSA";
/// Archive holding every city (RMB) and dungeon (RDB) block.
const BLOCKS_FILE: &str = "BLOCKS.BSA";
/// Archive holding region, location and dungeon layout data. Optional: only needed to open dungeons.
const MAPS_FILE: &str = "MAPS.BSA";
/// Palette used by all `TEXTURE.###` files.
const TEXTURE_PALETTE_FILE: &str = "ART_PAL.COL";

/// Files that must be present for a folder to be accepted as `ARENA2`.
pub const REQUIRED_FILES: [&str; 3] = [MODELS_FILE, BLOCKS_FILE, TEXTURE_PALETTE_FILE];

/// Provides access to the game data in an `ARENA2` folder.
///
/// Nothing is pre-computed or bundled with the program: the archives are read
/// from the user's folder when the library is opened, and models and textures
/// are decoded the first time they are requested and cached in memory for the
/// rest of the session. Failed lookups are cached too, so missing data is only
/// reported once.
pub struct Arena2Library {
    root: PathBuf,
    /// Every file in `root`, keyed by upper-case name, so lookups work whatever
    /// case the files were installed with (Linux file systems are case sensitive).
    files_by_name: HashMap<String, PathBuf>,
    models_archive: BsaArchive,
    blocks_archive: BsaArchive,
    /// `None` if the folder has no `MAPS.BSA`; rooms still work but dungeons cannot be opened.
    maps_archive: Option<BsaArchive>,
    texture_palette: Palette,
    model_cache: HashMap<u32, Option<Arc<ModelMesh>>>,
    texture_archive_cache: HashMap<u16, Option<TextureArchive>>,
    /// Names from texture archive headers; read separately so naming does not load whole archives.
    texture_archive_name_cache: HashMap<u16, Option<String>>,
    dungeon_cache: Option<Arc<[DungeonDefinition]>>,
}

impl Arena2Library {
    /// Returns a list of required files missing from `folder`. An empty list means the folder is usable.
    pub fn missing_files(folder: &Path) -> Vec<&'static str> {
        REQUIRED_FILES
            .into_iter()
            .filter(|file_name| !find_path(folder, file_name).is_some_and(|path| path.is_file()))
            .collect()
    }

    /// Resolves a user supplied folder to an `ARENA2` folder.
    ///
    /// Users often pick the game install folder instead of `ARENA2` itself, so
    /// a few well known sub-folders are also checked.
    pub fn resolve_folder(folder: &Path) -> Option<PathBuf> {
        const SUBFOLDER_CANDIDATES: [&str; 4] = ["", "ARENA2", "DAGGER/ARENA2", "DF/DAGGER/ARENA2"];
        SUBFOLDER_CANDIDATES
            .iter()
            .filter_map(|subfolder| find_path(folder, subfolder))
            .find(|candidate| Self::missing_files(candidate).is_empty())
    }

    /// Loads the archives in the `ARENA2` folder at `root`.
    pub fn open(root: &Path) -> Arena2Result<Self> {
        let missing = Self::missing_files(root);
        if !missing.is_empty() {
            return Err(Arena2Error::not_found(format!(
                "'{}' is missing {}",
                root.display(),
                missing.join(", ")
            )));
        }

        let files_by_name = index_files(root);
        let file_path = |name: &str| files_by_name.get(name).cloned().unwrap_or_else(|| root.join(name));

        let texture_palette = Palette::from_bytes(&read_file(&file_path(TEXTURE_PALETTE_FILE))?)?;
        let maps_path = file_path(MAPS_FILE);
        let maps_archive = match maps_path.is_file() {
            true => Some(BsaArchive::open(&maps_path)?),
            false => None,
        };
        let models_archive = BsaArchive::open(&file_path(MODELS_FILE))?;
        let blocks_archive = BsaArchive::open(&file_path(BLOCKS_FILE))?;

        Ok(Self {
            root: root.to_path_buf(),
            files_by_name,
            models_archive,
            blocks_archive,
            maps_archive,
            texture_palette,
            model_cache: HashMap::new(),
            texture_archive_cache: HashMap::new(),
            texture_archive_name_cache: HashMap::new(),
            dungeon_cache: None,
        })
    }

    /// Whether `MAPS.BSA` was found, so dungeons can be listed and opened.
    pub fn has_dungeons(&self) -> bool {
        self.maps_archive.is_some()
    }

    /// Returns every dungeon defined in `MAPS.BSA`, sorted by region then name.
    ///
    /// The list is read on first use and cached. It is empty if `MAPS.BSA` is missing.
    pub fn dungeons(&mut self) -> Arc<[DungeonDefinition]> {
        let maps_archive = &self.maps_archive;
        self.dungeon_cache
            .get_or_insert_with(|| maps_archive.as_ref().map(read_all_dungeons).unwrap_or_default().into())
            .clone()
    }

    /// The `ARENA2` folder this library was loaded from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the decoded model with id `model_id`, or `None` if it is missing or invalid.
    pub fn model(&mut self, model_id: u32) -> Option<Arc<ModelMesh>> {
        let models_archive = &self.models_archive;
        self.model_cache
            .entry(model_id)
            .or_insert_with(|| {
                let data = models_archive.record_by_id(model_id)?;
                match parse_model(model_id, data) {
                    Ok(mesh) => Some(Arc::new(mesh)),
                    Err(error) => {
                        log::warn!("could not decode model {model_id}: {error}");
                        None
                    }
                }
            })
            .clone()
    }

    /// Decodes the model with id `model_id` without adding it to the cache.
    ///
    /// Used when every model is inspected once, so that the whole of
    /// `ARCH3D.BSA` is not kept in memory afterwards.
    pub fn read_model_uncached(&self, model_id: u32) -> Option<ModelMesh> {
        if let Some(cached) = self.model_cache.get(&model_id) {
            return cached.as_deref().cloned();
        }
        let data = self.models_archive.record_by_id(model_id)?;
        parse_model(model_id, data).ok()
    }

    /// Returns the name stored in the header of `TEXTURE.{archive}`, for
    /// example "Door Set", or `None` if the file is missing or has no name.
    pub fn texture_archive_name(&mut self, archive: u16) -> Option<&str> {
        let files_by_name = &self.files_by_name;
        self.texture_archive_name_cache
            .entry(archive)
            .or_insert_with(|| {
                let mut header = [0u8; ARCHIVE_HEADER_SIZE];
                let path = files_by_name.get(&texture_file_name(archive))?;
                std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut header)).ok()?;
                read_archive_name(&header).ok().flatten()
            })
            .as_deref()
    }

    /// Decodes the image for `texture`, or returns `None` if it is missing or invalid.
    pub fn texture_image(&mut self, texture: TextureKey) -> Option<RgbaImage> {
        let palette = self.texture_palette.clone();
        let archive = self.texture_archive(texture.archive)?;
        match archive.decode(texture.record, &palette) {
            Ok(image) => Some(image),
            Err(error) => {
                log::warn!("could not decode {texture}: {error}");
                None
            }
        }
    }

    /// Returns the size of `texture` when drawn as a billboard, in world units.
    pub fn billboard_world_size(&mut self, texture: TextureKey) -> Option<Vec2> {
        let info = self.texture_archive(texture.archive)?.record_info(texture.record)?;
        let (width, height) = info.billboard_size();
        Some(Vec2::new(width as f32, height as f32) * WORLD_SCALE)
    }

    /// Returns the number of image records in `TEXTURE.{archive}`, or zero if it cannot be read.
    pub fn texture_record_count(&mut self, archive: u16) -> usize {
        self.texture_archive(archive).map_or(0, TextureArchive::record_count)
    }

    /// Returns the identifier of every model in `ARCH3D.BSA`, sorted ascending.
    pub fn model_ids(&self) -> Vec<u32> {
        let mut model_ids: Vec<u32> =
            self.models_archive.entries().iter().filter_map(|entry| entry.id).collect();
        model_ids.sort_unstable();
        model_ids
    }

    /// Returns the names of every RDB dungeon block (room), sorted alphabetically.
    pub fn rdb_block_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .blocks_archive
            .entries()
            .iter()
            .filter(|entry| entry.name.to_ascii_uppercase().ends_with(".RDB"))
            .map(|entry| entry.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Parses the RDB block called `name`.
    pub fn rdb_block(&self, name: &str) -> Arena2Result<RdbBlock> {
        let data = self
            .blocks_archive
            .record_by_name(name)
            .ok_or_else(|| Arena2Error::not_found(format!("block '{name}' in {BLOCKS_FILE}")))?;
        parse_rdb_block(name, data)
    }

    /// Loads (or fetches from cache) the texture archive with number `archive_index`.
    fn texture_archive(&mut self, archive_index: u16) -> Option<&TextureArchive> {
        let files_by_name = &self.files_by_name;
        self.texture_archive_cache
            .entry(archive_index)
            .or_insert_with(|| {
                let path = files_by_name.get(&texture_file_name(archive_index))?;
                let data = read_file(path).ok()?;
                match TextureArchive::parse(archive_index, data) {
                    Ok(archive) => Some(archive),
                    Err(error) => {
                        log::warn!("could not read {}: {error}", path.display());
                        None
                    }
                }
            })
            .as_ref()
    }
}

/// Name of the texture archive file with number `archive`, for example `TEXTURE.210`.
fn texture_file_name(archive: u16) -> String {
    format!("TEXTURE.{archive:03}")
}

/// Lists the files directly inside `folder`, keyed by upper-case file name.
fn index_files(folder: &Path) -> HashMap<String, PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| Some((entry.file_name().to_str()?.to_ascii_uppercase(), entry.path())))
        .collect()
}

/// Joins the `/`-separated `relative` path onto `folder`, matching each part
/// without regard to case. Returns `None` if a part does not exist.
///
/// Daggerfall's files are named in upper case, but copies on case-sensitive
/// file systems are often lower case.
fn find_path(folder: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = folder.to_path_buf();
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        let exact = path.join(part);
        path = match exact.exists() {
            true => exact,
            false => std::fs::read_dir(&path)
                .ok()?
                .flatten()
                .find(|entry| entry.file_name().to_str().is_some_and(|name| name.eq_ignore_ascii_case(part)))?
                .path(),
        };
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_path_ignores_case() {
        let folder = std::env::temp_dir().join(format!("ddm-find-path-{}", std::process::id()));
        let arena2 = folder.join("dagger").join("arena2");
        std::fs::create_dir_all(&arena2).unwrap();
        std::fs::write(arena2.join("arch3d.bsa"), b"").unwrap();

        let found = find_path(&folder, "DAGGER/ARENA2/ARCH3D.BSA");
        let found_is_file = found.as_ref().is_some_and(|path| path.is_file());
        let missing = find_path(&folder, "DAGGER/ARENA2/BLOCKS.BSA");
        let indexed = index_files(&arena2);
        std::fs::remove_dir_all(&folder).unwrap();

        // On case-insensitive file systems the typed case is kept, so compare without case.
        let found = found.expect("ARCH3D.BSA should be found");
        assert!(found_is_file);
        assert!(found.to_string_lossy().eq_ignore_ascii_case(&arena2.join("arch3d.bsa").to_string_lossy()));
        assert_eq!(missing, None);
        assert_eq!(indexed.get("ARCH3D.BSA"), Some(&arena2.join("arch3d.bsa")));
    }
}
