//! A window listing every entrance a dungeon can have, with a 3D preview.
//!
//! An entrance is the above-ground part of a location: a grid of RMB blocks
//! with a dungeon door somewhere in it. Daggerfall's 4,000-odd dungeons share
//! about 140 of them, so the window groups the dungeons by entrance. The
//! selected entrance is drawn on a patch of ground, with its dungeon door
//! highlighted.

use std::collections::HashMap;

use eframe::{egui, egui_wgpu};
use glam::Vec3;

use super::viewport_tools::{handle_camera_input, paint_scene, size_in_pixels, EditorResources};
use crate::arena2::{
    Arena2Library, DoorKind, DungeonDefinition, LocationExterior, TextureKey, RMB_BLOCK_SIDE,
};
use crate::camera::OrbitCamera;
use crate::project::DungeonReference;
use crate::render::{FrameRequest, ViewSettings, ViewportTarget};
use crate::scene::{FlatData, ModelData, Scene, SceneObject, SceneObjectKind};

/// Colour of warnings.
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 190, 70);
/// Colour of the "in use" note.
const GOOD_COLOR: egui::Color32 = egui::Color32::from_rgb(120, 230, 140);
/// Most dungeon names listed under the preview.
const LISTED_DUNGEONS: usize = 12;
/// Most block names listed for an entrance in the list.
const LISTED_BLOCKS_IN_LIST: usize = 3;
/// Most block names listed for the selected entrance.
const LISTED_BLOCKS_IN_DETAILS: usize = 12;

/// Names for the kinds of block an entrance is made of, by the block name's first four letters.
const BLOCK_KINDS: [(&str, &str); 14] = [
    ("DUNG", "Dungeon entrance"),
    ("RUIN", "Ruins"),
    ("CAST", "Castle"),
    ("GRVE", "Graveyard"),
    ("MANR", "Manor"),
    ("SHCK", "Shack"),
    ("FARM", "Farm"),
    ("SHRI", "Shrine"),
    ("TEMP", "Temple"),
    ("PALA", "Palace"),
    ("WITC", "Coven"),
    ("CUST", "Special"),
    ("WALL", "Town walls"),
    ("RESI", "Houses"),
];

/// One entrance: a grid of blocks, and the Daggerfall dungeons that have it.
pub struct EntranceDesign {
    pub exterior: LocationExterior,
    /// Indices into the list of Daggerfall dungeons.
    pub dungeons: Vec<usize>,
    /// What it is, for example "Castle".
    pub kind: &'static str,
}

impl EntranceDesign {
    /// Up to `limit` of the different block names, without `.RMB`, for
    /// example "CASTAA14, RESIAA03 and 5 more".
    fn block_list(&self, limit: usize) -> String {
        let mut names: Vec<&str> = Vec::new();
        for name in &self.exterior.block_names {
            let name = name.trim_end_matches(".RMB");
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let more = names.len().saturating_sub(limit);
        names.truncate(limit);
        match more {
            0 => names.join(", "),
            _ => format!("{} and {more} more", names.join(", ")),
        }
    }
}

/// An exterior's width, height and blocks.
type ExteriorLayout<'a> = (usize, usize, &'a [String]);

/// Groups `dungeons` by entrance, sorted by kind and then by how many dungeons share each.
pub fn entrance_designs(dungeons: &[DungeonDefinition]) -> Vec<EntranceDesign> {
    // Grouped by their blocks: locations with the same blocks look the same,
    // whatever their buildings are called.
    let mut by_exterior: HashMap<ExteriorLayout<'_>, (&LocationExterior, Vec<usize>)> = HashMap::new();
    for (index, dungeon) in dungeons.iter().enumerate() {
        if let Some(exterior) = &dungeon.exterior {
            let layout = (exterior.width, exterior.height, exterior.block_names.as_slice());
            by_exterior.entry(layout).or_insert_with(|| (exterior, Vec::new())).1.push(index);
        }
    }
    let mut designs: Vec<EntranceDesign> = by_exterior
        .into_values()
        .map(|(exterior, dungeons)| EntranceDesign {
            kind: entrance_kind(exterior),
            exterior: exterior.clone(),
            dungeons,
        })
        .collect();
    designs.sort_by(|first, second| {
        (first.kind, std::cmp::Reverse(first.dungeons.len()), &first.exterior.block_names).cmp(&(
            second.kind,
            std::cmp::Reverse(second.dungeons.len()),
            &second.exterior.block_names,
        ))
    });
    designs
}

