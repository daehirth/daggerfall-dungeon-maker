//! The asset palette: a browsable list of models, flats, markers and lights
//! that can be placed into the open room.
//!
//! Clicking a tile "arms" that asset. While an asset is armed the viewport
//! shows a ghost preview under the mouse, and a left click places a copy.
//! Model tiles are captioned with the model's display name and can be
//! renamed by right-clicking them.

use std::collections::HashMap;

use eframe::egui;
use glam::Vec3;

use crate::app::viewport_tools::EditorResources;
use crate::arena2::{Arena2Library, TextureKey};
use crate::catalog::{flat_description, AssetCatalog};
use crate::custom_names::CustomNames;
use crate::render::ThumbnailStatus;
use crate::scene::{default_model_object_name, FlatData, SceneObject, SceneObjectKind, ADDED_OBJECTS_GROUP};

/// Maximum number of thumbnails created per frame, to keep scrolling smooth.
const THUMBNAILS_PER_FRAME: usize = 4;
/// Height reserved under each tile for its caption, in points.
const CAPTION_HEIGHT: f32 = 16.0;
/// Gap between tiles, in points.
const TILE_SPACING: f32 = 6.0;
/// Radius given to newly placed lights, in world units (400 native units).
const DEFAULT_LIGHT_RADIUS: f32 = 10.0;
/// Height above the surface at which new lights are placed, in world units.
const LIGHT_PLACEMENT_HEIGHT: f32 = 1.5;

/// Something that can be placed in a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceableAsset {
    Model { model_id: u32 },
    Flat { texture: TextureKey },
    Light,
}

impl PlaceableAsset {
    /// Name given to objects created from this asset.
    ///
    /// Models get their default object name so they are listed under the
    /// model's display name, which follows the user's naming.
    pub fn name(&self) -> String {
        match self {
            Self::Model { model_id } => default_model_object_name(*model_id),
            Self::Flat { texture } => flat_description(*texture),
            Self::Light => "Light".to_string(),
        }
    }

    /// Name shown for this asset in the palette: for models, the model's display name.
    pub fn label(&self, custom_names: &CustomNames, catalog: &AssetCatalog) -> String {
        match self {
            Self::Model { model_id } => custom_names.model_label(*model_id, catalog),
            _ => self.name(),
        }
    }

    /// Creates a scene object for this asset resting on `surface_point`, turned by `yaw_degrees`.
    ///
    /// Models are lifted so the bottom of their bounding box touches the
    /// surface, flats so their lower edge does, and lights float above it.
    pub fn create_object(
        &self,
        library: &mut Arena2Library,
        surface_point: Vec3,
        yaw_degrees: f32,
    ) -> SceneObject {
        let (kind, position, rotation_degrees) = match *self {
            Self::Model { model_id } => {
                let lift = library.model(model_id).map_or(0.0, |mesh| -mesh.bounds_min.y);
                (
                    SceneObjectKind::Model { model_id, has_action: false },
                    surface_point + Vec3::Y * lift,
                    Vec3::new(0.0, yaw_degrees, 0.0),
                )
            }
            Self::Flat { texture } => {
                let half_height = library.billboard_world_size(texture).map_or(0.5, |size| size.y * 0.5);
                (
                    SceneObjectKind::Flat { texture, data: FlatData::default() },
                    surface_point + Vec3::Y * half_height,
                    Vec3::ZERO,
                )
            }
            Self::Light => (
                SceneObjectKind::Light { radius: DEFAULT_LIGHT_RADIUS },
                surface_point + Vec3::Y * LIGHT_PLACEMENT_HEIGHT,
                Vec3::ZERO,
            ),
        };
        SceneObject {
            name: self.name(),
            group_index: ADDED_OBJECTS_GROUP,
            kind,
            position,
            rotation_degrees,
            visible: true,
        }
    }
}

/// An asset chosen for placement, with the rotation the user has dialled in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmedAsset {
    pub asset: PlaceableAsset,
    /// Rotation around the vertical axis applied to placed models, in degrees.
    pub yaw_degrees: f32,
}

/// The palette's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteTab {
    RoomModels,
    AllModels,
    Flats,
    Markers,
    Lights,
}

impl PaletteTab {
    const ALL: [Self; 5] = [Self::RoomModels, Self::AllModels, Self::Flats, Self::Markers, Self::Lights];

    fn label(self) -> &'static str {
        match self {
            Self::RoomModels => "Cell models",
            Self::AllModels => "All models",
            Self::Flats => "Flats",
            Self::Markers => "Markers",
            Self::Lights => "Lights",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::RoomModels => "Models used in Daggerfall's dungeon cells, most common first",
            Self::AllModels => "Every model in ARCH3D.BSA",
            Self::Flats => "Billboard sprites used in dungeon cells: torches, furniture, people, treasure",
            Self::Markers => {
                "Editor markers: start and enter points, enemy spawns, quest and treasure markers"
            }
            Self::Lights => "Point lights",
        }
    }
}

