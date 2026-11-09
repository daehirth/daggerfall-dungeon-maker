//! The outliner (object list) and properties panels of the room editor.
//!
//! Both panels show the user's own names: the room's name above the
//! outliner, and model names in the outliner rows and properties. Names are
//! edited here too and apply everywhere the room or model appears.

use eframe::egui;

use super::history::SceneEdit;
use super::RoomEditor;
use crate::app::room_browser::{category_description, show_room_name_field};
use crate::app::viewport_tools::EditorResources;
use crate::arena2::{world_to_native, RDB_BLOCK_SIDE, WORLD_SCALE};
use crate::catalog::{flat_description, AssetCatalog};
use crate::custom_names::CustomNames;
use crate::encounters::{EncounterTable, Enemy};
use crate::scene::{
    action_type_name, axis_name, group_label, trigger_name, EnemyGender, FixedEnemy, FlatData, ObjectAction,
    ObjectId, SceneObject, SceneObjectKind, ACTION_TYPES, ADDED_OBJECTS_GROUP, FIXED_ENEMY_MARKER_RECORD,
    NO_ENEMY_ID, OUTSIDE_CELL_TOLERANCE, RANDOM_ENEMY_MARKER_RECORD, STARTING_LOCKS, START_MARKER_RECORD,
    TRIGGER_TYPES,
};

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
                        let source = match self.scene.block_name.as_str() {
                            "" => "New cell".to_string(),
                            block_name => format!("Copy of {block_name}"),
                        };
                        ui.label(egui::RichText::new(source).weak());
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
        let mut menu_choice = None;
        let mut clicked = None;
        let selected = self.selected_indices();
        let shift_held = ui.input(|input| input.modifiers.shift);
        for (index, object) in self.scene.objects.iter().enumerate() {
            if object.group_index != group_index {
                continue;
            }
            ui.horizontal(|ui| {
                let mut visible = object.visible;
                if ui.checkbox(&mut visible, "").on_hover_text("Visible").changed() {
                    visibility_change = Some((index, visible));
                }
                let is_selected = selected.contains(&index);
                let name = object_display_name(object, resources.custom_names, resources.catalog);
                let outside = self.is_outside_cell(index);
                let label = format!("{}{name}  ·  {}", if outside { "⚠ " } else { "" }, object.kind_label());
                let mut response = ui.selectable_label(is_selected, label);
                if outside {
                    response = response.on_hover_text(super::outside_cell_warning(self.outside_cell[index]));
                }
                // Right-clicking a row selects it too, like the viewport does, unless
                // several objects are selected: they can only be moved, so they have no menu.
                if response.clicked() || (response.secondary_clicked() && selected.len() <= 1) {
                    clicked = Some(index);
                }
                if is_selected && self.viewport.selection_changed_in_viewport {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
                response.context_menu(|ui| {
                    if selected.len() > 1 && is_selected {
                        ui.label("Several objects are selected; they can only be moved");
                        return;
                    }
                    if let Some(info) = self.object_menu_info(index, resources) {
                        if let Some(choice) = super::object_menu(ui, &info) {
                            menu_choice = Some((choice, index));
                        }
                    }
                });
            });
        }
        if let Some(index) = clicked {
            // Shift+click adds a row to the selection, or takes it out.
            if shift_held {
                self.toggle_selected(index);
            } else {
                self.select_objects(vec![index]);
            }
        }
        if let Some((choice, index)) = menu_choice {
            self.apply_object_menu(choice, index, resources);
        }
        if let Some((index, visible)) = visibility_change {
            if self.can_edit() {
                let object = &mut self.scene.objects[index];
                self.history.record(SceneEdit::Replace { index, object: object.clone() });
                object.visible = visible;
            }
        }
    }

    /// With several objects selected: lists them, and explains they can only be moved.
    fn show_multiple_selection(&mut self, ui: &mut egui::Ui, resources: &EditorResources<'_>) {
        let selected = self.selected_indices();
        ui.strong(format!("{} objects selected", selected.len())).on_hover_text(
            "Shift+click objects, or drag a box around them, to select several. Several objects can \
             only be moved: drag the arrows to move them together.",
        );
        ui.label(
            egui::RichText::new("Drag the arrows to move them together. Click one object to edit it.").weak(),
        );
        ui.add_space(6.0);
        let mut select_only = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                for &index in &selected {
                    let Some(object) = self.scene.objects.get(index) else { continue };
                    let name = object_display_name(object, resources.custom_names, resources.catalog);
                    let label = format!("{name}  ·  {}", object.kind_label());
                    if ui
                        .selectable_label(false, label)
                        .on_hover_text("Click to select only this object")
                        .clicked()
                    {
                        select_only = Some(index);
                    }
                }
            });
        if let Some(index) = select_only {
            self.select_objects(vec![index]);
            self.viewport.selection_changed_in_viewport = true;
        }
    }

    /// Draws the properties of the selected object and applies any edits.
    pub(super) fn show_properties(&mut self, context: &egui::Context, resources: &mut EditorResources<'_>) {
        egui::SidePanel::right(self.id.with("properties")).resizable(true).default_width(280.0).show(
            context,
            |ui| {
                self.show_used_in(ui);
                ui.heading("Properties");
                ui.separator();

                if self.has_multiple_selection() {
                    self.show_multiple_selection(ui, resources);
                    return;
                }
                let selected = self
                    .selected_object
                    .and_then(|index| self.scene.objects.get(index).cloned().map(|object| (index, object)));
                let Some((index, original)) = selected else {
                    self.show_cell_properties(ui, resources);
                    return;
                };

                let is_random_enemy = original.is_random_enemy_marker();
                let targets: Vec<LinkTarget> = self
                    .scene
                    .objects
                    .iter()
                    .map(|object| LinkTarget {
                        id: object.id,
                        label: format!(
                            "{} · {}",
                            object_display_name(object, resources.custom_names, resources.catalog),
                            object.kind_label()
                        ),
                    })
                    .collect();
                let can_preview = !self.preview_motions(index, resources.library).is_empty();
                let mut preview_clicked = false;
                let mut edited = original.clone();
                let mut go_to = None;
                let mut is_interacting = false;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(ui.available_height() - 40.0)
                    .show(ui, |ui| {
                        is_interacting = draw_object_fields(ui, &mut edited, resources);
                        if edited.is_marker() {
                            ui.add_space(8.0);
                            is_interacting |= show_marker(ui, self.id, &mut edited);
                        }
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
                        ui.add_space(8.0);
                        let (responses, link_go_to, preview) =
                            draw_game_settings(ui, self.id, &mut edited, &targets, can_preview);
                        preview_clicked = preview;
                        is_interacting |=
                            responses.iter().any(|response| response.dragged() || response.has_focus());
                        go_to = link_go_to;
                    });

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
                if let Some(target) = go_to {
                    if let Some(target_index) =
                        self.scene.objects.iter().position(|object| object.id == target)
                    {
                        self.selected_object = Some(target_index);
                        self.viewport.selection_changed_in_viewport = true;
                        self.focus_selection(resources.library);
                        if let Some(description) = self.describe_object_for_status(target_index, resources) {
                            self.status_message = format!("Selected {description}, the linked object");
                        }
                    }
                }
                if preview_clicked {
                    let now = ui.input(|input| input.time);
                    self.start_preview(index, now, resources.library);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Frame")
                        .on_hover_text("Moves the camera to show the selected object (F)")
                        .clicked()
                    {
                        self.focus_selection(resources.library);
                    }
                    if ui
                        .button("Duplicate")
                        .on_hover_text("Adds a copy of the object next to it, and selects the copy (Ctrl+D)")
                        .clicked()
                    {
                        self.duplicate_selection();
                    }
                    if ui
                        .button("Delete")
                        .on_hover_text("Removes the object from the cell; Ctrl+Z brings it back (Del)")
                        .clicked()
                    {
                        self.delete_selection();
                    }
                });
            },
        );
    }
}