/// Names the entrance `reference` gives a dungeon by what it is, for example
/// "Ruins · RUINAA08", or by the dungeon it comes from if that is not found.
pub fn entrance_label(reference: &DungeonReference, dungeons: &[DungeonDefinition]) -> String {
    let exterior =
        dungeons.iter().find(|dungeon| reference.is(dungeon)).and_then(|dungeon| dungeon.exterior.as_ref());
    match exterior {
        Some(exterior) => {
            let design = EntranceDesign {
                kind: entrance_kind(exterior),
                exterior: exterior.clone(),
                dungeons: Vec::new(),
            };
            format!("{} · {}", design.kind, design.block_list(1))
        }
        None => reference.label(),
    }
}

/// What an entrance is: the kind of its most telling block.
fn entrance_kind(exterior: &LocationExterior) -> &'static str {
    BLOCK_KINDS
        .iter()
        .find(|(prefix, _)| exterior.block_names.iter().any(|name| name.starts_with(prefix)))
        .map_or("Other", |(_, kind)| kind)
}

/// What the window needs from the dungeon being edited.
pub struct EntranceRequest<'request> {
    /// Name of the dungeon, for the title.
    pub dungeon_name: &'request str,
    pub daggerfall_dungeons: &'request [DungeonDefinition],
    /// The entrance the dungeon has now.
    pub current: Option<&'request DungeonReference>,
    /// Set for Daggerfall's own dungeons, whose entrance can only be looked at.
    pub read_only: bool,
}

/// The selected entrance, ready to draw.
struct Preview {
    design_index: usize,
    scene: Scene,
    ground: Vec<(Vec3, f32)>,
    /// Indices of the models with a dungeon door.
    entrance_doors: Vec<usize>,
    /// Blocks that could not be loaded.
    problems: Vec<String>,
}

/// State of the entrance window.
pub struct EntrancePicker {
    open: bool,
    search: String,
    /// Built from the dungeon list the first time the window opens.
    designs: Vec<EntranceDesign>,
    selected: Option<usize>,
    /// Scroll the list to the selected entrance on the next frame.
    scroll_to_selected: bool,
    preview: Option<Preview>,
    camera: OrbitCamera,
    target: ViewportTarget,
    view_settings: ViewSettings,
}

impl Default for EntrancePicker {
    fn default() -> Self {
        Self {
            open: false,
            search: String::new(),
            designs: Vec::new(),
            selected: None,
            scroll_to_selected: false,
            preview: None,
            camera: OrbitCamera::default(),
            target: ViewportTarget::default(),
            view_settings: ViewSettings {
                show_grid: false,
                show_room_outline: false,
                ..ViewSettings::default()
            },
        }
    }
}

impl EntrancePicker {
    /// Opens the window with the dungeon's current entrance selected.
    pub fn open(&mut self) {
        self.open = true;
        self.selected = None;
        self.scroll_to_selected = true;
    }

    /// Frees the GPU image the preview renders into.
    pub fn release(&mut self, render_state: &egui_wgpu::RenderState) {
        self.target.release(render_state);
    }

