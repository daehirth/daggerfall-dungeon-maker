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
use crate::encounters::{EncounterTable, Enemy};
use crate::scene::{group_label, EnemyGender, FixedEnemy, SceneObject, SceneObjectKind, NO_ENEMY_ID};

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
                match &self.project_room_name {
                    Some(name) => {
                        ui.heading(name);
                        ui.label(egui::RichText::new(format!("Copy of {}", self.scene.block_name)).weak());
                    }
                    None => {
                        ui.heading(&self.scene.block_name);
                        show_room_name_field(ui, &self.scene.block_name, resources.custom_names);
                        if self.read_only {
                            ui.label(egui::RichText::new("From Daggerfall (read-only)").weak());
                        }
                    }
                }
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
        let mut visibility_change = None;
        for (index, object) in self.scene.objects.iter().enumerate() {
            if object.group_index != group_index {
                continue;
            }
            ui.horizontal(|ui| {
                let mut visible = object.visible;
                if ui.checkbox(&mut visible, "").on_hover_text("Visible").changed() {
                    visibility_change = Some((index, visible));
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
        if let Some((index, visible)) = visibility_change {
            if self.can_edit() {
                let object = &mut self.scene.objects[index];
                self.history.record(SceneEdit::Replace { index, object: object.clone() });
                object.visible = visible;
            }
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

                let is_random_enemy = original.is_random_enemy_marker();
                let mut edited = original.clone();
                let is_interacting = draw_object_fields(ui, &mut edited, resources);
                if is_random_enemy {
                    ui.add_space(8.0);
                    self.show_possible_enemies(ui, index);
                }
                // Edits here go through the same undo and read-only checks as the fields above.
                if let Some(fixed_enemy) = original.fixed_enemy() {
                    ui.add_space(8.0);
                    if let Some(changed) = show_fixed_enemy(ui, self.id, fixed_enemy) {
                        edited.set_fixed_enemy(changed);
                    }
                }

                if edited != original && self.can_edit() {
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

impl RoomEditor {
    /// Lists the enemies a random enemy marker can become, with their chances.
    fn show_possible_enemies(&mut self, ui: &mut egui::Ui, object_index: usize) {
        ui.strong("Possible enemies");
        let Some(table) = self.encounter_table(object_index) else {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "The game picks an enemy for this marker from the dungeon's type. Open this \
                         cell from a dungeon to see which enemies it can become.",
                    )
                    .weak(),
                )
                .wrap(),
            );
            return;
        };
        let situation = match table {
            EncounterTable::Underwater => "Under water (below this cell's water level)".to_string(),
            EncounterTable::Dungeon(dungeon_type) => format!("In a {}", dungeon_type.name()),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{situation} for a player of level"));
            let level_field =
                egui::DragValue::new(&mut self.enemy_preview_level).range(1..=super::MAX_PREVIEW_LEVEL);
            let response = ui.add(level_field).on_hover_text("Scroll, drag or type to change the level");
            if response.hovered() {
                self.enemy_preview_level = scroll_level(ui, self.enemy_preview_level);
            }
        });
        egui::Grid::new(self.id.with("possible_enemies")).num_columns(2).striped(true).show(ui, |ui| {
            for chance in self.possible_enemies(object_index).unwrap_or_default() {
                ui.label(chance.enemy.name());
                ui.label(percent(chance.chance));
                ui.end_row();
            }
        });
        if table == EncounterTable::Underwater {
            ui.label(egui::RichText::new("Underwater markers use the same table in every dungeon.").weak());
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "The game picks one enemy from these chances. Daggerfall's own method (Daggerfall \
                     Unity's default) fixes the picks for each dungeon and player level, so returning \
                     at the same level meets the same enemies.",
                )
                .weak(),
            )
            .wrap(),
        );
    }
}

/// Steps `level` by one for each frame the mouse wheel turns (up to raise it,
/// down to lower it), within 1 and the highest preview level. The scroll is
/// used up so the panel behind the field does not scroll as well.
fn scroll_level(ui: &egui::Ui, level: u32) -> u32 {
    let scroll = ui.input(|input| input.raw_scroll_delta.y);
    if scroll == 0.0 {
        return level;
    }
    ui.ctx().input_mut(|input| {
        input.raw_scroll_delta = egui::Vec2::ZERO;
        input.smooth_scroll_delta = egui::Vec2::ZERO;
    });
    let stepped = if scroll > 0.0 { level.saturating_add(1) } else { level.saturating_sub(1) };
    stepped.clamp(1, super::MAX_PREVIEW_LEVEL)
}

/// Shows what a fixed enemy marker spawns, with controls to change it.
/// Returns the new settings if the user changed any.
fn show_fixed_enemy(ui: &mut egui::Ui, id: egui::Id, fixed_enemy: FixedEnemy) -> Option<FixedEnemy> {
    let mut changed = fixed_enemy;
    ui.strong("Spawns");
    egui::Grid::new(id.with("fixed_enemy")).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Enemy");
        egui::ComboBox::from_id_salt(id.with("fixed_enemy_type"))
            .selected_text(fixed_enemy.description())
            .width(190.0)
            .height(360.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut changed.mobile_id, NO_ENEMY_ID, "Nothing (switch the marker off)");
                for (heading, humans) in [("Monsters", false), ("Classes", true)] {
                    ui.separator();
                    ui.label(egui::RichText::new(heading).weak());
                    for enemy in Enemy::all().filter(|enemy| enemy.can_spawn() && enemy.is_human() == humans)
                    {
                        ui.selectable_value(&mut changed.mobile_id, enemy.mobile_id() as u8, enemy.name());
                    }
                }
            });
        ui.end_row();
        if fixed_enemy.is_switched_off() {
            return;
        }
        ui.label("Behaviour");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut changed.passive, false, "Hostile");
            ui.selectable_value(&mut changed.passive, true, "Passive")
                .on_hover_text("Attacks only when provoked");
        });
        ui.end_row();
        if fixed_enemy.human_gender().is_some() {
            ui.label("Gender");
            ui.horizontal(|ui| {
                for gender in EnemyGender::ALL {
                    ui.selectable_value(&mut changed.gender, gender, gender.name());
                }
            });
            ui.end_row();
        }
    });
    if fixed_enemy.enemy().is_some_and(|enemy| enemy.lives_in_water()) {
        ui.label(egui::RichText::new("Only appears if this marker is under water.").weak());
    }
    ui.add(
        egui::Label::new(
            egui::RichText::new(
                "Stored in the cell, so it is the same in every dungeon that uses this cell.",
            )
            .weak(),
        )
        .wrap(),
    );
    (changed != fixed_enemy).then_some(changed)
}

/// Formats a chance between 0 and 1 as a percentage, keeping small chances visible.
pub fn percent(chance: f32) -> String {
    let percentage = chance * 100.0;
    if percentage < 10.0 {
        format!("{percentage:.1}%")
    } else {
        format!("{percentage:.0}%")
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
                    ui.label(format!("{} cells, {} placements", usage.room_count, usage.placement_count));
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
            SceneObjectKind::Flat { texture, .. } => {
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
