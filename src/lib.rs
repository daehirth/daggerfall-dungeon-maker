//! Daggerfall Dungeon Maker: a CAD-style viewer and editor for Daggerfall dungeon rooms.
//!
//! The crate is split into a library and a thin binary so the data readers
//! can be tested independently of the user interface:
//!
//! * [`arena2`] reads the original game files (models, blocks, textures).
//! * [`scene`] turns an RDB room block into an editable list of objects.
//! * [`catalog`] lists the models, flats, markers and lights that can be placed,
//!   and describes each model from its data.
//! * [`custom_names`] holds the names users give rooms and models in the editor.
//! * [`encounters`] lists which enemies random enemy markers can become, by dungeon type.
//! * [`project`] reads and writes project files holding the user's own rooms and dungeons.
//! * [`export`] turns the project into a Daggerfall Unity mod.
//! * [`music`] chooses a dungeon's music and plays songs to preview them.
//! * [`connectors`] finds the open faces where dungeon pieces join, for snapping.
//! * [`camera`] and [`picking`] provide viewport navigation and selection.
//! * [`render`] draws a scene with wgpu into an offscreen texture.
//! * [`app`] is the egui application: menus, dialogs, panels and the viewport.

pub mod app;
pub mod arena2;
pub mod camera;
pub mod catalog;
pub mod connectors;
pub mod custom_names;
pub mod encounters;
pub mod export;
pub mod music;
pub mod picking;
pub mod project;
pub mod render;
pub mod scene;
