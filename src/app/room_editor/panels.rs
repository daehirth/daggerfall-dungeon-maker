//! The outliner (object list) and properties panels of the room editor.
//!
//! Both panels show the user's own names: the room's name above the
//! outliner, and model names in the outliner rows and properties. Names are
//! edited here too and apply everywhere the room or model appears.

use eframe::egui;

use super::history::SceneEdit;
use super::RoomEditor;
use crate::app::room_browser::show_room_name_field;
use crate::app::viewport_tools::EditorResources;
use crate::arena2::world_to_native;
use crate::catalog::AssetCatalog;
use crate::custom_names::CustomNames;
use crate::scene::{group_label, SceneObject, SceneObjectKind};

/// Drag speed of position fields, in world units per pixel.
const POSITION_DRAG_SPEED: f32 = 0.02;
/// Drag speed of rotation fields, in degrees per pixel.
const ROTATION_DRAG_SPEED: f32 = 0.5;

impl RoomEditor {
    /// Draws the outliner listing every object, grouped by the RDB object list it came from.
    pub(super) fn show_outliner(&mut self, context: &egui::Context, resources: &mut EditorResources<'_>) {
        egui::SidePanel::left(self.id.with("outliner")).resizable(true).default_width(240.0).show(
            context,
            |ui| {
                ui.heading(&self.scene.block_name);
                show_room_name_field(ui, &self.scene.block_name, resources.custom_names);
                ui.label(egui::RichText::new(format!("{} objects", self.scene.objects.len())).weak());
                ui.separator();

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for group_index in self.scene.group_indices() {
                        let member_count = self
                            .scene
                            .objects
                            .iter()
                            .filter(|object| object.group_index == group_index)
                            .count();
                        let header = format!("{} ({member_count})", group_label(group_index));
                        egui::CollapsingHeader::new(header)
                            .id_salt(self.id.with(("object_group", group_index)))
                            .default_open(true)
                            .show(ui, |ui| self.show_outliner_group(ui, group_index, resources));
                    }
                });
            },
        );
        self.viewport.selection_changed_in_viewport = false;
    }

    /// Draws one row per object in `group_index`.
    fn show_outliner_group(
        &mut self,
        ui: &mut egui::Ui,
        group_index: usize,
        resources: &EditorResources<'_>,
    ) {
        for (index, object) in self.scene.objects.iter_mut().enumerate() {
            if object.group_index != group_index {
                continue;
            }
            ui.horizontal(|ui| {
                let mut visible = object.visible;
                if ui.checkbox(&mut visible, "").on_hover_text("Visible").changed() {
                    self.history.record(SceneEdit::Replace { index, object: object.clone() });
                    object.visible = visible;
                }
                let is_selected = self.selected_object == Some(index);
                let name = object_display_name(object, resources.custom_names, resources.catalog);
                let label = format!("{name}  ·  {}", object.kind_label());
                let response = ui.selectable_label(is_selected, label);
                if response.clicked() {
                    self.selected_object = Some(index);
                }
                if is_selected && self.viewport.selection_changed_in_viewport {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
            });
        }
    }

    /// Draws the properties of the selected object and applies any edits.
    pub(super) fn show_properties(&mut self, context: &egui::Context, resources: &mut EditorResources<'_>) {
        egui::SidePanel::right(self.id.with("properties")).resizable(true).default_width(280.0).show(
            context,
            |ui| {
                ui.heading("Properties");
                ui.separator();

                let selected = self
                    .selected_object
                    .and_then(|index| self.scene.objects.get(index).cloned().map(|object| (index, object)));
                let Some((index, original)) = selected else {
                    ui.label(
                        egui::RichText::new(
                            "Nothing selected. Click an object in the viewport or the outliner.",
                        )
                        .weak(),
                    );
                    self.property_edit_session = None;
                    return;
                };

                let mut edited = original.clone();
                let is_interacting = draw_object_fields(ui, &mut edited, resources);

                if edited != original {
                    // One undo step per continuous interaction, not one per frame of dragging.
                    if self.property_edit_session != Some(index) {
                        self.history.record(SceneEdit::Replace { index, object: original });
                        self.property_edit_session = Some(index);
                    }
                    self.scene.objects[index] = edited;
                }
                if !is_interacting {
                    self.property_edit_session = None;
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Frame").on_hover_text("F").clicked() {
                        self.focus_selection(resources.library);
                    }
                    if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
                        self.duplicate_selection();
                    }
                    if ui.button("Delete").on_hover_text("Del").clicked() {
                        self.delete_selection();
                    }
                });
            },
        );
    }
}

/// Name to list `object` under.
///
/// Models that still have their default name ("Model 55000") are listed by
/// the model's display name instead: the user's name for the model, or the
/// category worked out from the game data ("Door 55000").
pub fn object_display_name(
    object: &SceneObject,
    custom_names: &CustomNames,
    catalog: &AssetCatalog,
) -> String {
    match object.kind {
        SceneObjectKind::Model { model_id, .. } if object.has_default_model_name() => {
            custom_names.model_label(model_id, catalog)
        }
        _ => object.name.clone(),
    }
}