impl RoomEditor {
    /// Lists the dungeons that use this cell; double-clicking one asks to open it.
    fn show_used_in(&mut self, ui: &mut egui::Ui) {
        egui::TopBottomPanel::bottom(self.id.with("used_in"))
            .resizable(true)
            .default_height(150.0)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.strong(format!("Used in ({})", self.used_in.len())).on_hover_text(
                    "Every dungeon that uses this cell. Click one to see its other cells faded around this \
                     one; double-click to open it in the dungeon view.",
                );
                if self.used_in.is_empty() {
                    ui.label(egui::RichText::new("Not used in any dungeon").weak());
                    return;
                }
                let mut selection = None;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for entry in &self.used_in {
                        let is_selected = self.selected_dungeon.as_ref() == Some(&entry.item);
                        let response = ui.selectable_label(is_selected, &entry.label).on_hover_text(
                            "Click to see the dungeon's other cells faded around this one (click again to \
                             hide them). Double-click to open it in the dungeon view.",
                        );
                        if is_selected && self.scroll_to_selected_dungeon {
                            response.scroll_to_me(Some(egui::Align::Center));
                            self.scroll_to_selected_dungeon = false;
                        }
                        if response.double_clicked() {
                            self.open_request = Some(entry.item.clone());
                        } else if response.clicked() {
                            selection = Some((!is_selected).then(|| entry.item.clone()));
                        }
                    }
                });
                if let Some(selection) = selection {
                    self.select_used_in(selection);
                    // Chosen by the user, who can already see the row.
                    self.scroll_to_selected_dungeon = false;
                }
            });
    }

    /// Shows the cell's own properties, when no object is selected. The water
    /// level and castle flag live on the cell's start marker and are edited there.
    fn show_cell_properties(&mut self, ui: &mut egui::Ui, resources: &mut EditorResources<'_>) {
        ui.strong("Cell").on_hover_text(
            "The cell (RDB block) being edited. Select an object to see the object's own properties instead.",
        );
        ui.label(egui::RichText::new("Select an object to see its properties.").weak());
        ui.add_space(4.0);
        let block_name = self.scene.block_name.clone();
        let start_marker = self.scene.start_marker_index();
        let mut edited_marker = start_marker.map(|index| self.scene.objects[index].clone());
        let mut is_interacting = false;
        let outside: Vec<usize> =
            (0..self.scene.objects.len()).filter(|&index| self.is_outside_cell(index)).collect();
        let mut select_outside = false;
        egui::Grid::new(self.id.with("cell_properties")).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Name").on_hover_text("The cell's name: your copy's name, or Daggerfall's block name.");
            ui.label(self.project_room_name.clone().unwrap_or_else(|| block_name.clone()));
            ui.end_row();

            // A new cell is not based on a Daggerfall block, so it has no category.
            if !block_name.is_empty() {
                ui.label("Category").on_hover_text("Kind of cell, from the first letter of its block name: N interior, W interior with water, M interior variant, S special (quest), B border cap.");
                let letter = block_name.chars().next().unwrap_or('?');
                ui.label(format!("{letter} · {}", category_description(&block_name)));
                ui.end_row();
            }

            ui.label("Source").on_hover_text("Where the cell comes from: Daggerfall's own data (read-only), or a copy in your project.");
            match (self.read_only, self.project_room_name.is_some()) {
                (true, _) => ui.label("Daggerfall (read-only)"),
                (false, true) if block_name.is_empty() => ui.label("New cell in your project"),
                (false, true) => ui.label(format!("Copy of {block_name}")),
                (false, false) => ui.label("Daggerfall"),
            };
            ui.end_row();

            ui.label("Size").on_hover_text("Every cell covers a fixed 51.2 × 51.2 m square of the dungeon grid; objects can reach past it.");
            ui.label(format!("{RDB_BLOCK_SIDE} × {RDB_BLOCK_SIDE} m"));
            ui.end_row();

            ui.label("Objects").on_hover_text("Number of objects in the cell: models, flats, markers and lights.");
            ui.label(self.scene.objects.len().to_string());
            ui.end_row();

            ui.label("Outside").on_hover_text(
                "Objects that reach outside the cell's 51.2 m square. In a dungeon they can overlap the \
                 neighbouring cells. They are marked ⚠ in the outliner.",
            );
            if outside.is_empty() {
                ui.label("None");
            } else {
                ui.horizontal(|ui| {
                    ui.colored_label(WARNING_COLOR, format!("⚠ {} object{}", outside.len(), if outside.len() == 1 { "" } else { "s" }));
                    if ui.small_button("Select them").on_hover_text("Selects every object outside the square").clicked() {
                        select_outside = true;
                    }
                });
            }
            ui.end_row();

            ui.label("Object lists").on_hover_text("The groups in the outliner, roughly one per room");
            let lists =
                self.scene.group_indices().into_iter().filter(|group| *group != ADDED_OBJECTS_GROUP).count();
            ui.label(lists.to_string());
            ui.end_row();

            let Some(SceneObjectKind::Flat { data, .. }) =
                edited_marker.as_mut().map(|marker| &mut marker.kind)
            else {
                ui.label("Water level").on_hover_text("The water level is stored on the cell's start marker, which this cell does not have.");
                ui.label(egui::RichText::new("No start marker to hold it").weak());
                ui.end_row();
                return;
            };
            is_interacting |= start_marker_rows(ui, data);
        });
        if select_outside {
            self.select_objects(outside.clone());
            self.viewport.selection_changed_in_viewport = true;
        }

        // Jumping to the player's start and to the way out.
        ui.add_space(8.0);
        let exit_door = self.scene.objects.iter().position(SceneObject::is_exit_door);
        let mut go_to = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(start_marker.is_some(), egui::Button::new("Go to Start"))
                .on_hover_text(
                    "Selects the cell's start marker, where the player enters, and moves the camera to it",
                )
                .on_disabled_hover_text("This cell has no start marker")
                .clicked()
            {
                go_to = start_marker.map(|index| (index, "the start marker"));
            }
            if ui
                .add_enabled(exit_door.is_some(), egui::Button::new("Go to Exit"))
                .on_hover_text(
                    "Selects the cell's exit door, which leaves the dungeon, and moves the camera to it",
                )
                .on_disabled_hover_text("This cell has no exit door; only a dungeon's start cell has one")
                .clicked()
            {
                go_to = exit_door.map(|index| (index, "the exit door"));
            }
        });
        if let Some((index, what)) = go_to {
            self.select_objects(vec![index]);
            self.viewport.selection_changed_in_viewport = true;
            self.focus_selection(resources.library);
            self.status_message = format!("Selected {what}");
        }

        if let (Some(index), Some(marker)) = (start_marker, edited_marker) {
            if marker != self.scene.objects[index] && self.can_edit() {
                // One undo step per continuous change, as for object properties.
                if self.property_edit_session != Some(index) {
                    self.history
                        .record(SceneEdit::Replace { index, object: self.scene.objects[index].clone() });
                    self.property_edit_session = Some(index);
                }
                self.scene.objects[index] = marker;
            }
        }
        if !is_interacting {
            self.property_edit_session = None;
        }
    }

    /// Lists the enemies a random enemy marker can become, with their chances.
    fn show_possible_enemies(&mut self, ui: &mut egui::Ui, object_index: usize) {
        ui.strong("Possible enemies")
            .on_hover_text("The enemies the game can place at this random enemy marker, with their chances.");
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
    ui.strong("Spawns").on_hover_text("The enemy this fixed enemy marker places in the game.");
    egui::Grid::new(id.with("fixed_enemy")).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Enemy")
            .on_hover_text("Which enemy appears here. Nothing switches the marker off without deleting it.");
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
        ui.label("Behaviour")
            .on_hover_text("Hostile enemies attack on sight; passive ones only attack when provoked.");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut changed.passive, false, "Hostile");
            ui.selectable_value(&mut changed.passive, true, "Passive")
                .on_hover_text("Attacks only when provoked");
        });
        ui.end_row();
        if fixed_enemy.human_gender().is_some() {
            ui.label("Gender").on_hover_text("Gender of a human enemy; Either lets the game choose.");
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

/// Adds the start marker's rows to a grid: the cell's water level (the
/// marker's sound byte, 8 game units per step) and castle flag (its magnitude
/// byte). Returns `true` while the water level is being dragged or typed in.
fn start_marker_rows(ui: &mut egui::Ui, data: &mut FlatData) -> bool {
    let mut is_interacting = false;
    let [mut castle, mut water_steps] = data.faction_or_mobile_id.to_le_bytes();

    ui.label("Water").on_hover_text(
        "Stored on the cell's start marker. Random enemy markers below it use the underwater table.",
    );
    ui.horizontal(|ui| {
        let mut has_water = water_steps != 0;
        if ui.checkbox(&mut has_water, "").changed() {
            water_steps = u8::from(has_water);
        }
        if has_water {
            let field = egui::DragValue::new(&mut water_steps)
                .range(1..=255)
                .custom_formatter(|steps, _| format!("{:.1} m", steps * 8.0 * f64::from(WORLD_SCALE)))
                .custom_parser(|text| {
                    let metres: f64 = text.trim().trim_end_matches('m').trim().parse().ok()?;
                    Some((metres / (8.0 * f64::from(WORLD_SCALE))).round())
                });
            let response = ui.add(field);
            is_interacting = response.dragged() || response.has_focus();
        } else {
            ui.label("None");
        }
    });
    ui.end_row();

    ui.label("Castle cell").on_hover_text("Marks a main-story castle cell; stored on the start marker");
    let mut is_castle = castle != 0;
    if ui.checkbox(&mut is_castle, "").changed() {
        castle = u8::from(is_castle);
    }
    ui.end_row();
    data.faction_or_mobile_id = u16::from_le_bytes([castle, water_steps]);
    is_interacting
}

/// Shows what kind of marker `object` is and every value the game reads from
/// it, by meaning. The same bytes appear by number under Game settings.
/// Returns `true` while a field is being dragged or typed in.
fn show_marker(ui: &mut egui::Ui, id: egui::Id, object: &mut SceneObject) -> bool {
    let SceneObjectKind::Flat { texture, data } = &mut object.kind else { return false };
    let record = texture.record;
    let mut is_interacting = false;
    ui.strong("Marker").on_hover_text("Markers are invisible in the game. They tell it where to start the player and where to place enemies, treasure and quest items.");
    egui::Grid::new(id.with("marker")).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Kind").on_hover_text("What the game uses this marker for.");
        ui.label(flat_description(*texture));
        ui.end_row();
        match record {
            START_MARKER_RECORD => is_interacting = start_marker_rows(ui, data),
            RANDOM_ENEMY_MARKER_RECORD => {
                ui.label("Slot").on_hover_text(
                    "Random enemy markers in a dungeon that share a slot get the same enemy. \
                     0 lets the game pick a slot from 1 to 6 each time.",
                );
                ui.horizontal(|ui| {
                    let response = ui.add(egui::DragValue::new(&mut data.flags).range(0..=255));
                    is_interacting |= response.dragged() || response.has_focus();
                    if data.flags == 0 {
                        ui.label(egui::RichText::new("random").weak());
                    }
                });
                ui.end_row();
                is_interacting |= spawn_distance_row(ui, data);
            }
            FIXED_ENEMY_MARKER_RECORD => {
                is_interacting |= spawn_distance_row(ui, data);
            }
            _ => {
                let meaning = match record {
                    8 => "None. It marks where the player arrives when entering the dungeon.",
                    11 => "None. The quest system picks quest markers by their position.",
                    17 => "None. It is where the player lands when an object whose action is Teleport, and                            which is tied to this marker, is triggered.",
                    18 => "None. The quest system places quest items at these markers, by position.",
                    19 => "None. The game puts a loot pile here, chosen by the dungeon's type.",
                    _ => "None. Daggerfall Unity reads nothing from this marker.",
                };
                ui.label("Data").on_hover_text(
                    "Values the game reads from this marker. Like any flat, a marker can also have an action \
                     and trigger another object (see Game settings).",
                );
                ui.add(egui::Label::new(egui::RichText::new(meaning).weak()).wrap());
                ui.end_row();
            }
        }
    });
    is_interacting
}

