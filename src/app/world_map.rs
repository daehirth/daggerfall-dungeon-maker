//! A window of its own showing the world map, for choosing where a dungeon goes.
//!
//! The map is drawn from the game data: each region in its own colour, the sea
//! in blue, and every location as a dark dot (dungeons in red). Scroll to zoom,
//! drag to pan, and click a spot to choose it for the dungeon; the choice is
//! only made when **Select Location** (bottom right) is pressed. Clicking one
//! of Daggerfall's dungeons chooses to replace it: the dungeon then takes that
//! dungeon's place in the game. Every dungeon of
//! the project that has a spot is marked with a diamond and its name, the one
//! being placed in yellow and the others in cyan.
//!
//! The search field finds a location by name (Daggerfall's own, or a project
//! dungeon), suggesting matches as the user types; choosing one, or **Find**,
//! zooms in on it and rings it in white.

use eframe::egui;

use crate::arena2::{
    region_name, DungeonDefinition, LocationType, MapPixel, WorldMap, MAP_HEIGHT, MAP_WIDTH,
};
use crate::project::DungeonReference;

/// Colour of the sea, and of anything outside every region.
const SEA_COLOR: egui::Color32 = egui::Color32::from_rgb(30, 56, 92);
/// Colour of a location that is not a dungeon.
const LOCATION_COLOR: egui::Color32 = egui::Color32::from_rgb(35, 35, 35);
/// Colour of a dungeon location.
const DUNGEON_COLOR: egui::Color32 = egui::Color32::from_rgb(150, 30, 30);
/// Marker of the dungeon being placed.
const CURRENT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 220, 60);
/// Markers of the project's other dungeons.
const OTHER_DUNGEON_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 220, 230);
/// Outline of the markers, so they show on any colour.
const MARKER_OUTLINE: egui::Color32 = egui::Color32::from_rgb(15, 15, 15);
/// Half the width of a project dungeon's marker, in points.
const MARKER_SIZE: f32 = 6.0;
/// Marker of the spot clicked but not yet selected.
const PENDING_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 150, 40);
/// Ring around a location found with the search field.
const FOUND_COLOR: egui::Color32 = egui::Color32::WHITE;
/// Most suggestions listed under the search field.
const MAX_SUGGESTIONS: usize = 12;
/// Zoom used to show a location that was found.
const FOUND_ZOOM: f32 = 12.0;
/// Largest zoom: this many screen points per map pixel at most, roughly.
const MAX_ZOOM: f32 = 40.0;

/// What the window can change.
pub struct MapPickRequest<'request> {
    pub world: &'request WorldMap,
    /// Name of the dungeon being placed, for the title.
    pub dungeon_name: &'request str,
    /// Its current spot.
    pub current: Option<MapPixel>,
    /// Whether the dungeon being shown is one of the project's.
    pub current_is_project_dungeon: bool,
    /// The project's other placed dungeons.
    pub other_dungeons: &'request [(&'request str, MapPixel)],
    /// Set for Daggerfall's own dungeons, which are only shown.
    pub read_only: bool,
    /// Why the dungeon cannot be placed yet, if it cannot: the map is then only shown.
    pub placement_blocked: Option<String>,
    /// Daggerfall's dungeons, which can be chosen to be replaced.
    pub daggerfall_dungeons: &'request [DungeonDefinition],
}

/// Where the user chose to put the dungeon.
#[derive(Debug, Clone, PartialEq)]
pub enum MapChoice {
    /// A free spot, as a new location.
    Spot(MapPixel),
    /// The place of one of Daggerfall's dungeons, at its spot, replacing it.
    Replace(DungeonReference, MapPixel),
}

impl MapChoice {
    pub fn pixel(&self) -> MapPixel {
        match self {
            Self::Spot(pixel) | Self::Replace(_, pixel) => *pixel,
        }
    }
}

impl MapPickRequest<'_> {
    /// Whether clicking the map places the dungeon.
    fn can_place(&self) -> bool {
        !self.read_only && self.placement_blocked.is_none()
    }
}