/// One tile in the palette grid.
struct PaletteEntry {
    asset: PlaceableAsset,
    /// Name shown under the tile.
    caption: String,
    /// Extra information shown in the tooltip.
    details: String,
}

impl PaletteEntry {
    /// Builds the tile for `asset`, with `usage` describing where the game uses it.
    fn new(asset: PlaceableAsset, usage: String, custom_names: &CustomNames, catalog: &AssetCatalog) -> Self {
        let details = match asset {
            PlaceableAsset::Model { model_id } => match catalog.model_description(model_id) {
                Some(description) => format!("Model {model_id}\n{}\n{usage}", description.summary()),
                None => format!("Model {model_id}\n{usage}"),
            },
            _ => usage,
        };
        Self { asset, caption: asset.label(custom_names, catalog), details }
    }
}

/// State of the asset palette panel.
pub struct AssetPalette {
    /// Whether the panel is shown.
    pub is_open: bool,
    /// Asset currently armed for placement.
    pub armed: Option<ArmedAsset>,
    tab: PaletteTab,
    search_text: String,
    /// Edge length of each tile, in points.
    tile_size: f32,
    /// egui textures for flat thumbnails; `None` marks textures that failed to load.
    flat_thumbnails: HashMap<TextureKey, Option<egui::TextureHandle>>,
}

impl Default for AssetPalette {
    fn default() -> Self {
        Self {
            is_open: true,
            armed: None,
            tab: PaletteTab::RoomModels,
            search_text: String::new(),
            tile_size: 84.0,
            flat_thumbnails: HashMap::new(),
        }
    }
}

impl AssetPalette {
    /// Arms `asset` for placement, or disarms it if it is already armed.
    fn toggle_armed(&mut self, asset: PlaceableAsset) {
        self.armed = match self.armed {
            Some(armed) if armed.asset == asset => None,
            _ => Some(ArmedAsset { asset, yaw_degrees: 0.0 }),
        };
    }

    /// Rotates the armed asset by `degrees` around the vertical axis.
    pub fn rotate_armed(&mut self, degrees: f32) {
        if let Some(armed) = self.armed.as_mut() {
            armed.yaw_degrees = (armed.yaw_degrees + degrees).rem_euclid(360.0);
        }
    }

