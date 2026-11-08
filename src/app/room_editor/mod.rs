//! A complete editor for one room: outliner, properties, asset palette and 3D viewport.
//!
//! A [`RoomEditor`] owns everything specific to the room being edited
//! (scene, camera, selection, undo history) and borrows the shared game data
//! and renderer through [`EditorResources`]. That lets the same editor run in
//! the main window or in its own window opened from the dungeon view.

mod asset_palette;
mod history;
mod panels;
mod viewport;

use eframe::{egui, egui_wgpu};
use glam::Vec3;

use super::viewport_tools::EditorResources;
use crate::arena2::{Arena2Library, RDB_BLOCK_SIDE};
use crate::camera::OrbitCamera;
use crate::render::{ViewSettings, ViewportTarget};
use crate::scene::{Scene, SceneObjectKind};
use asset_palette::AssetPalette;
use history::{EditHistory, SceneEdit};
use viewport::ViewportState;

/// How far a duplicated object is offset from the original, in world units.
const DUPLICATE_OFFSET: Vec3 = Vec3::new(1.0, 0.0, 0.0);
/// Radius used to frame objects without a mesh (flats and lights), in world units.
const DEFAULT_FOCUS_RADIUS: f32 = 2.0;

/// Grid snapping used when moving or placing objects.
#[derive(Debug, Clone, Copy)]
pub struct SnapSettings {
    /// Whether snapping is on. Holding Ctrl while dragging inverts this.
    pub enabled: bool,
    /// Snap increment in world units.
    pub step: f32,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            step: 0.4, // 16 native units.
        }
    }
}

/// Editor state for one room.
pub struct RoomEditor {
    /// The room being edited.
    pub scene: Scene,
    /// Title shown in window titles, for example the room name and its dungeon.
    title: String,
    /// Prefix for widget ids so several editors can be open at once.
    id: egui::Id,
    selected_object: Option<usize>,
    camera: OrbitCamera,
    view_settings: ViewSettings,
    snap: SnapSettings,
    history: EditHistory,
    /// Object currently being edited in the properties panel, so a drag records one undo step.
    property_edit_session: Option<usize>,
    viewport: ViewportState,
    palette: AssetPalette,
    /// Height of the floor grid, placed just under the room.
    grid_height: f32,
    target: ViewportTarget,
    status_message: String,
}

impl RoomEditor {
    /// Creates an editor for `scene` and frames the camera on it.
    pub fn new(scene: Scene, title: String, id: egui::Id, library: &mut Arena2Library) -> Self {
        let status_message = format!("{title}: {} objects", scene.objects.len());
        let mut editor = Self {
            scene,
            title,
            id,
            selected_object: None,
            camera: OrbitCamera::default(),
            view_settings: ViewSettings::default(),
            snap: SnapSettings::default(),
            history: EditHistory::default(),
            property_edit_session: None,
            viewport: ViewportState::default(),
            palette: AssetPalette::default(),
            grid_height: 0.0,
            target: ViewportTarget::default(),
            status_message,
        };
        editor.frame_room(library);
        if let Some((minimum, _)) = editor.scene.bounds(library) {
            editor.grid_height = minimum.y - 0.05;
        }
        editor
    }

    /// Title of the room, for window titles.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Base id of this editor's widgets, unique among open editors.
    pub fn id(&self) -> egui::Id {
        self.id
    }

    /// Latest status message, for the status bar.
    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    /// Frees the GPU image this editor renders into. Call when the editor is closed.
    pub fn release(&mut self, render_state: &egui_wgpu::RenderState) {
        self.target.release(render_state);
    }

    /// Draws the side panels, asset palette and viewport into `context`.
    ///
    /// Call this after any top or bottom bars of the window have been added.
    pub fn show(&mut self, context: &egui::Context, resources: &mut EditorResources<'_>) {
        self.show_outliner(context, resources);
        self.show_properties(context, resources);
        self.palette.show(context, self.id, resources);
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(context, |ui| self.show_viewport(ui, resources));
    }