/// What is already at `pixel`, which a dungeon cannot share: a Daggerfall
/// location or another of the project's dungeons. `None` if the spot is free.
pub fn occupant(world: &WorldMap, other_dungeons: &[(&str, MapPixel)], pixel: MapPixel) -> Option<String> {
    if let Some(location) = world.locations_at(pixel).next() {
        return Some(format!("Daggerfall's {}", location.name));
    }
    other_dungeons
        .iter()
        .find(|(_, other)| *other == pixel)
        .map(|(name, _)| format!("{name}, in this project,"))
}

/// Colour of the reason a dungeon cannot be placed.
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 190, 70);

/// State of the world map window.
pub struct WorldMapPicker {
    open: bool,
    /// 1 shows the whole map; larger values zoom in.
    zoom: f32,
    /// Map pixel at the centre of the view.
    center: egui::Vec2,
    texture: Option<egui::TextureHandle>,
    /// Text typed in the search field.
    search: String,
    /// The location last found with the search field, ringed on the map.
    found: Option<Suggestion>,
    /// Why the last click did not choose a spot, shown under the map.
    refusal: Option<String>,
    /// The spot clicked, waiting for **Select Location**.
    pending: Option<MapChoice>,
}

/// A location the search field suggests.
#[derive(Debug, Clone, PartialEq)]
struct Suggestion {
    /// Its name.
    name: String,
    /// What and where it is, for example "Large dungeon · Daggerfall".
    details: String,
    pixel: MapPixel,
}

impl Default for WorldMapPicker {
    fn default() -> Self {
        Self {
            open: false,
            zoom: 1.0,
            center: map_size() / 2.0,
            texture: None,
            search: String::new(),
            found: None,
            refusal: None,
            pending: None,
        }
    }
}

impl WorldMapPicker {
    /// Opens the window, zoomed in on `pixel` if given.
    pub fn open(&mut self, pixel: Option<MapPixel>) {
        self.open = true;
        self.pending = None;
        self.refusal = None;
        if let Some(pixel) = pixel {
            self.zoom = self.zoom.max(6.0);
            self.center = pixel_center(pixel);
        }
    }