/// Adds the enemy markers' spawn distance row (the marker's sound byte).
/// Returns `true` while it is being dragged or typed in.
fn spawn_distance_row(ui: &mut egui::Ui, data: &mut FlatData) -> bool {
    let [low, mut spawn_distance] = data.faction_or_mobile_id.to_le_bytes();
    ui.label("Spawn distance").on_hover_text(
        "Daggerfall's spawn distance setting for this enemy, passed on by Daggerfall Unity when it \
         places the enemy. Its exact effect is not documented.",
    );
    let response = ui.add(egui::DragValue::new(&mut spawn_distance));
    ui.end_row();
    data.faction_or_mobile_id = u16::from_le_bytes([low, spawn_distance]);
    response.dragged() || response.has_focus()
}

/// Colour of warnings in the panels.
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 190, 70);

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

        ui.label("Type").on_hover_text("What the object is: a model, an action model (a model with an action), a flat (a sprite), a marker or a light.");
        ui.label(object.kind_label());
        ui.end_row();

        ui.label("Group").on_hover_text("The object list the object belongs to, roughly one per room. Objects you add go in Added objects.");
        ui.label(group_label(object.group_index));
        ui.end_row();

        ui.label("Visible").on_hover_text("Shows or hides the object in the editor's view only. Hidden objects are still saved and still in the game.");
        responses.push(ui.checkbox(&mut object.visible, ""));
        ui.end_row();

        match &mut object.kind {
            SceneObjectKind::Model { model_id, .. } => {
                let model_id = *model_id;
                ui.label("Model id").on_hover_text("Number of the 3D model in ARCH3D.BSA that this object shows.");
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
                    ui.label("Used in").on_hover_text("How many of Daggerfall's cells use this model, and how many times in total.");
                    ui.label(format!("{} cells, {} placements", usage.room_count, usage.placement_count));
                    ui.end_row();
                }
                if let Some(mesh) = library.model(model_id) {
                    ui.label("Triangles").on_hover_text("Number of triangles in the model's mesh.");
                    ui.label(mesh.triangle_count().to_string());
                    ui.end_row();
                    ui.label("Textures").on_hover_text("The texture images the model is painted with: archive, record and texture set name.");
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
                ui.label("Texture").on_hover_text("The image the flat shows: its TEXTURE.### archive and record.");
                ui.label(texture.to_string());
                ui.end_row();
            }
            SceneObjectKind::Light { radius, .. } => {
                ui.label("Radius").on_hover_text("How far the light reaches, in metres.");
                responses
                    .push(ui.add(egui::DragValue::new(radius).speed(0.05).range(0.0..=200.0).suffix(" m")));
                ui.end_row();
            }
        }
    });

    ui.add_space(8.0);
    ui.strong("Position (metres)").on_hover_text(
        "Where the object is in the cell, in metres. Native units are Daggerfall's own (40 per metre).",
    );
    responses.extend(vector_fields(ui, "position", &mut object.position, POSITION_DRAG_SPEED, ""));
    let native = world_to_native(object.position);
    ui.label(egui::RichText::new(format!("Native units: {}, {}, {}", native.x, native.y, native.z)).weak());
    let outside = super::object_distance_outside_cell(object, library);
    if outside > OUTSIDE_CELL_TOLERANCE {
        ui.add(
            egui::Label::new(egui::RichText::new(super::outside_cell_warning(outside)).color(WARNING_COLOR))
                .wrap(),
        );
    }

    if matches!(object.kind, SceneObjectKind::Model { .. }) {
        ui.add_space(8.0);
        ui.strong("Rotation (degrees)")
            .on_hover_text("How far the object is turned around each axis, in degrees.");
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

/// One object in the cell, offered as the target of an action link.
struct LinkTarget {
    id: ObjectId,
    label: String,
}

/// Draws every setting Daggerfall stores with `object` beyond its name,
/// position and rotation. Settings the object does not use show their empty
/// value ("None", 0) so the full set is always visible.
///
/// Returns the fields' responses (to tell whether one is being dragged or
/// typed in) and, if the user asked to go to the linked object, its id.
fn draw_game_settings(
    ui: &mut egui::Ui,
    id: egui::Id,
    object: &mut SceneObject,
    targets: &[LinkTarget],
    can_preview: bool,
) -> (Vec<egui::Response>, Option<ObjectId>, bool) {
    let mut responses = Vec::new();
    let mut go_to = None;
    let object_id = object.id;
    // Markers reuse these bytes for their own data, shown in the Marker section.
    let is_marker = object.is_marker();
    ui.strong("Game settings")
        .on_hover_text("Every setting Daggerfall stores with this object, including ones it does not use.");
    ui.label(
        egui::RichText::new("Everything Daggerfall stores with this object; unused settings show None or 0.")
            .weak(),
    );
    ui.add_space(4.0);
    egui::Grid::new(id.with("game_settings")).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| match &mut object.kind {
        SceneObjectKind::Model { data, .. } => {
            ui.label("Trigger").on_hover_text("What sets the model's action off");
            let mut trigger = data.trigger();
            choice(ui, id.with("trigger"), trigger_name(trigger), &mut trigger, TRIGGER_TYPES.iter().map(|(value, name)| (*value, name.to_string())));
            data.set_trigger(trigger);
            ui.end_row();

            ui.label("Starting lock").on_hover_text(
                "Lock strength a door starts with. Only doors use it, and Daggerfall Unity reads the \
                 trigger only when this is Unlocked.",
            );
            let mut lock_index = data.lock_index();
            let lock_label = |index: u32| match STARTING_LOCKS.get(index as usize) {
                Some(0) => "Unlocked".to_string(),
                Some(strength) => format!("Lock {strength}"),
                None => format!("Unknown {index}"),
            };
            choice(ui, id.with("lock"), lock_label(lock_index), &mut lock_index, (0..STARTING_LOCKS.len() as u32).map(|index| (index, lock_label(index))));
            data.set_lock_index(lock_index);
            ui.end_row();

            ui.label("Sound").on_hover_text("Sound the action plays; 0 for none");
            responses.push(ui.add(egui::DragValue::new(&mut data.sound)));
            ui.end_row();

            ui.label("Action").on_hover_text("What the object does when triggered: move, rotate, open a door, teleport, hurt and so on. None if it has no action.");
            match &mut data.action {
                None => {
                    ui.horizontal(|ui| {
                        ui.label("None");
                        if ui.small_button("Add action").clicked() {
                            data.action = Some(ObjectAction::default());
                        }
                    });
                    ui.end_row();
                }
                Some(action) => {
                    let mut remove = false;
                    ui.horizontal(|ui| {
                        choice(ui, id.with("action_type"), action_type_name(action.action_type), &mut action.action_type, ACTION_TYPES.iter().map(|(value, name)| (*value, name.to_string())));
                        remove = ui.small_button("Remove").on_hover_text("Remove the action").clicked();
                    });
                    ui.end_row();

                    ui.label("Axis").on_hover_text("Direction of movement (1-6); some actions use it as an amount instead");
                    ui.horizontal(|ui| {
                        responses.push(ui.add(egui::DragValue::new(&mut action.axis)));
                        ui.label(egui::RichText::new(axis_name(action.axis)).weak());
                    });
                    ui.end_row();

                    ui.label("Duration").on_hover_text("How long the movement takes");
                    responses.push(ui.add(egui::DragValue::new(&mut action.duration)));
                    ui.end_row();

                    ui.label("Magnitude").on_hover_text("How far it moves or turns, or how strong the effect is");
                    responses.push(ui.add(egui::DragValue::new(&mut action.magnitude)));
                    ui.end_row();
                    if remove {
                        data.action = None;
                    }
                }
            }

            ui.label("Triggers next").on_hover_text("Object this one's action sets off next, such as the door a lever opens");
            if data.action.is_some() {
                go_to = link_choice(ui, id, object_id, &mut object.next, targets);
            } else {
                ui.label(egui::RichText::new("Needs an action").weak());
                object.next = None;
            }
            ui.end_row();
        }
        SceneObjectKind::Flat { data, .. } => {
            ui.label("Flags").on_hover_text(
                "Trigger type for flats with an action. Enemy markers use it for gender (fixed) or slot (random).",
            );
            ui.horizontal(|ui| {
                responses.push(ui.add(egui::DragValue::new(&mut data.flags)));
                let meaning = if is_marker {
                    "Marker data, see above".to_string()
                } else {
                    format!("Trigger: {}", trigger_name(u32::from(data.flags)))
                };
                ui.label(egui::RichText::new(meaning).weak());
            });
            ui.end_row();

            ui.label("Action").on_hover_text("What the flat does when triggered; fixed enemies use 99 for passive");
            choice(ui, id.with("flat_action"), action_type_name(data.action), &mut data.action, ACTION_TYPES.iter().map(|(value, name)| (*value, name.to_string())));
            ui.end_row();

            let [mut magnitude, mut sound] = data.faction_or_mobile_id.to_le_bytes();
            ui.label("Magnitude").on_hover_text("Strength of the flat's action; the enemy of a fixed enemy marker");
            responses.push(ui.add(egui::DragValue::new(&mut magnitude)));
            ui.end_row();
            ui.label("Sound").on_hover_text("Sound of the flat's action; the water level on a start marker");
            responses.push(ui.add(egui::DragValue::new(&mut sound)));
            ui.end_row();
            data.faction_or_mobile_id = u16::from_le_bytes([magnitude, sound]);

            ui.label("Faction").on_hover_text("Magnitude and sound read together, as a faction id for people flats");
            ui.label(data.faction_or_mobile_id.to_string());
            ui.end_row();

            ui.label("Triggers next").on_hover_text("Object this flat's action sets off next");
            go_to = link_choice(ui, id, object_id, &mut object.next, targets);
            ui.end_row();
        }
        SceneObjectKind::Light { data, .. } => {
            ui.label("Unknown 1").on_hover_text(
                "Stored with every light; its meaning is not known and Daggerfall Unity ignores it. In \
                 Daggerfall's cells it is a small number, most often 16, 25 or 14, so it may be a \
                 brightness, but that is unconfirmed.",
            );
            responses.push(ui.add(egui::DragValue::new(&mut data.unknown_1)));
            ui.end_row();
            ui.label("Unknown 2").on_hover_text(
                "Stored with every light; its meaning is not known and Daggerfall Unity ignores it. In \
                 Daggerfall's cells it is only ever 0 or 4294967294 (−2), so it may be a flag, but that \
                 is unconfirmed.",
            );
            responses.push(ui.add(egui::DragValue::new(&mut data.unknown_2)));
            ui.end_row();
        }
    });
    let mut preview = false;
    if can_preview {
        ui.add_space(4.0);
        preview = ui
            .button("Preview action")
            .on_hover_text(
                "Plays what happens when this object's action is triggered: it and every object it \
                 triggers move as in the game. After half a second everything goes back; the cell is \
                 not changed.",
            )
            .clicked();
    }
    (responses, go_to, preview)
}