    /// Draws the window, in its own OS window, if it is open. Returns the entrance the user chose.
    pub fn show(
        &mut self,
        context: &egui::Context,
        request: &EntranceRequest<'_>,
        resources: &mut EditorResources<'_>,
    ) -> Option<DungeonReference> {
        if !self.open {
            return None;
        }
        if self.designs.is_empty() {
            self.designs = entrance_designs(request.daggerfall_dungeons);
        }
        if self.selected.is_none() {
            self.selected = request
                .current
                .and_then(|current| {
                    let dungeon =
                        request.daggerfall_dungeons.iter().position(|dungeon| current.is(dungeon))?;
                    self.designs.iter().position(|design| design.dungeons.contains(&dungeon))
                })
                .or((!self.designs.is_empty()).then_some(0));
        }

        let title = match request.read_only {
            true => format!("Entrance of {}", request.dungeon_name),
            false => format!("Choose an entrance for {}", request.dungeon_name),
        };
        let builder = egui::ViewportBuilder::default().with_title(title).with_inner_size([1100.0, 700.0]);
        let mut chosen = None;
        context.show_viewport_immediate(
            egui::ViewportId::from_hash_of("entrance_window"),
            builder,
            |context, _| {
                chosen = self.show_contents(context, request, resources);
                if context.input(|input| input.viewport().close_requested()) {
                    self.open = false;
                }
            },
        );
        if chosen.is_some() {
            self.open = false;
        }
        if !self.open {
            if let Some(render_state) = resources.render_state {
                self.release(render_state);
            }
        }
        chosen
    }