    /// Draws the window, in its own OS window, if it is open. Returns the spot
    /// chosen, once **Select Location** confirms it (the window then closes).
    pub fn show(&mut self, context: &egui::Context, request: &MapPickRequest<'_>) -> Option<MapChoice> {
        if !self.open {
            return None;
        }
        // Textures are shared by every window of the application.
        let texture = self
            .texture
            .get_or_insert_with(|| {
                context.load_texture("world_map", map_image(request.world), egui::TextureOptions::NEAREST)
            })
            .clone();
        let mut action = None;
        let title = match request.read_only {
            true => format!("{} on the world map", request.dungeon_name),
            false => format!("Place {} on the world map", request.dungeon_name),
        };
        let builder = egui::ViewportBuilder::default().with_title(title).with_inner_size([1000.0, 600.0]);
        context.show_viewport_immediate(
            egui::ViewportId::from_hash_of("world_map_window"),
            builder,
            |context, _| {
                if !request.read_only {
                    action = self.show_select_bar(context, request);
                }
                egui::CentralPanel::default().show(context, |ui| {
                    ui.horizontal(|ui| {
                        match (&request.placement_blocked, request.read_only) {
                            (Some(reason), false) => {
                                ui.colored_label(
                                    WARNING_COLOR,
                                    format!("⚠ It cannot be placed yet: {reason}."),
                                );
                            }
                            (_, true) => {
                                ui.label(egui::RichText::new("Scroll to zoom, drag to pan.").weak());
                            }
                            (None, false) => {
                                let hint = "Click a free spot, or one of Daggerfall's dungeons to replace \
                                            it, then Select Location. Scroll to zoom, drag to pan.";
                                ui.label(egui::RichText::new(hint).weak());
                            }
                        }
                        if ui
                            .button("Whole map")
                            .on_hover_text("Zoom out to show the whole Iliac Bay")
                            .clicked()
                        {
                            self.zoom = 1.0;
                            self.center = map_size() / 2.0;
                        }
                        if let Some(pixel) = request.current {
                            if ui
                                .button("Go to dungeon")
                                .on_hover_text("Zoom in on the dungeon's spot")
                                .clicked()
                            {
                                self.zoom = self.zoom.max(6.0);
                                self.center = pixel_center(pixel);
                            }
                        }
                        let (icon, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        paint_diamond(ui.painter(), icon.center(), OTHER_DUNGEON_COLOR);
                        ui.label(egui::RichText::new("your dungeons").color(OTHER_DUNGEON_COLOR))
                            .on_hover_text("Every dungeon of this project that has a spot on the map");
                    });
                    self.show_search(ui, request);
                    if let Some(choice) = self.show_map(ui, &texture, request) {
                        self.pending = Some(choice);
                    }
                });
                if context.input(|input| input.viewport().close_requested()) {
                    self.open = false;
                }
            },
        );
        action
    }

    /// Draws the search field and its suggestions. Choosing a suggestion, or
    /// **Find** (or Enter) for the best match, zooms in on that location.
    fn show_search(&mut self, ui: &mut egui::Ui, request: &MapPickRequest<'_>) {
        let suggestions = suggestions(&self.search, request);
        let popup_id = ui.make_persistent_id("world_map_search_suggestions");
        let mut chosen = None;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Find a location on the map")
                    .desired_width(260.0),
            );
            if field.changed() {
                if self.search.trim().is_empty() {
                    ui.memory_mut(|memory| memory.close_popup());
                } else {
                    ui.memory_mut(|memory| memory.open_popup(popup_id));
                }
            }
            let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let find = ui
                .add_enabled(!suggestions.is_empty(), egui::Button::new("Find"))
                .on_hover_text("Zoom in on the best match")
                .on_disabled_hover_text("Type part of a location's name");
            if (find.clicked() || entered) && !suggestions.is_empty() {
                chosen = suggestions.first().cloned();
            }
            if !suggestions.is_empty() {
                egui::popup_below_widget(
                    ui,
                    popup_id,
                    &field,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    |ui| {
                        ui.set_min_width(360.0);
                        for suggestion in &suggestions {
                            let mut text = egui::text::LayoutJob::default();
                            let body =
                                egui::TextFormat { color: ui.visuals().text_color(), ..Default::default() };
                            let weak = egui::TextFormat {
                                color: ui.visuals().weak_text_color(),
                                ..Default::default()
                            };
                            text.append(&suggestion.name, 0.0, body);
                            text.append(&format!("   {}", suggestion.details), 0.0, weak);
                            if ui.selectable_label(false, text).clicked() {
                                chosen = Some(suggestion.clone());
                                ui.memory_mut(|memory| memory.close_popup());
                            }
                        }
                    },
                );
            }
            if let Some(found) = &self.found {
                ui.label(format!("Found {} at {}", found.name, found.pixel));
                if ui.small_button("✖").on_hover_text("Clear the found location").clicked() {
                    self.found = None;
                }
            }
        });
        if let Some(suggestion) = chosen {
            self.zoom = self.zoom.max(FOUND_ZOOM);
            self.center = pixel_center(suggestion.pixel);
            self.search = suggestion.name.clone();
            self.found = Some(suggestion);
            ui.memory_mut(|memory| memory.close_popup());
        }
    }

    /// Draws the bar along the bottom: the spot chosen and **Select Location**,
    /// which confirms it. Returns the spot once confirmed, and closes the window.
    fn show_select_bar(
        &mut self,
        context: &egui::Context,
        request: &MapPickRequest<'_>,
    ) -> Option<MapChoice> {
        let mut selected = None;
        egui::TopBottomPanel::bottom("world_map_select_bar").show(context, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                match &self.pending {
                    Some(MapChoice::Spot(pixel)) => {
                        let region = request.world.region_at(*pixel).map_or("Sea", region_name);
                        ui.label(format!("Chosen: {pixel} · {region}"));
                        if ui.small_button("✖").on_hover_text("Clear the chosen spot").clicked() {
                            self.pending = None;
                        }
                    }
                    Some(MapChoice::Replace(replaced, pixel)) => {
                        ui.label(format!("Chosen: replace {} at {pixel}", replaced.label())).on_hover_text(
                            "The dungeon takes this dungeon's place in the game: its spot, and its name, \
                             which quests and the travel map know it by.",
                        );
                        if ui.small_button("✖").on_hover_text("Clear the choice").clicked() {
                            self.pending = None;
                        }
                    }
                    None => {
                        ui.label(egui::RichText::new("Click a free spot on the map to choose it").weak());
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = egui::Button::new(egui::RichText::new("Select Location").strong());
                    let response = ui
                        .add_enabled(self.pending.is_some(), button)
                        .on_hover_text("Puts the dungeon at the chosen spot (or in place of the chosen dungeon) and closes the map")
                        .on_disabled_hover_text("Click a free spot on the map first");
                    if response.clicked() {
                        selected = self.pending.take();
                        self.open = false;
                    }
                });
            });
            ui.add_space(4.0);
        });
        selected
    }

    /// Draws the zoomable map filling `ui`. Returns what was clicked: a free
    /// spot, or a Daggerfall dungeon to replace.
    fn show_map(
        &mut self,
        ui: &mut egui::Ui,
        texture: &egui::TextureHandle,
        request: &MapPickRequest<'_>,
    ) -> Option<MapChoice> {
        let info_height = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
        let available = ui.available_size() - egui::vec2(0.0, info_height);
        let size = fit_aspect(available.max(egui::vec2(200.0, 100.0)));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

        // Zoom around the pointer, then pan by dragging.
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(pointer) = response.hover_pos() {
                    let before = self.screen_to_map(rect, pointer);
                    self.zoom = (self.zoom * (scroll * 0.003).exp()).clamp(1.0, MAX_ZOOM);
                    self.clamp_center();
                    let after = self.screen_to_map(rect, pointer);
                    self.center += before - after;
                }
            }
        }
        if response.dragged() {
            self.center -= response.drag_delta() / self.scale(rect);
        }
        self.clamp_center();

        let visible = map_size() / self.zoom;
        let top_left = self.center - visible / 2.0;
        let uv = egui::Rect::from_min_size((top_left / map_size()).to_pos2(), visible / map_size());
        let painter = ui.painter_at(rect);
        painter.image(texture.id(), rect, uv, egui::Color32::WHITE);

        // The project's dungeons: a diamond with the name beside it, at any zoom.
        let label_font = egui::FontId::proportional(13.0);
        let draw_marker = |pixel: MapPixel, name: &str, color: egui::Color32| {
            let center = self.map_to_screen(rect, pixel_center(pixel));
            paint_diamond(&painter, center, color);
            let position = center + egui::vec2(MARKER_SIZE + 4.0, 0.0);
            // A dark copy behind the name keeps it readable on light regions.
            painter.text(
                position + egui::vec2(1.0, 1.0),
                egui::Align2::LEFT_CENTER,
                name,
                label_font.clone(),
                MARKER_OUTLINE,
            );
            painter.text(position, egui::Align2::LEFT_CENTER, name, label_font.clone(), color);
        };
        for (name, pixel) in request.other_dungeons {
            draw_marker(*pixel, name, OTHER_DUNGEON_COLOR);
        }
        if let Some(found) = &self.found {
            let center = self.map_to_screen(rect, pixel_center(found.pixel));
            let radius = (self.scale(rect) * 0.9).max(6.0) + 4.0;
            painter.circle_stroke(center, radius + 1.5, egui::Stroke::new(4.0_f32, MARKER_OUTLINE));
            painter.circle_stroke(center, radius, egui::Stroke::new(2.0_f32, FOUND_COLOR));
            let position = center + egui::vec2(radius + 4.0, -radius);
            painter.text(
                position + egui::vec2(1.0, 1.0),
                egui::Align2::LEFT_BOTTOM,
                &found.name,
                label_font.clone(),
                MARKER_OUTLINE,
            );
            painter.text(position, egui::Align2::LEFT_BOTTOM, &found.name, label_font.clone(), FOUND_COLOR);
        }
        match &self.pending {
            Some(MapChoice::Spot(pixel)) => {
                draw_marker(*pixel, &format!("{} (chosen)", request.dungeon_name), PENDING_COLOR);
            }
            Some(MapChoice::Replace(replaced, pixel)) => {
                let label = format!("{} (replaces {})", request.dungeon_name, replaced.name);
                draw_marker(*pixel, &label, PENDING_COLOR);
            }
            None => {}
        }
        if let Some(pixel) = request.current {
            if request.current_is_project_dungeon {
                draw_marker(pixel, request.dungeon_name, CURRENT_COLOR);
            } else {
                let center = self.map_to_screen(rect, pixel_center(pixel));
                let radius = (self.scale(rect) * 0.9).max(5.0) + 2.0;
                painter.circle_stroke(center, radius, egui::Stroke::new(3.0_f32, CURRENT_COLOR));
            }
        }

        // What is under the pointer.
        let hovered = response
            .hover_pos()
            .map(|pointer| self.screen_to_map(rect, pointer))
            .map(|map| MapPixel::new(map.x.floor() as i32, map.y.floor() as i32))
            .filter(|pixel| pixel.is_on_map());
        ui.label(match hovered {
            Some(pixel) => describe_spot(pixel, request),
            None => "Point at the map to see what is there".to_string(),
        });

        // A free spot is taken as a new location, and a Daggerfall dungeon is
        // replaced; anything else already at a spot keeps it.
        let hovered_choice = hovered.map(|pixel| choice_at(request, pixel));
        let clicked = response.clicked() && request.can_place();
        if clicked {
            match hovered_choice.clone() {
                Some(Ok(choice)) => {
                    self.refusal = None;
                    return Some(choice);
                }
                Some(Err(refusal)) => self.refusal = Some(refusal),
                None => {}
            }
        }
        if let Some(refusal) = &self.refusal {
            ui.colored_label(WARNING_COLOR, format!("⚠ {refusal}"));
        }
        if request.can_place() && response.hovered() {
            let icon = match hovered_choice {
                Some(Err(_)) => egui::CursorIcon::NotAllowed,
                _ => egui::CursorIcon::Crosshair,
            };
            ui.ctx().set_cursor_icon(icon);
        }
        None
    }

    /// Screen points per map pixel.
    fn scale(&self, rect: egui::Rect) -> f32 {
        rect.width() * self.zoom / MAP_WIDTH as f32
    }

    fn screen_to_map(&self, rect: egui::Rect, position: egui::Pos2) -> egui::Vec2 {
        self.center + (position - rect.center()) / self.scale(rect)
    }

    fn map_to_screen(&self, rect: egui::Rect, map: egui::Vec2) -> egui::Pos2 {
        rect.center() + (map - self.center) * self.scale(rect)
    }

    /// Keeps the view inside the map.
    fn clamp_center(&mut self) {
        let half = map_size() / self.zoom / 2.0;
        self.center.x = self.center.x.clamp(half.x, map_size().x - half.x);
        self.center.y = self.center.y.clamp(half.y, map_size().y - half.y);
    }
}