/// Draws a drop-down choosing `value` from `options`, showing `selected_text`.
fn choice<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected_text: String,
    value: &mut T,
    options: impl Iterator<Item = (T, String)>,
) {
    egui::ComboBox::from_id_salt(id).selected_text(selected_text).width(170.0).height(320.0).show_ui(
        ui,
        |ui| {
            for (option, label) in options {
                ui.selectable_value(value, option, label);
            }
        },
    );
}

/// Draws the drop-down choosing which object `next` links to, and a button to go to it.
/// Returns the linked object's id if the button was clicked.
fn link_choice(
    ui: &mut egui::Ui,
    id: egui::Id,
    object_id: ObjectId,
    next: &mut Option<ObjectId>,
    targets: &[LinkTarget],
) -> Option<ObjectId> {
    let selected_text = match *next {
        None => "None".to_string(),
        Some(target) => targets
            .iter()
            .find(|candidate| candidate.id == target)
            .map_or_else(|| format!("Missing object {target}"), |candidate| candidate.label.clone()),
    };
    let mut go_to = None;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id.with("next"))
            .selected_text(selected_text)
            .width(170.0)
            .height(320.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(next, None, "None");
                for target in targets.iter().filter(|target| target.id != object_id) {
                    ui.selectable_value(next, Some(target.id), &target.label);
                }
            });
        if let Some(target) = *next {
            if ui.small_button("Go to").on_hover_text("Select the linked object").clicked() {
                go_to = Some(target);
            }
        }
    });
    go_to
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
