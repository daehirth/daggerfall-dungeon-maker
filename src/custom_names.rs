//! Names the user gives to rooms and models, and the assets they marked as favourites.
//!
//! Daggerfall identifies rooms by file name (`N0000004.RDB`) and models by
//! number (`70300`). Users can give either a name of their own, which the
//! editor shows wherever that room or model appears. These names belong to the
//! user, not to the game: they are saved with the application's settings and
//! never written to the game data. So are the user's favourite assets, which
//! the asset palette lists on a tab of their own.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::AssetCatalog;

/// The user's own names for rooms and models.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomNames {
    /// Names of rooms, keyed by RDB block name (for example `N0000004.RDB`).
    room_names: BTreeMap<String, String>,
    /// Names of models, keyed by `ARCH3D.BSA` model id.
    model_names: BTreeMap<u32, String>,
    /// Assets marked as favourites, in the order they were marked.
    favorites: Vec<Favorite>,
}

/// An asset the user marked as a favourite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Favorite {
    Model {
        model_id: u32,
    },
    /// A flat or editor marker, by texture archive and record.
    Flat {
        archive: u16,
        record: u8,
    },
    Light,
}

impl CustomNames {
    /// The assets marked as favourites, in the order they were marked.
    pub fn favorites(&self) -> &[Favorite] {
        &self.favorites
    }

    /// Whether `favorite` is marked as a favourite.
    pub fn is_favorite(&self, favorite: Favorite) -> bool {
        self.favorites.contains(&favorite)
    }

    /// Marks `favorite` as a favourite, or unmarks it if it is one.
    pub fn toggle_favorite(&mut self, favorite: Favorite) {
        match self.favorites.iter().position(|existing| *existing == favorite) {
            Some(position) => {
                self.favorites.remove(position);
            }
            None => self.favorites.push(favorite),
        }
    }

    /// The user's name for the room `block_name`, if they gave it one.
    pub fn room_name(&self, block_name: &str) -> Option<&str> {
        self.room_names.get(block_name).map(|name| name.trim())
    }

    /// The user's name for model `model_id`, if they gave it one.
    pub fn model_name(&self, model_id: u32) -> Option<&str> {
        self.model_names.get(&model_id).map(|name| name.trim())
    }

    /// The name as typed, for editing in a text field. Empty if there is none.
    pub fn room_name_for_editing(&self, block_name: &str) -> String {
        self.room_names.get(block_name).cloned().unwrap_or_default()
    }

    /// The name as typed, for editing in a text field. Empty if there is none.
    pub fn model_name_for_editing(&self, model_id: u32) -> String {
        self.model_names.get(&model_id).cloned().unwrap_or_default()
    }

    /// Names the room `block_name`. A blank name removes the custom name.
    ///
    /// The text is stored as typed, so spaces typed while editing are kept;
    /// surrounding spaces are ignored when the name is shown.
    pub fn set_room_name(&mut self, block_name: &str, name: &str) {
        set_or_remove(&mut self.room_names, block_name.to_string(), name);
    }

    /// Names model `model_id`. A blank name removes the custom name.
    pub fn set_model_name(&mut self, model_id: u32, name: &str) {
        set_or_remove(&mut self.model_names, model_id, name);
    }

    /// Display name of a room: "My name (N0000004.RDB)", or just the block name.
    pub fn room_label(&self, block_name: &str) -> String {
        match self.room_name(block_name) {
            Some(name) => format!("{name} ({block_name})"),
            None => block_name.to_string(),
        }
    }

    /// Display name of a model.
    ///
    /// Uses the user's name if there is one, otherwise the category worked
    /// out from the game data followed by the id, for example "Door 55000".
    pub fn model_label(&self, model_id: u32, catalog: &AssetCatalog) -> String {
        if let Some(name) = self.model_name(model_id) {
            return name.to_string();
        }
        match catalog.model_description(model_id) {
            Some(description) => format!("{} {model_id}", description.category),
            None => format!("Model {model_id}"),
        }
    }

    /// Whether the user's name for the room contains `lowercase_search`.
    pub fn room_name_matches(&self, block_name: &str, lowercase_search: &str) -> bool {
        self.room_name(block_name).is_some_and(|name| name.to_lowercase().contains(lowercase_search))
    }

    /// Whether the user's name for the model contains `lowercase_search`.
    pub fn model_name_matches(&self, model_id: u32, lowercase_search: &str) -> bool {
        self.model_name(model_id).is_some_and(|name| name.to_lowercase().contains(lowercase_search))
    }
}

/// Stores `name` under `key`, or removes the entry if `name` is blank.
fn set_or_remove<Key: Ord>(names: &mut BTreeMap<Key, String>, key: Key, name: &str) {
    if name.trim().is_empty() {
        names.remove(&key);
    } else {
        names.insert(key, name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_names_remove_the_entry() {
        let mut names = CustomNames::default();
        names.set_room_name("N0000004.RDB", "Big hall ");
        assert_eq!(names.room_name("N0000004.RDB"), Some("Big hall"));
        assert_eq!(names.room_name_for_editing("N0000004.RDB"), "Big hall ");
        assert_eq!(names.room_label("N0000004.RDB"), "Big hall (N0000004.RDB)");

        names.set_room_name("N0000004.RDB", "   ");
        assert_eq!(names.room_name("N0000004.RDB"), None);
        assert_eq!(names, CustomNames::default());
    }

    #[test]
    fn model_labels_fall_back_to_the_id() {
        let mut names = CustomNames::default();
        let catalog = AssetCatalog::default();
        assert_eq!(names.model_label(70300, &catalog), "Model 70300");

        names.set_model_name(70300, "Way out");
        assert_eq!(names.model_label(70300, &catalog), "Way out");
        assert!(names.model_name_matches(70300, "way"));
    }
}