/// What clicking `pixel` chooses, or why it cannot be chosen.
fn choice_at(request: &MapPickRequest<'_>, pixel: MapPixel) -> Result<MapChoice, String> {
    if let Some((name, _)) = request.other_dungeons.iter().find(|(_, other)| *other == pixel) {
        return Err(format!("{name}, in this project, is already at {pixel}: choose another spot"));
    }
    let dungeon = request.daggerfall_dungeons.iter().find(|dungeon| dungeon.map_pixel == Some(pixel));
    match (dungeon, request.world.locations_at(pixel).next()) {
        (Some(dungeon), _) if dungeon.classic.is_some() => {
            Ok(MapChoice::Replace(DungeonReference::to(dungeon), pixel))
        }
        (Some(dungeon), _) => {
            Err(format!("{} cannot be replaced: its game data is incomplete", dungeon.name))
        }
        (None, Some(location)) => Err(format!(
            "Daggerfall's {} is already at {pixel}, and only dungeons can be replaced: choose another spot",
            location.name
        )),
        (None, None) => Ok(MapChoice::Spot(pixel)),
    }
}

/// Describes `pixel`: its position, region and anything already there.
fn describe_spot(pixel: MapPixel, request: &MapPickRequest<'_>) -> String {
    let region = match request.world.region_at(pixel) {
        Some(region_index) => region_name(region_index),
        None if request.world.has_regions() => "Sea",
        None => "Unknown region",
    };
    let mut text = format!("{pixel} · {region}");
    let mut taken = false;
    for location in request.world.locations_at(pixel) {
        let kind = location.location_type.map_or("location", LocationType::name);
        text.push_str(&format!(" · {} ({kind})", location.name));
        taken = true;
    }
    for (name, _) in request.other_dungeons.iter().filter(|(_, other)| *other == pixel) {
        text.push_str(&format!(" · {name} (this project)"));
        taken = true;
    }
    if Some(pixel) == request.current {
        text.push_str(&format!(" · {} is here", request.dungeon_name));
    } else if request.can_place() && !taken && request.world.region_at(pixel).is_some() {
        text.push_str(" · free");
    } else if request.can_place() && matches!(choice_at(request, pixel), Ok(MapChoice::Replace(..))) {
        text.push_str(" · click to replace this dungeon");
    }
    text
}