    /// Draws the palette as a panel along the bottom of the window.
    ///
    /// `editor_id` keeps the panel's id unique when several editors are open.
    pub fn show(
        &mut self,
        context: &egui::Context,
        editor_id: egui::Id,
        resources: &mut EditorResources<'_>,
    ) {
        if !self.is_open {
            return;
        }

        egui::TopBottomPanel::bottom(editor_id.with("asset_palette"))
            .resizable(true)
            .default_height(240.0)
            .min_height(120.0)
            .show(context, |ui| {
                self.show_header(ui, resources);
                ui.separator();

                let entries = self.visible_entries(resources.catalog, resources.custom_names);
                let mut thumbnail_budget = THUMBNAILS_PER_FRAME;
                let mut has_pending_thumbnails = false;

                let tile_size = self.tile_size;
                let tile_stride = tile_size + TILE_SPACING;
                let columns = ((ui.available_width() + TILE_SPACING) / tile_stride).floor().max(1.0) as usize;
                let row_count = entries.len().div_ceil(columns);

                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
                    ui,
                    tile_size + CAPTION_HEIGHT,
                    row_count,
                    |ui, row_range| {
                        ui.spacing_mut().item_spacing = egui::vec2(TILE_SPACING, TILE_SPACING);
                        for row in row_range {
                            ui.horizontal(|ui| {
                                for entry in entries.iter().skip(row * columns).take(columns) {
                                    let thumbnail = self.thumbnail_for(
                                        ui.ctx(),
                                        entry.asset,
                                        resources,
                                        &mut thumbnail_budget,
                                    );
                                    has_pending_thumbnails |= thumbnail == TileImage::Pending;
                                    let response = self.draw_tile(ui, entry, thumbnail);
                                    if response.clicked() {
                                        self.toggle_armed(entry.asset);
                                    }
                                    if let PlaceableAsset::Model { model_id } = entry.asset {
                                        response.context_menu(|ui| {
                                            show_model_name_menu(ui, model_id, resources.custom_names);
                                        });
                                    }
                                }
                            });
                        }
                    },
                );

                if has_pending_thumbnails {
                    ui.ctx().request_repaint();
                }
            });
    }

    /// Draws the tab bar, search box, tile size slider and placement hint.
    fn show_header(&mut self, ui: &mut egui::Ui, resources: &EditorResources<'_>) {
        ui.horizontal(|ui| {
            ui.strong("Assets");
            ui.separator();
            for tab in PaletteTab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.label()).on_hover_text(tab.tooltip());
            }
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.search_text)
                    .hint_text("Search: id, name, door, Torture…")
                    .desired_width(180.0),
            );
            ui.add(egui::Slider::new(&mut self.tile_size, 56.0..=140.0).show_value(false).text("Size"));
        });

        // The placement hint gets its own line so it is never cut off by the panel width.
        let hint_color = egui::Color32::from_rgb(110, 230, 140);
        match self.armed {
            Some(armed) => {
                ui.horizontal_wrapped(|ui| {
                    let asset_label = armed.asset.label(resources.custom_names, resources.catalog);
                    ui.colored_label(hint_color, format!("Placing {asset_label} ({:.0}°)", armed.yaw_degrees));
                    ui.label(
                        egui::RichText::new(
                            "Click in the cell to place · Shift+click to keep placing · R to rotate · Esc to cancel",
                        )
                        .weak(),
                    );
                    if ui.small_button("Cancel").clicked() {
                        self.armed = None;
                    }
                });
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "Click an asset to start placing it in the cell · right-click a model to name it",
                    )
                    .weak(),
                );
            }
        }
    }

    /// Returns the tiles of the current tab that match the search text.
    fn visible_entries(&self, catalog: &AssetCatalog, custom_names: &CustomNames) -> Vec<PaletteEntry> {
        let search = self.search_text.trim().to_lowercase();
        let matches = |text: &str| search.is_empty() || text.to_lowercase().contains(&search);
        // Models match on their id, the user's name for them, or their description.
        let model_matches = |model_id: u32| {
            matches(&model_id.to_string())
                || custom_names.model_name_matches(model_id, &search)
                || catalog.model_description(model_id).is_some_and(|description| description.matches(&search))
        };
        let entry = |asset, usage| PaletteEntry::new(asset, usage, custom_names, catalog);

        match self.tab {
            PaletteTab::RoomModels => catalog
                .room_models
                .iter()
                .filter(|model| model_matches(model.model_id))
                .map(|model| {
                    let usage = format!(
                        "Used in {} cells, {} placements in total",
                        model.room_count, model.placement_count
                    );
                    entry(PlaceableAsset::Model { model_id: model.model_id }, usage)
                })
                .collect(),
            PaletteTab::AllModels => catalog
                .all_model_ids
                .iter()
                .filter(|&&model_id| model_matches(model_id))
                .map(|&model_id| {
                    let usage = match catalog.room_model(model_id) {
                        Some(model) => format!("Used in {} cells", model.room_count),
                        None => "Not used in any dungeon cell".to_string(),
                    };
                    entry(PlaceableAsset::Model { model_id }, usage)
                })
                .collect(),
            PaletteTab::Flats | PaletteTab::Markers => {
                let flats = if self.tab == PaletteTab::Flats { &catalog.flats } else { &catalog.markers };
                flats
                    .iter()
                    .filter(|flat| matches(&flat.description()))
                    .map(|flat| {
                        let usage = format!("{}\nUsed in {} cells", flat.texture, flat.room_count);
                        entry(PlaceableAsset::Flat { texture: flat.texture }, usage)
                    })
                    .collect()
            }
            PaletteTab::Lights => vec![entry(
                PlaceableAsset::Light,
                format!("Point light, radius {DEFAULT_LIGHT_RADIUS} m (editable after placing)"),
            )],
        }
    }

    /// Fetches (and if the budget allows, creates) the image shown on a tile.
    fn thumbnail_for(
        &mut self,
        context: &egui::Context,
        asset: PlaceableAsset,
        resources: &mut EditorResources<'_>,
        budget: &mut usize,
    ) -> TileImage {
        let library = &mut *resources.library;
        match asset {
            PlaceableAsset::Model { model_id } => {
                let (Some(render_state), Some(renderer)) =
                    (resources.render_state, resources.renderer.as_deref_mut())
                else {
                    return TileImage::None;
                };
                // Only thumbnails that still need rendering count against this frame's budget.
                let needs_render = !renderer.has_cached_thumbnail(model_id);
                let allow_render = *budget > 0;
                if needs_render && allow_render {
                    *budget -= 1;
                }
                match renderer.model_thumbnail(render_state, library, model_id, allow_render) {
                    ThumbnailStatus::Ready(texture_id) => {
                        TileImage::Texture { texture_id, aspect_ratio: 1.0 }
                    }
                    ThumbnailStatus::Pending => TileImage::Pending,
                    ThumbnailStatus::Unavailable => TileImage::None,
                }
            }
            PlaceableAsset::Flat { texture } => {
                let cached_handle = match self.flat_thumbnails.entry(texture) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        if *budget == 0 {
                            return TileImage::Pending;
                        }
                        *budget -= 1;
                        entry.insert(library.texture_image(texture).map(|image| {
                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                [image.width as usize, image.height as usize],
                                &image.pixels,
                            );
                            context.load_texture(
                                texture.to_string(),
                                color_image,
                                egui::TextureOptions::NEAREST,
                            )
                        }))
                    }
                };
                match cached_handle.as_ref() {
                    Some(handle) => {
                        let [width, height] = handle.size();
                        TileImage::Texture {
                            texture_id: handle.id(),
                            aspect_ratio: width as f32 / height.max(1) as f32,
                        }
                    }
                    None => TileImage::None,
                }
            }
            PlaceableAsset::Light => TileImage::LightIcon,
        }
    }

    /// Draws one tile and returns its response.
    fn draw_tile(&self, ui: &mut egui::Ui, entry: &PaletteEntry, image: TileImage) -> egui::Response {
        let tile_size = self.tile_size;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(tile_size, tile_size + CAPTION_HEIGHT), egui::Sense::click());
        let image_rect = egui::Rect::from_min_size(rect.min, egui::vec2(tile_size, tile_size));
        let painter = ui.painter_at(rect);

        let is_armed = self.armed.is_some_and(|armed| armed.asset == entry.asset);
        let background = if is_armed {
            egui::Color32::from_rgb(40, 80, 50)
        } else if response.hovered() {
            egui::Color32::from_gray(45)
        } else {
            egui::Color32::from_gray(28)
        };
        painter.rect_filled(image_rect, 4.0, background);

        match image {
            TileImage::Texture { texture_id, aspect_ratio } => {
                let inner = image_rect.shrink(4.0);
                let size = if aspect_ratio >= 1.0 {
                    egui::vec2(inner.width(), inner.width() / aspect_ratio)
                } else {
                    egui::vec2(inner.height() * aspect_ratio, inner.height())
                };
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(
                    texture_id,
                    egui::Rect::from_center_size(inner.center(), size),
                    uv,
                    egui::Color32::WHITE,
                );
            }
            TileImage::Pending => {
                painter.text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "…",
                    egui::FontId::proportional(18.0),
                    egui::Color32::GRAY,
                );
            }
            TileImage::LightIcon => draw_light_icon(&painter, image_rect),
            TileImage::None => {
                painter.text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "?",
                    egui::FontId::proportional(18.0),
                    egui::Color32::GRAY,
                );
            }
        }

        if is_armed {
            painter.rect_stroke(
                image_rect,
                4.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(110, 230, 140)),
            );
        }

        painter.text(
            egui::pos2(rect.center().x, image_rect.bottom() + 2.0),
            egui::Align2::CENTER_TOP,
            truncate_caption(&entry.caption, tile_size),
            egui::FontId::proportional(11.0),
            ui.visuals().text_color(),
        );

        let rename_hint = match entry.asset {
            PlaceableAsset::Model { .. } => " · right-click to name it",
            _ => "",
        };
        response.on_hover_text(format!("{}\n{}\nClick to place{rename_hint}", entry.caption, entry.details))
    }
}

