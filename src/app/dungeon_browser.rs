//! The "Open dungeon" window listing every pre-built dungeon in `MAPS.BSA`.

use eframe::egui;

use crate::arena2::DungeonDefinition;

/// State of the dungeon browser window.
#[derive(Default)]
pub struct DungeonBrowser {
    /// Whether the window is shown.
    pub is_open: bool,
    /// Text filter applied to dungeon and region names.
    search_text: String,
    /// Region filter; `None` shows every region.
    region_filter: Option<&'static str>,
    /// Index (into the dungeon list) of the highlighted dungeon.
    highlighted_dungeon: Option<usize>,
}

impl DungeonBrowser {
    /// Draws the browser over `dungeons`. Returns the index of the dungeon the user chose to open.
    pub fn show(&mut self, context: &egui::Context, dungeons: &[DungeonDefinition]) -> Option<usize> {
        if !self.is_open {
            return None;
        }

        let mut chosen_dungeon = None;
        let mut is_open = self.is_open;
        egui::Window::new("Open dungeon")
            .open(&mut is_open)
            .default_size([460.0, 560.0])
            .collapsible(false)
            .show(context, |ui| {
                self.show_filters(ui, dungeons);
                ui.separator();

                let search = self.search_text.trim().to_lowercase();
                let visible_dungeons: Vec<(usize, &DungeonDefinition)> = dungeons
                    .iter()
                    .enumerate()
                    .filter(|(_, dungeon)| {
                        self.region_filter.is_none_or(|region| dungeon.region_name == region)
                    })
                    .filter(|(_, dungeon)| {
                        search.is_empty()
                            || dungeon.name.to_lowercase().contains(&search)
                            || dungeon.region_name.to_lowercase().contains(&search)
                    })
                    .collect();

                let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(ui.available_height() - 36.0)
                    .show_rows(ui, row_height, visible_dungeons.len(), |ui, row_range| {
                        for &(dungeon_index, dungeon) in &visible_dungeons[row_range] {
                            let is_highlighted = self.highlighted_dungeon == Some(dungeon_index);
                            let label = format!(
                                "{}   ·   {}   ·   {} cells",
                                dungeon.name,
                                dungeon.region_name,
                                dungeon.blocks.len()
                            );
                            let response = ui.selectable_label(is_highlighted, label);
                            if response.clicked() {
                                self.highlighted_dungeon = Some(dungeon_index);
                            }
                            if response.double_clicked() {
                                chosen_dungeon = Some(dungeon_index);
                            }
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("{} dungeons", visible_dungeons.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_open = self.highlighted_dungeon.is_some();
                        if ui.add_enabled(can_open, egui::Button::new("Open")).clicked() {
                            chosen_dungeon = self.highlighted_dungeon;
                        }
                    });
                });
            });

        self.is_open = is_open && chosen_dungeon.is_none();
        chosen_dungeon
    }

    /// Draws the search box and region drop-down.
    fn show_filters(&mut self, ui: &mut egui::Ui, dungeons: &[DungeonDefinition]) {
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add(egui::TextEdit::singleline(&mut self.search_text).hint_text("Dungeon or region name"));
        });
        ui.horizontal(|ui| {
            ui.label("Region");
            let selected_text = self.region_filter.unwrap_or("All regions");
            egui::ComboBox::from_id_salt("dungeon_region_filter")
                .selected_text(selected_text)
                .width(240.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.region_filter, None, "All regions");
                    let mut regions: Vec<&'static str> =
                        dungeons.iter().map(|dungeon| dungeon.region_name).collect();
                    regions.sort_unstable();
                    regions.dedup();
                    for region in regions {
                        ui.selectable_value(&mut self.region_filter, Some(region), region);
                    }
                });
        });
    }
}