/// The locations whose names hold `search` (ignoring case): the project's
/// dungeons first, then names starting with it, then the rest, each in
/// alphabetical order. At most [`MAX_SUGGESTIONS`].
fn suggestions(search: &str, request: &MapPickRequest<'_>) -> Vec<Suggestion> {
    let needle = search.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<(u8, Suggestion)> = Vec::new();
    let mut add = |rank: u8, name: &str, details: String, pixel: MapPixel| {
        let lower = name.to_lowercase();
        if let Some(position) = lower.find(&needle) {
            let rank = rank * 2 + u8::from(position != 0);
            matches.push((rank, Suggestion { name: name.to_string(), details, pixel }));
        }
    };
    for (name, pixel) in request.other_dungeons {
        add(0, name, "your dungeon".to_string(), *pixel);
    }
    if let Some(pixel) = request.current.filter(|_| request.current_is_project_dungeon) {
        add(0, request.dungeon_name, "this dungeon".to_string(), pixel);
    }
    for location in &request.world.locations {
        let kind = location.location_type.map_or("Location", LocationType::name);
        add(1, &location.name, format!("{kind} · {}", region_name(location.region_index)), location.pixel);
    }
    matches.sort_by(|first, second| (first.0, &first.1.name).cmp(&(second.0, &second.1.name)));
    matches.into_iter().take(MAX_SUGGESTIONS).map(|(_, suggestion)| suggestion).collect()
}

