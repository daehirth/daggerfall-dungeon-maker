//! The "Open room" window listing every RDB block in `BLOCKS.BSA`.
//!
//! Rooms can be given names of the user's own here, which the editor then
//! shows wherever the room appears.

use eframe::egui;

use crate::custom_names::CustomNames;

/// Block categories, keyed by the first letter of the block name.
const ROOM_CATEGORIES: [(char, &str); 5] = [
    ('N', "Interior"),
    ('W', "Interior with water"),
    ('M', "Interior variant"),
    ('S', "Special (quest)"),
    ('B', "Border cap"),
];

/// State of the room browser window.
#[derive(Default)]
pub struct RoomBrowser {
    /// Whether the window is shown.
    pub is_open: bool,
    /// Every RDB block name, loaded when the ARENA2 folder is opened.
    room_names: Vec<String>,
    /// Text filter typed by the user.
    search_text: String,
    /// Category filter; `None` shows every category.
    category_filter: Option<char>,
    /// Name highlighted in the list.
    highlighted_room: Option<String>,
}

impl RoomBrowser {
    /// Replaces the list of rooms, for example after a new ARENA2 folder is chosen.
    pub fn set_room_names(&mut self, room_names: Vec<String>) {
        self.room_names = room_names;
        self.highlighted_room = None;
    }

    /// Draws the browser. Returns the name of the room the user chose to open.
    ///
    /// The highlighted room can be renamed in `custom_names`.
    pub fn show(&mut self, context: &egui::Context, custom_names: &mut CustomNames) -> Option<String> {
        if !self.is_open {
            return None;
        }

        let mut chosen_room = None;
        let mut is_open = self.is_open;
        egui::Window::new("Open room")
            .open(&mut is_open)
            .default_size([360.0, 480.0])
            .collapsible(false)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search_text)
                            .hint_text("e.g. N0000004 or your room name"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.category_filter, None, "All");
                    for (letter, description) in ROOM_CATEGORIES {
                        ui.selectable_value(&mut self.category_filter, Some(letter), format!("{letter}"))
                            .on_hover_text(description);
                    }
                });
                ui.separator();

                let search = self.search_text.trim().to_lowercase();
                let visible_rooms: Vec<&String> = self
                    .room_names
                    .iter()
                    .filter(|name| self.category_filter.is_none_or(|letter| name.starts_with(letter)))
                    .filter(|name| {
                        search.is_empty()
                            || name.to_lowercase().contains(&search)
                            || custom_names.room_name_matches(name, &search)
                    })
                    .collect();

                let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(ui.available_height() - 64.0)
                    .show_rows(ui, row_height, visible_rooms.len(), |ui, row_range| {
                        for name in &visible_rooms[row_range] {
                            let is_highlighted = self.highlighted_room.as_deref() == Some(name.as_str());
                            let label = match custom_names.room_name(name) {
                                Some(custom_name) => {
                                    format!("{name}   {custom_name}   ({})", category_description(name))
                                }
                                None => format!("{name}   {}", category_description(name)),
                            };
                            let response = ui.selectable_label(is_highlighted, label);
                            if response.clicked() {
                                self.highlighted_room = Some((*name).clone());
                            }
                            if response.double_clicked() {
                                chosen_room = Some((*name).clone());
                            }
                        }
                    });

                ui.separator();
                if let Some(highlighted) = self.highlighted_room.clone() {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        show_room_name_field(ui, &highlighted, custom_names);
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(format!("{} rooms", visible_rooms.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_open = self.highlighted_room.is_some();
                        if ui.add_enabled(can_open, egui::Button::new("Open")).clicked() {
                            chosen_room = self.highlighted_room.clone();
                        }
                    });
                });
            });

        self.is_open = is_open && chosen_room.is_none();
        chosen_room
    }
}

/// Draws a text field that edits the user's name for the room `block_name`.
pub fn show_room_name_field(ui: &mut egui::Ui, block_name: &str, custom_names: &mut CustomNames) {
    let mut name = custom_names.room_name_for_editing(block_name);
    let field = egui::TextEdit::singleline(&mut name).hint_text("Your name for this room (editor only)");
    if ui.add(field).changed() {
        custom_names.set_room_name(block_name, &name);
    }
}

/// Human readable category of a room, based on its name's first letter.
pub fn category_description(room_name: &str) -> &'static str {
    let first_letter = room_name.chars().next().unwrap_or(' ');
    ROOM_CATEGORIES
        .iter()
        .find(|(letter, _)| *letter == first_letter)
        .map_or("", |(_, description)| description)
}