    fn show_contents(
        &mut self,
        context: &egui::Context,
        request: &EntranceRequest<'_>,
        resources: &mut EditorResources<'_>,
    ) -> Option<DungeonReference> {
        let current_design = request.current.and_then(|current| {
            let dungeon = request.daggerfall_dungeons.iter().position(|dungeon| current.is(dungeon))?;
            self.designs.iter().position(|design| design.dungeons.contains(&dungeon))
        });

        egui::SidePanel::left("entrance_list").resizable(true).default_width(330.0).show(context, |ui| {
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search by kind, block or dungeon name")
                    .desired_width(f32::INFINITY),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} entrances, shared by {} Daggerfall dungeons",
                    self.designs.len(),
                    request.daggerfall_dungeons.len()
                ))
                .weak(),
            );
            ui.separator();
            let needle = self.search.to_lowercase();
            // A new search starts at the top of the list.
            let list =
                egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("entrances", &self.search));
            list.show(ui, |ui| {
                for (index, design) in self.designs.iter().enumerate() {
                    if !needle.is_empty() && !design_matches(design, &needle, request.daggerfall_dungeons) {
                        continue;
                    }
                    let mut text = egui::text::LayoutJob::default();
                    let body =
                        egui::TextFormat { color: ui.visuals().text_color(), ..egui::TextFormat::default() };
                    let weak = egui::TextFormat { color: ui.visuals().weak_text_color(), ..body.clone() };
                    text.append(design.kind, 0.0, body.clone());
                    text.append(
                        &format!("  {}", design.block_list(LISTED_BLOCKS_IN_LIST)),
                        0.0,
                        weak.clone(),
                    );
                    let users = design.dungeons.len();
                    text.append(
                        &format!("\n{users} dungeon{}", if users == 1 { "" } else { "s" }),
                        0.0,
                        weak.clone(),
                    );
                    if Some(index) == current_design {
                        text.append("  ✔ in use", 0.0, egui::TextFormat { color: GOOD_COLOR, ..weak });
                    }
                    let response = ui.selectable_label(self.selected == Some(index), text);
                    if self.scroll_to_selected && self.selected == Some(index) {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        self.selected = Some(index);
                    }
                }
                self.scroll_to_selected = false;
            });
        });

        let Some(selected) = self.selected.filter(|&index| index < self.designs.len()) else {
            egui::CentralPanel::default().show(context, |ui| {
                ui.label("No entrances found in the game data.");
            });
            return None;
        };
        if self.preview.as_ref().map(|preview| preview.design_index) != Some(selected) {
            let preview = build_preview(selected, &self.designs[selected], resources.library);
            if let Some((minimum, maximum)) = preview.scene.bounds(resources.library) {
                self.camera.frame_bounds(minimum, maximum);
            }
            self.preview = Some(preview);
        }
        let design = &self.designs[selected];
        let mut chosen = None;

        egui::TopBottomPanel::top("entrance_details").show(context, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading(design.kind);
                ui.label(format!(
                    "{} × {} block{}: {}",
                    design.exterior.width,
                    design.exterior.height,
                    if design.exterior.block_names.len() == 1 { "" } else { "s" },
                    design.block_list(LISTED_BLOCKS_IN_DETAILS)
                ));
            });
            let names: Vec<String> = design
                .dungeons
                .iter()
                .take(LISTED_DUNGEONS)
                .filter_map(|&index| request.daggerfall_dungeons.get(index))
                .map(|dungeon| format!("{} ({})", dungeon.name, dungeon.region_name))
                .collect();
            let more = design.dungeons.len().saturating_sub(LISTED_DUNGEONS);
            let more = if more > 0 { format!(" and {more} more") } else { String::new() };
            ui.label(format!("Used by {}{more}", names.join(", ")))
                .on_hover_text("Daggerfall's dungeons with this entrance");
            if let Some(preview) = &self.preview {
                for problem in &preview.problems {
                    ui.colored_label(WARNING_COLOR, format!("⚠ {problem}"));
                }
                if preview.entrance_doors.is_empty() {
                    ui.colored_label(
                        WARNING_COLOR,
                        "⚠ No dungeon door was found in these blocks, so the player may not be able to go in",
                    );
                }
            }
            ui.horizontal(|ui| {
                if Some(selected) == current_design {
                    ui.colored_label(GOOD_COLOR, "✔ This is the dungeon's entrance");
                } else if !request.read_only {
                    let button = ui.button(egui::RichText::new("Use this entrance").strong()).on_hover_text(
                        "Gives the dungeon this entrance. It is taken from the first Daggerfall dungeon listed above.",
                    );
                    if button.clicked() {
                        chosen = design
                            .dungeons
                            .first()
                            .and_then(|&index| request.daggerfall_dungeons.get(index))
                            .map(DungeonReference::to);
                    }
                }
                ui.label(
                    egui::RichText::new(
                        "The dungeon door is highlighted. Right-drag to turn, scroll to zoom, middle-drag to pan.",
                    )
                    .weak(),
                );
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().frame(egui::Frame::none()).show(context, |ui| {
            let rect = ui.available_rect_before_wrap();
            let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            let _ = handle_camera_input(&mut self.camera, ui, &response);
            let Some(preview) = &self.preview else { return };
            let request = FrameRequest {
                scene: Some(&preview.scene),
                camera: &self.camera,
                selected_object: None,
                hovered_object: None,
                selected_objects: &preview.entrance_doors,
                hovered_objects: &[],
                highlighted_group: None,
                highlighted_groups: &[],
                selected_groups: &[],
                placement_preview: None,
                context_scene: None,
                ground_squares: &preview.ground,
                water_planes: &[],
                room_outlines: &[],
                settings: &self.view_settings,
                grid_height: 0.0,
                snap_grid_spacing: None,
                size_in_pixels: size_in_pixels(ui, rect),
                sky: None,
            };
            paint_scene(ui, rect, resources, &mut self.target, &request);
        });
        chosen
    }
}

/// Whether `design` matches the lower case search text `needle`.
fn design_matches(design: &EntranceDesign, needle: &str, dungeons: &[DungeonDefinition]) -> bool {
    design.kind.to_lowercase().contains(needle)
        || design.exterior.block_names.iter().any(|name| name.to_lowercase().contains(needle))
        || design.dungeons.iter().filter_map(|&index| dungeons.get(index)).any(|dungeon| {
            dungeon.name.to_lowercase().contains(needle)
                || dungeon.region_name.to_lowercase().contains(needle)
        })
}