/// The map's size in map pixels.
fn map_size() -> egui::Vec2 {
    egui::vec2(MAP_WIDTH as f32, MAP_HEIGHT as f32)
}

/// The centre of `pixel`, in map pixels.
fn pixel_center(pixel: MapPixel) -> egui::Vec2 {
    egui::vec2(pixel.x as f32 + 0.5, pixel.y as f32 + 0.5)
}

/// The largest size with the map's 2:1 shape that fits in `available`.
fn fit_aspect(available: egui::Vec2) -> egui::Vec2 {
    let aspect = MAP_WIDTH as f32 / MAP_HEIGHT as f32;
    let width = available.x.min(available.y * aspect);
    egui::vec2(width, width / aspect)
}

/// Draws the world map: regions in their colours, the sea, and the locations.
fn map_image(world: &WorldMap) -> egui::ColorImage {
    let width = MAP_WIDTH as usize;
    let mut pixels = vec![SEA_COLOR; width * MAP_HEIGHT as usize];
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            if let Some(region_index) = world.region_at(MapPixel::new(x, y)) {
                pixels[y as usize * width + x as usize] = region_color(region_index);
            } else if !world.has_regions() {
                pixels[y as usize * width + x as usize] = egui::Color32::from_rgb(120, 120, 110);
            }
        }
    }
    for location in &world.locations {
        if !location.pixel.is_on_map() {
            continue;
        }
        let is_dungeon = location
            .location_type
            .is_some_and(|location_type| LocationType::DUNGEON_TYPES.contains(&location_type));
        pixels[location.pixel.y as usize * width + location.pixel.x as usize] =
            if is_dungeon { DUNGEON_COLOR } else { LOCATION_COLOR };
    }
    egui::ColorImage { size: [width, MAP_HEIGHT as usize], pixels }
}

/// A muted colour for region `region_index`, different from its neighbours in the list.
fn region_color(region_index: usize) -> egui::Color32 {
    // Spread the hues by the golden angle, varying the lightness a little too.
    let hue = (region_index as f32 * 0.618_034).fract();
    let value = if region_index.is_multiple_of(2) { 0.72 } else { 0.6 };
    egui::ecolor::Hsva::new(hue, 0.35, value, 1.0).into()
}

/// Draws a project dungeon's marker: a diamond with a dark outline, centred on `center`.
fn paint_diamond(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let points = vec![
        center + egui::vec2(0.0, -MARKER_SIZE),
        center + egui::vec2(MARKER_SIZE, 0.0),
        center + egui::vec2(0.0, MARKER_SIZE),
        center + egui::vec2(-MARKER_SIZE, 0.0),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::new(1.5_f32, MARKER_OUTLINE)));
}