/// Draws editable fields for `object`. Returns `true` while any field is being dragged or typed in.
///
/// For models this also shows the model's description and a field naming the
/// model itself, which renames every placement of it rather than just this object.
fn draw_object_fields(
    ui: &mut egui::Ui,
    object: &mut SceneObject,
    resources: &mut EditorResources<'_>,
) -> bool {
    let mut responses: Vec<egui::Response> = Vec::new();
    let library = &mut *resources.library;

    egui::Grid::new("object_properties").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Name").on_hover_text("Name of this object only");
        responses.push(ui.text_edit_singleline(&mut object.name));
        ui.end_row();

        ui.label("Type");
        ui.label(object.kind_label());
        ui.end_row();

        ui.label("Group");
        ui.label(group_label(object.group_index));
        ui.end_row();

        ui.label("Visible");
        responses.push(ui.checkbox(&mut object.visible, ""));
        ui.end_row();

        match &mut object.kind {
            SceneObjectKind::Model { model_id, has_action } => {
                let model_id = *model_id;
                ui.label("Model id");
                ui.label(model_id.to_string());
                ui.end_row();

                ui.label("Model name").on_hover_text(
                    "Your name for this model, shown for every placement of it. Used only in the editor.",
                );
                let mut model_name = resources.custom_names.model_name_for_editing(model_id);
                let field = egui::TextEdit::singleline(&mut model_name).hint_text("Name this model");
                if ui.add(field).changed() {
                    resources.custom_names.set_model_name(model_id, &model_name);
                }
                ui.end_row();

                if let Some(description) = resources.catalog.model_description(model_id) {
                    ui.label("Looks like").on_hover_text(
                        "Worked out from the game data: door textures, actions, size and texture set names",
                    );
                    ui.add(egui::Label::new(description.summary()).wrap());
                    ui.end_row();
                }
                if let Some(usage) = resources.catalog.room_model(model_id) {
                    ui.label("Used in");
                    ui.label(format!("{} rooms, {} placements", usage.room_count, usage.placement_count));
                    ui.end_row();
                }
                if *has_action {
                    ui.label("Action");
                    ui.label("Has an action record");
                    ui.end_row();
                }
                if let Some(mesh) = library.model(model_id) {
                    ui.label("Triangles");
                    ui.label(mesh.triangle_count().to_string());
                    ui.end_row();
                    ui.label("Textures");
                    ui.vertical(|ui| {
                        for submesh in &mesh.submeshes {
                            let texture = submesh.texture;
                            match library.texture_archive_name(texture.archive) {
                                Some(set_name) => ui.label(format!("{texture}  {set_name}")),
                                None => ui.label(texture.to_string()),
                            };
                        }
                    });
                    ui.end_row();
                }
            }
            SceneObjectKind::Flat { texture } => {
                ui.label("Texture");
                ui.label(texture.to_string());
                ui.end_row();
            }
            SceneObjectKind::Light { radius } => {
                ui.label("Radius");
                responses
                    .push(ui.add(egui::DragValue::new(radius).speed(0.05).range(0.0..=200.0).suffix(" m")));
                ui.end_row();
            }
        }
    });

    ui.add_space(8.0);
    ui.strong("Position (metres)");
    responses.extend(vector_fields(ui, "position", &mut object.position, POSITION_DRAG_SPEED, ""));
    let native = world_to_native(object.position);
    ui.label(egui::RichText::new(format!("Native units: {}, {}, {}", native.x, native.y, native.z)).weak());

    if matches!(object.kind, SceneObjectKind::Model { .. }) {
        ui.add_space(8.0);
        ui.strong("Rotation (degrees)");
        responses.extend(vector_fields(
            ui,
            "rotation",
            &mut object.rotation_degrees,
            ROTATION_DRAG_SPEED,
            "°",
        ));
    }

    responses.iter().any(|response| response.dragged() || response.has_focus())
}

/// Draws three labelled drag fields for the X, Y and Z components of `value`.
fn vector_fields(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut glam::Vec3,
    speed: f32,
    suffix: &str,
) -> Vec<egui::Response> {
    let axis_colors = [
        egui::Color32::from_rgb(220, 80, 80),
        egui::Color32::from_rgb(110, 200, 90),
        egui::Color32::from_rgb(90, 140, 230),
    ];
    let mut responses = Vec::with_capacity(3);
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            for ((label, component), color) in
                ["X", "Y", "Z"].into_iter().zip(value.as_mut()).zip(axis_colors)
            {
                ui.colored_label(color, label);
                responses.push(
                    ui.add(egui::DragValue::new(component).speed(speed).max_decimals(3).suffix(suffix)),
                );
            }
        });
    });
    responses
}