/// Builds the scene of `design`: its blocks side by side on squares of ground.
fn build_preview(design_index: usize, design: &EntranceDesign, library: &mut Arena2Library) -> Preview {
    let mut objects = Vec::new();
    let mut ground = Vec::new();
    let mut entrance_doors = Vec::new();
    let mut problems = Vec::new();
    let width = design.exterior.width.max(1);
    for (index, name) in design.exterior.block_names.iter().enumerate() {
        let corner =
            Vec3::new((index % width) as f32 * RMB_BLOCK_SIDE, 0.0, (index / width) as f32 * RMB_BLOCK_SIDE);
        ground.push((corner, RMB_BLOCK_SIDE));
        let block = match library.rmb_block(name) {
            Ok(block) => block,
            Err(error) => {
                problems.push(format!("{name} could not be loaded: {error}"));
                continue;
            }
        };
        for model in &block.models {
            let is_entrance = library
                .model(model.model_id)
                .and_then(|mesh| mesh.door_kind())
                .is_some_and(|kind| kind == DoorKind::DungeonEntrance);
            if is_entrance {
                entrance_doors.push(objects.len());
            }
            objects.push(preview_object(
                format!("Model {}", model.model_id),
                SceneObjectKind::Model { model_id: model.model_id, data: ModelData::default() },
                corner + model.position,
                Vec3::new(0.0, model.yaw_degrees, 0.0),
                objects.len(),
            ));
        }
        for flat in &block.flats {
            // Flats stand on their position; the renderer centres them on it.
            let height = library.billboard_world_size(flat.texture).map_or(0.0, |size| size.y);
            objects.push(preview_object(
                flat_name(flat.texture),
                SceneObjectKind::Flat { texture: flat.texture, data: FlatData::default() },
                corner + flat.base_position + Vec3::new(0.0, height / 2.0, 0.0),
                Vec3::ZERO,
                objects.len(),
            ));
        }
    }
    let scene = Scene { block_name: design.block_list(usize::MAX), objects };
    Preview { design_index, scene, ground, entrance_doors, problems }
}

fn flat_name(texture: TextureKey) -> String {
    format!("Flat {}.{}", texture.archive, texture.record)
}

fn preview_object(
    name: String,
    kind: SceneObjectKind,
    position: Vec3,
    rotation_degrees: Vec3,
    index: usize,
) -> SceneObject {
    SceneObject {
        id: index as u32 + 1,
        name,
        group_index: 0,
        kind,
        position,
        rotation_degrees,
        visible: true,
        next: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dungeon(name: &str, blocks: &[&str]) -> DungeonDefinition {
        DungeonDefinition {
            region_index: 17,
            region_name: "Daggerfall",
            name: name.to_string(),
            dungeon_type: None,
            map_pixel: None,
            location_type: None,
            exterior: Some(LocationExterior {
                width: blocks.len(),
                height: 1,
                block_names: blocks.iter().map(|name| name.to_string()).collect(),
                buildings: Vec::new(),
            }),
            classic: None,
            blocks: Vec::new(),
        }
    }

    #[test]
    fn groups_dungeons_by_entrance() {
        let dungeons = [
            dungeon("A", &["RUINAA01.RMB"]),
            dungeon("B", &["CASTAA02.RMB", "RESIAA01.RMB"]),
            dungeon("C", &["RUINAA01.RMB"]),
            dungeon("D", &["GENRAA01.RMB"]),
        ];
        let designs = entrance_designs(&dungeons);
        let summary: Vec<(&str, Vec<usize>)> =
            designs.iter().map(|design| (design.kind, design.dungeons.clone())).collect();
        assert_eq!(summary, [("Castle", vec![1]), ("Other", vec![3]), ("Ruins", vec![0, 2])]);
        assert_eq!(designs[0].block_list(5), "CASTAA02, RESIAA01");
        assert_eq!(designs[0].block_list(1), "CASTAA02 and 1 more");
        assert!(design_matches(&designs[2], "c", &dungeons));
        assert!(!design_matches(&designs[2], "zz", &dungeons));
    }
}