    /// Handles editor keyboard shortcuts in `context`.
    pub fn handle_shortcuts(&mut self, context: &egui::Context, library: &mut Arena2Library) {
        use egui::{Key, KeyboardShortcut, Modifiers};

        let shortcut = |modifiers, key| {
            context.input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || shortcut(Modifiers::COMMAND, Key::Y) {
            self.redo();
        }
        if shortcut(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
        if shortcut(Modifiers::COMMAND, Key::D) {
            self.duplicate_selection();
        }

        // Single key shortcuts must not fire while the user is typing in a text field.
        if context.wants_keyboard_input() {
            return;
        }
        if shortcut(Modifiers::NONE, Key::Delete) {
            self.delete_selection();
        }
        if shortcut(Modifiers::NONE, Key::F) {
            self.focus_selection(library);
        }
        if shortcut(Modifiers::NONE, Key::Home) {
            self.frame_room(library);
        }

        // While an asset is armed for placement, Escape and R act on it instead of the selection.
        let is_placing = self.palette.armed.is_some();
        if shortcut(Modifiers::NONE, Key::Escape) {
            if is_placing {
                self.palette.armed = None;
            } else {
                self.selected_object = None;
            }
        }
        for (modifiers, degrees) in [(Modifiers::SHIFT, -90.0), (Modifiers::NONE, 90.0)] {
            if shortcut(modifiers, Key::R) {
                if is_placing {
                    self.palette.rotate_armed(degrees);
                } else {
                    self.rotate_selection(degrees);
                }
            }
        }
    }

    /// Adds the Edit menu entries.
    pub fn edit_menu(&mut self, ui: &mut egui::Ui) {
        let has_selection = self.selected_object.is_some();
        let menu_button = |ui: &mut egui::Ui, enabled: bool, label: &str, shortcut: &str| {
            let clicked = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked();
            if clicked {
                ui.close_menu();
            }
            clicked
        };

        if menu_button(ui, self.history.can_undo(), "Undo", "Ctrl+Z") {
            self.undo();
        }
        if menu_button(ui, self.history.can_redo(), "Redo", "Ctrl+Y") {
            self.redo();
        }
        ui.separator();
        if menu_button(ui, has_selection, "Duplicate", "Ctrl+D") {
            self.duplicate_selection();
        }
        if menu_button(ui, has_selection, "Rotate 90°", "R") {
            self.rotate_selection(90.0);
        }
        if menu_button(ui, has_selection, "Delete", "Del") {
            self.delete_selection();
        }
        ui.separator();
        ui.checkbox(&mut self.snap.enabled, "Snap to grid while moving");
        ui.horizontal(|ui| {
            ui.label("Snap step");
            ui.add(egui::DragValue::new(&mut self.snap.step).speed(0.05).range(0.025..=10.0).suffix(" m"));
        });
    }

    /// Adds the View menu entries.
    pub fn view_menu(&mut self, ui: &mut egui::Ui, library: &mut Arena2Library) {
        if ui.add(egui::Button::new("Frame room").shortcut_text("Home")).clicked() {
            self.frame_room(library);
            ui.close_menu();
        }
        let has_selection = self.selected_object.is_some();
        if ui.add_enabled(has_selection, egui::Button::new("Frame selection").shortcut_text("F")).clicked() {
            self.focus_selection(library);
            ui.close_menu();
        }
        ui.separator();
        ui.checkbox(&mut self.palette.is_open, "Asset palette");
        ui.separator();
        view_settings_menu(ui, &mut self.view_settings);
    }

    /// Frames the whole room.
    fn frame_room(&mut self, library: &mut Arena2Library) {
        match self.scene.bounds(library) {
            Some((minimum, maximum)) => self.camera.frame_bounds(minimum, maximum),
            None => self
                .camera
                .frame_bounds(Vec3::new(0.0, -5.0, 0.0), Vec3::new(RDB_BLOCK_SIDE, 5.0, RDB_BLOCK_SIDE)),
        }
    }

    /// Centres the camera on the selected object.
    fn focus_selection(&mut self, library: &mut Arena2Library) {
        let Some(object) = self.selected_object.and_then(|index| self.scene.objects.get(index)) else {
            return;
        };
        let radius = match object.kind {
            SceneObjectKind::Model { model_id, .. } => {
                library.model(model_id).map_or(DEFAULT_FOCUS_RADIUS, |mesh| mesh.bounding_radius())
            }
            _ => DEFAULT_FOCUS_RADIUS,
        };
        let center = object.position;
        self.camera.frame_bounds(center - Vec3::splat(radius), center + Vec3::splat(radius));
    }

    /// Removes the selected object.
    fn delete_selection(&mut self) {
        let Some(index) = self.selected_object.filter(|&index| index < self.scene.objects.len()) else {
            return;
        };
        let object = self.scene.objects.remove(index);
        self.status_message = format!("Deleted {}", object.name);
        self.history.record(SceneEdit::Insert { index, object });
        self.selected_object = None;
    }

    /// Copies the selected object and selects the copy.
    fn duplicate_selection(&mut self) {
        let Some(original) = self.selected_object.and_then(|index| self.scene.objects.get(index)) else {
            return;
        };
        let mut copy = original.clone();
        copy.position += DUPLICATE_OFFSET;
        let copy_index = self.scene.objects.len();
        self.status_message = format!("Duplicated {}", copy.name);
        self.scene.objects.push(copy);
        self.history.record(SceneEdit::Remove { index: copy_index });
        self.selected_object = Some(copy_index);
    }

    /// Turns the selected object around the vertical axis by `degrees`.
    fn rotate_selection(&mut self, degrees: f32) {
        let Some(index) = self.selected_object else { return };
        let Some(object) = self.scene.objects.get_mut(index) else { return };
        self.history.record(SceneEdit::Replace { index, object: object.clone() });
        object.rotation_degrees.y = (object.rotation_degrees.y + degrees).rem_euclid(360.0);
    }

    /// Reverts the last edit.
    fn undo(&mut self) {
        if self.history.undo(&mut self.scene.objects) {
            self.after_history_change();
            self.status_message = "Undo".into();
        }
    }

    /// Re-applies the last undone edit.
    fn redo(&mut self) {
        if self.history.redo(&mut self.scene.objects) {
            self.after_history_change();
            self.status_message = "Redo".into();
        }
    }

    /// Keeps the selection valid after the object list changed underneath it.
    fn after_history_change(&mut self) {
        self.property_edit_session = None;
        if self.selected_object.is_some_and(|index| index >= self.scene.objects.len()) {
            self.selected_object = None;
        }
    }
}

/// Adds checkboxes for every [`ViewSettings`] option.
pub fn view_settings_menu(ui: &mut egui::Ui, settings: &mut ViewSettings) {
    ui.checkbox(&mut settings.cull_back_faces, "Hide back faces (see into rooms)");
    ui.checkbox(&mut settings.show_grid, "Grid");
    ui.checkbox(&mut settings.show_room_outline, "Block outlines");
    ui.separator();
    ui.checkbox(&mut settings.show_models, "Models");
    ui.checkbox(&mut settings.show_flats, "Flats");
    ui.checkbox(&mut settings.show_markers, "Editor markers");
    ui.checkbox(&mut settings.show_lights, "Lights");
}