/// Contents of a model tile's right-click menu: a field for the user's name for the model.
fn show_model_name_menu(ui: &mut egui::Ui, model_id: u32, custom_names: &mut CustomNames) {
    ui.strong(format!("Model {model_id}"));
    let mut name = custom_names.model_name_for_editing(model_id);
    let field = egui::TextEdit::singleline(&mut name).hint_text("Your name for this model (editor only)");
    if ui.add(field).changed() {
        custom_names.set_model_name(model_id, &name);
    }
}

/// What is drawn inside a tile.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TileImage {
    Texture { texture_id: egui::TextureId, aspect_ratio: f32 },
    Pending,
    LightIcon,
    None,
}

/// Draws a simple light bulb style icon.
fn draw_light_icon(painter: &egui::Painter, rect: egui::Rect) {
    let center = rect.center();
    let radius = rect.width() * 0.16;
    let color = egui::Color32::from_rgb(255, 225, 110);
    painter.circle_filled(center, radius, color);
    for ray in 0..8 {
        let angle = ray as f32 / 8.0 * std::f32::consts::TAU;
        let direction = egui::vec2(angle.cos(), angle.sin());
        painter.line_segment(
            [center + direction * radius * 1.4, center + direction * radius * 2.2],
            egui::Stroke::new(2.0_f32, color),
        );
    }
}

/// Shortens a caption with an ellipsis so it fits under a tile.
fn truncate_caption(caption: &str, tile_size: f32) -> String {
    let max_characters = ((tile_size / 6.0) as usize).max(4);
    if caption.chars().count() <= max_characters {
        caption.to_string()
    } else {
        let shortened: String = caption.chars().take(max_characters - 1).collect();
        format!("{shortened}…")
    }
}
