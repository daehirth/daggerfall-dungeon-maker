//! The project panel: every room and dungeon that can be opened, in one tree.
//!
//! The **Daggerfall** list holds the game's own dungeons (grouped by region)
//! and rooms; they open read-only. Below it are the project's own rooms and
//! dungeons: loose at the top level, or in folders the user creates.
//! Right-clicking an entry offers what can be done with it.
//!
//! Entries can also be dragged: onto a folder to move them into it, onto an
//! entry to place them just before it, or onto the space below the list to move
//! them to the top level. Dragging a Daggerfall room or dungeon into the project
//! adds a copy there.

use std::sync::Arc;

use eframe::egui;

use super::room_browser::category_description;
use crate::arena2::DungeonDefinition;
use crate::custom_names::CustomNames;
use crate::project::{ItemId, Project, ProjectEntry};

/// Most search matches listed per section, so a short search stays responsive.
const MAX_SEARCH_MATCHES: usize = 200;
/// Name given to new folders.
const NEW_FOLDER_NAME: &str = "New folder";

/// What the main window is showing, so the panel can highlight it.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenedItem {
    /// A room from Daggerfall, by RDB block name.
    DaggerfallRoom(String),
    /// A dungeon from Daggerfall, by index in the library's dungeon list.
    DaggerfallDungeon(usize),
    /// A room or dungeon of the project.
    Project(ItemId),
}

/// Something the user asked for that the application carries out.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelAction {
    Open(OpenedItem),
    /// Copy a Daggerfall room or dungeon into the project.
    CopyFromDaggerfall {
        item: OpenedItem,
        /// Folder to put the copy in; `None` for the top level.
        folder: Option<ItemId>,
        /// Open the copy (or, if the item is open, turn it into the copy) rather than just adding it.
        open: bool,
    },
    Duplicate(ItemId),
    /// Delete a room or dungeon (the application asks first).
    Delete(ItemId),
}

/// What is being dragged in the panel.
#[derive(Debug, Clone, PartialEq)]
enum DragPayload {
    /// A project room, dungeon or folder.
    Project(ItemId),
    /// A Daggerfall room or dungeon, which is copied where it is dropped.
    Daggerfall(OpenedItem),
}

/// Where a dragged entry was dropped.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DropTarget {
    /// Into a folder, or the top level for `None`.
    Into(Option<ItemId>),
    /// Just before an entry, in the same folder.
    Before(ItemId),
}

/// A change to the project tree, applied after the tree has been drawn.
enum TreeChange {
    Rename(ItemId, String),
    Move(ItemId, Option<ItemId>),
    Drop(DragPayload, DropTarget),
    NewFolder(Option<ItemId>),
    RemoveFolder(ItemId),
}

/// A region's dungeons: a range of the (region-sorted) dungeon list.
struct RegionGroup {
    name: &'static str,
    dungeons: std::ops::Range<usize>,
}

/// State of the project panel.
pub struct ProjectPanel {
    /// Whether the panel is shown.
    pub is_open: bool,
    search_text: String,
    /// Entry being renamed and the name typed so far.
    renaming: Option<(ItemId, String)>,
    /// Set when renaming starts, to put the cursor in the name field.
    focus_rename_field: bool,
    room_names: Vec<String>,
    dungeons: Arc<[DungeonDefinition]>,
    regions: Vec<RegionGroup>,
}

impl Default for ProjectPanel {
    fn default() -> Self {
        Self {
            is_open: true,
            search_text: String::new(),
            renaming: None,
            focus_rename_field: false,
            room_names: Vec::new(),
            dungeons: Arc::new([]),
            regions: Vec::new(),
        }
    }
}

impl ProjectPanel {
    /// Sets the Daggerfall rooms and dungeons listed, after the game data is loaded.
    ///
    /// `dungeons` must be sorted by region, as the library returns them.
    pub fn set_daggerfall_contents(&mut self, room_names: Vec<String>, dungeons: Arc<[DungeonDefinition]>) {
        self.regions.clear();
        for (index, dungeon) in dungeons.iter().enumerate() {
            match self.regions.last_mut() {
                Some(region) if region.name == dungeon.region_name => region.dungeons.end = index + 1,
                _ => self.regions.push(RegionGroup { name: dungeon.region_name, dungeons: index..index + 1 }),
            }
        }
        self.room_names = room_names;
        self.dungeons = dungeons;
    }

    /// Starts renaming the entry `id`, for example a folder that was just created.
    pub fn start_renaming(&mut self, id: ItemId, current_name: &str) {
        self.renaming = Some((id, current_name.to_string()));
        self.focus_rename_field = true;
    }

    /// Draws the panel on the left of the main window. Returns what the user asked for, if anything.
    pub fn show(
        &mut self,
        context: &egui::Context,
        project: &mut Project,
        project_title: &str,
        custom_names: &CustomNames,
        opened: Option<&OpenedItem>,
    ) -> Option<PanelAction> {
        if !self.is_open {
            return None;
        }
        let mut action = None;
        let mut changes = Vec::new();

        egui::SidePanel::left("project_panel").resizable(true).default_width(250.0).show(context, |ui| {
            ui.horizontal(|ui| {
                ui.heading(project_title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("New folder").on_hover_text("Add a folder for organising the project").clicked() {
                        changes.push(TreeChange::NewFolder(None));
                    }
                });
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.search_text)
                    .hint_text("Search cells and dungeons")
                    .desired_width(f32::INFINITY),
            );
            ui.separator();

            let search = self.search_text.trim().to_lowercase();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.show_daggerfall_list(ui, &search, custom_names, opened, &mut action);
                let folders = project.folders();
                let context = TreeContext { search: &search, opened, folders: &folders, project };
                for entry in folders_first(&project.entries) {
                    self.show_entry(ui, entry, &context, &mut action, &mut changes);
                }
                if project.is_empty() && search.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "Your cells and dungeons appear here. Edit anything from Daggerfall to make a copy.",
                        )
                        .weak(),
                    );
                }
                // Leave room below the last entry to right-click, or to drop entries at the top level.
                let rest = ui.allocate_response(
                    egui::vec2(ui.available_width(), ui.available_height().max(40.0)),
                    egui::Sense::click(),
                );
                drop_target(ui, &rest, DropTarget::Into(None), DropHighlight::Area, &mut changes);
                rest.context_menu(|ui| {
                    if ui.button("New folder").clicked() {
                        changes.push(TreeChange::NewFolder(None));
                        ui.close_menu();
                    }
                });
            });
        });

        show_drag_preview(context, project, &self.dungeons);
        for change in changes {
            if let TreeChange::Drop(DragPayload::Daggerfall(item), target) = change {
                let folder = match target {
                    DropTarget::Into(folder) => folder,
                    DropTarget::Before(entry) => project.parent_of(entry).flatten(),
                };
                action = Some(PanelAction::CopyFromDaggerfall { item, folder, open: false });
                continue;
            }
            self.apply_change(project, change);
        }
        action
    }

    /// Carries out a change to the project tree.
    fn apply_change(&mut self, project: &mut Project, change: TreeChange) {
        match change {
            TreeChange::Rename(id, name) => {
                if let Some(entry) = project.find_mut(id) {
                    *entry.name_mut() = name;
                }
            }
            TreeChange::Move(id, parent)
            | TreeChange::Drop(DragPayload::Project(id), DropTarget::Into(parent)) => {
                project.move_entry(id, parent);
            }
            TreeChange::Drop(DragPayload::Project(id), DropTarget::Before(target)) => {
                project.move_before(id, target);
            }
            // Copies of Daggerfall items are made by the application, in `show`.
            TreeChange::Drop(DragPayload::Daggerfall(_), _) => {}
            TreeChange::NewFolder(parent) => {
                let id = project.add_folder(NEW_FOLDER_NAME, parent);
                self.start_renaming(id, NEW_FOLDER_NAME);
            }
            TreeChange::RemoveFolder(id) => {
                project.remove_folder_keeping_contents(id);
            }
        }
    }

    /// Draws the read-only list of Daggerfall's own dungeons and rooms.
    fn show_daggerfall_list(
        &self,
        ui: &mut egui::Ui,
        search: &str,
        custom_names: &CustomNames,
        opened: Option<&OpenedItem>,
        action: &mut Option<PanelAction>,
    ) {
        if self.room_names.is_empty() {
            egui::CollapsingHeader::new("Daggerfall").id_salt("daggerfall_list").default_open(true).show(
                ui,
                |ui| {
                    ui.label(
                        egui::RichText::new("Choose your ARENA2 folder to list the game's cells.").weak(),
                    );
                },
            );
            return;
        }

        // Search results are a flat list, so the folded state of the lists is left as the user had it.
        if !search.is_empty() {
            ui.strong("Daggerfall");
            ui.indent("daggerfall_matches", |ui| {
                ui.label("Dungeons");
                let dungeon_matches = (0..self.dungeons.len()).filter(|&index| {
                    let dungeon = &self.dungeons[index];
                    dungeon.name.to_lowercase().contains(search)
                        || dungeon.region_name.to_lowercase().contains(search)
                });
                ui.indent("daggerfall_dungeon_matches", |ui| {
                    show_limited(ui, dungeon_matches, |ui, index| {
                        self.show_daggerfall_dungeon(ui, index, true, opened, action);
                    });
                });
                ui.label("Cells");
                let room_matches = self.room_names.iter().filter(|name| {
                    name.to_lowercase().contains(search) || custom_names.room_name_matches(name, search)
                });
                ui.indent("daggerfall_room_matches", |ui| {
                    show_limited(ui, room_matches, |ui, name| {
                        show_daggerfall_room(ui, name, custom_names, opened, action);
                    });
                });
            });
            return;
        }

        egui::CollapsingHeader::new("Daggerfall").id_salt("daggerfall_list").default_open(true).show(
            ui,
            |ui| {
                let dungeon_header = format!("Dungeons ({})", self.dungeons.len());
                egui::CollapsingHeader::new(dungeon_header).id_salt("daggerfall_dungeons").show(ui, |ui| {
                    for region in &self.regions {
                        let header = format!("{} ({})", region.name, region.dungeons.len());
                        egui::CollapsingHeader::new(header).id_salt(("daggerfall_region", region.name)).show(
                            ui,
                            |ui| {
                                for index in region.dungeons.clone() {
                                    self.show_daggerfall_dungeon(ui, index, false, opened, action);
                                }
                            },
                        );
                    }
                });
                let room_header = format!("Cells ({})", self.room_names.len());
                egui::CollapsingHeader::new(room_header).id_salt("daggerfall_rooms").show(ui, |ui| {
                    for name in &self.room_names {
                        show_daggerfall_room(ui, name, custom_names, opened, action);
                    }
                });
            },
        );
    }

    /// Draws one Daggerfall dungeon row; `show_region` adds its region, for search results.
    fn show_daggerfall_dungeon(
        &self,
        ui: &mut egui::Ui,
        index: usize,
        show_region: bool,
        opened: Option<&OpenedItem>,
        action: &mut Option<PanelAction>,
    ) {
        let dungeon = &self.dungeons[index];
        let label = if show_region {
            format!("{}   ·   {}", dungeon.name, dungeon.region_name)
        } else {
            dungeon.name.clone()
        };
        let item = OpenedItem::DaggerfallDungeon(index);
        let response = ui.selectable_label(opened == Some(&item), label).on_hover_text(format!(
            "{} · {} cells\nDouble-click to open (read-only), or drag into the project to copy it",
            dungeon.dungeon_type.map_or("Unknown type", |dungeon_type| dungeon_type.name()),
            dungeon.blocks.len()
        ));
        daggerfall_item_interaction(&response, item, action);
    }

    /// Draws a project entry, and a folder's contents.
    fn show_entry(
        &mut self,
        ui: &mut egui::Ui,
        entry: &ProjectEntry,
        context: &TreeContext<'_>,
        action: &mut Option<PanelAction>,
        changes: &mut Vec<TreeChange>,
    ) {
        if !entry_matches(entry, context.search) {
            return;
        }
        let id = entry.id();
        if self.renaming.as_ref().is_some_and(|(renamed, _)| *renamed == id) {
            self.show_rename_field(ui, changes);
            if let ProjectEntry::Folder(folder) = entry {
                ui.indent(("project_folder_renaming", id), |ui| {
                    for child in folders_first(&folder.entries) {
                        self.show_entry(ui, child, context, action, changes);
                    }
                });
            }
            return;
        }

        match entry {
            ProjectEntry::Folder(folder) => {
                let searching = !context.search.is_empty();
                let response = egui::CollapsingHeader::new(&folder.name)
                    .id_salt(("project_folder", id))
                    .default_open(true)
                    .open(searching.then_some(true))
                    .show(ui, |ui| {
                        if folder.entries.is_empty() {
                            ui.label(egui::RichText::new("Empty").weak());
                        }
                        for child in folders_first(&folder.entries) {
                            self.show_entry(ui, child, context, action, changes);
                        }
                    });
                let header = drag_source(&response.header_response, DragPayload::Project(id));
                drop_target(ui, &header, DropTarget::Into(Some(id)), DropHighlight::Area, changes);
                header.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        self.start_renaming(id, &folder.name);
                        ui.close_menu();
                    }
                    if ui.button("New folder inside").clicked() {
                        changes.push(TreeChange::NewFolder(Some(id)));
                        ui.close_menu();
                    }
                    move_to_menu(ui, id, context, changes);
                    ui.separator();
                    if ui.button("Remove folder").on_hover_text("Its contents move up a level").clicked() {
                        changes.push(TreeChange::RemoveFolder(id));
                        ui.close_menu();
                    }
                });
            }
            ProjectEntry::Room(_) | ProjectEntry::Dungeon(_) => {
                let (kind, details) = match entry {
                    ProjectEntry::Room(room) => {
                        let mut details = room
                            .based_on
                            .as_deref()
                            .map(|source| format!("Copy of {source}"))
                            .unwrap_or_default();
                        let users = context.project.dungeons_using(id);
                        if !users.is_empty() {
                            details.push_str(&format!("\nUsed in {}", users.join(", ")));
                        }
                        ("cell", details)
                    }
                    ProjectEntry::Dungeon(dungeon) => {
                        let dungeon_type =
                            dungeon.dungeon_type.map_or("Unknown type", |dungeon_type| dungeon_type.name());
                        let details = format!(
                            "{dungeon_type} · {} cells · {}",
                            dungeon.rooms.len(),
                            dungeon.region_name
                        );
                        ("dungeon", details)
                    }
                    ProjectEntry::Folder(_) => unreachable!("folders are handled above"),
                };
                let item = OpenedItem::Project(id);
                let response = ui
                    .horizontal(|ui| {
                        let response = ui.selectable_label(context.opened == Some(&item), entry.name());
                        ui.label(egui::RichText::new(kind).weak().small());
                        response
                    })
                    .inner
                    .on_hover_text(format!(
                        "{details}\nDouble-click to open, right-click for more, drag to move"
                    ));
                let response = drag_source(&response, DragPayload::Project(id));
                drop_target(ui, &response, DropTarget::Before(id), DropHighlight::LineAbove, changes);
                if response.double_clicked() {
                    *action = Some(PanelAction::Open(item.clone()));
                }
                response.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        *action = Some(PanelAction::Open(item.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Rename").clicked() {
                        self.start_renaming(id, entry.name());
                        ui.close_menu();
                    }
                    if ui.button("Duplicate").clicked() {
                        *action = Some(PanelAction::Duplicate(id));
                        ui.close_menu();
                    }
                    move_to_menu(ui, id, context, changes);
                    ui.separator();
                    if ui.button("Delete…").clicked() {
                        *action = Some(PanelAction::Delete(id));
                        ui.close_menu();
                    }
                });
            }
        }
    }

    /// Draws the name field of the entry being renamed. Enter or clicking away keeps the name; Escape cancels.
    fn show_rename_field(&mut self, ui: &mut egui::Ui, changes: &mut Vec<TreeChange>) {
        let Some((id, name)) = self.renaming.as_mut() else { return };
        let response = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        if std::mem::take(&mut self.focus_rename_field) {
            response.request_focus();
        }
        if response.lost_focus() {
            let cancelled = ui.input(|input| input.key_pressed(egui::Key::Escape));
            if !cancelled && !name.trim().is_empty() {
                changes.push(TreeChange::Rename(*id, name.trim().to_string()));
            }
            self.renaming = None;
        }
    }
}

/// What every project entry row needs to know.
struct TreeContext<'tree> {
    search: &'tree str,
    opened: Option<&'tree OpenedItem>,
    folders: &'tree [crate::project::FolderListing],
    project: &'tree Project,
}

/// Draws one Daggerfall room row, with the user's name for the room if it has one.
fn show_daggerfall_room(
    ui: &mut egui::Ui,
    name: &str,
    custom_names: &CustomNames,
    opened: Option<&OpenedItem>,
    action: &mut Option<PanelAction>,
) {
    let label = match custom_names.room_name(name) {
        Some(custom_name) => format!("{name}   {custom_name}"),
        None => name.to_string(),
    };
    let item = OpenedItem::DaggerfallRoom(name.to_string());
    let response = ui.selectable_label(opened == Some(&item), label).on_hover_text(format!(
        "{}\nDouble-click to open (read-only), or drag into the project to copy it",
        category_description(name)
    ));
    daggerfall_item_interaction(&response, item, action);
}

/// Double-click opens a Daggerfall item; right-click offers to open or copy it.
fn daggerfall_item_interaction(
    response: &egui::Response,
    item: OpenedItem,
    action: &mut Option<PanelAction>,
) {
    let response = drag_source(response, DragPayload::Daggerfall(item.clone()));
    if response.double_clicked() {
        *action = Some(PanelAction::Open(item.clone()));
    }
    response.context_menu(|ui| {
        if ui.button("Open (read-only)").clicked() {
            *action = Some(PanelAction::Open(item.clone()));
            ui.close_menu();
        }
        if ui.button("Create a copy").on_hover_text("Add an editable copy to the project").clicked() {
            *action = Some(PanelAction::CopyFromDaggerfall { item: item.clone(), folder: None, open: true });
            ui.close_menu();
        }
    });
}

/// Adds a "Move to" submenu listing the top level and every folder `id` can move into.
fn move_to_menu(ui: &mut egui::Ui, id: ItemId, context: &TreeContext<'_>, changes: &mut Vec<TreeChange>) {
    let current_parent = context.project.parent_of(id).flatten();
    ui.menu_button("Move to", |ui| {
        if ui.add_enabled(current_parent.is_some(), egui::Button::new("Top level")).clicked() {
            changes.push(TreeChange::Move(id, None));
            ui.close_menu();
        }
        let moved = context.project.find(id);
        for folder in context.folders {
            // A folder cannot move into itself or one of its own folders.
            let inside_moved = moved.is_some_and(|entry| match entry {
                ProjectEntry::Folder(_) => {
                    folder.id == id || context.project.parent_chain(folder.id).contains(&id)
                }
                _ => false,
            });
            if inside_moved {
                continue;
            }
            let enabled = current_parent != Some(folder.id);
            if ui.add_enabled(enabled, egui::Button::new(&folder.path)).clicked() {
                changes.push(TreeChange::Move(id, Some(folder.id)));
                ui.close_menu();
            }
        }
    });
}

/// Whether `entry`, or anything inside it, matches the lowercase `search`.
fn entry_matches(entry: &ProjectEntry, search: &str) -> bool {
    if search.is_empty() || entry.name().to_lowercase().contains(search) {
        return true;
    }
    match entry {
        ProjectEntry::Folder(folder) => folder.entries.iter().any(|child| entry_matches(child, search)),
        _ => false,
    }
}

/// Draws up to [`MAX_SEARCH_MATCHES`] rows, then says how many more there are.
fn show_limited<Item>(
    ui: &mut egui::Ui,
    items: impl Iterator<Item = Item>,
    mut show_row: impl FnMut(&mut egui::Ui, Item),
) {
    let mut count = 0;
    for item in items {
        if count < MAX_SEARCH_MATCHES {
            show_row(ui, item);
        }
        count += 1;
    }
    if count == 0 {
        ui.label(egui::RichText::new("No matches").weak());
    } else if count > MAX_SEARCH_MATCHES {
        ui.label(egui::RichText::new(format!("…and {} more", count - MAX_SEARCH_MATCHES)).weak());
    }
}

/// The entries of a list with folders before rooms and dungeons, each in their stored order.
fn folders_first(entries: &[ProjectEntry]) -> impl Iterator<Item = &ProjectEntry> {
    let folders = entries.iter().filter(|entry| entry.is_folder());
    folders.chain(entries.iter().filter(|entry| !entry.is_folder()))
}

/// How a drop target shows that something would be dropped on it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DropHighlight {
    /// Outline the whole target.
    Area,
    /// Draw a line above the target, where the entry would be inserted.
    LineAbove,
}

/// Lets `response`'s widget be dragged, carrying `payload`. Clicks still work as before.
fn drag_source(response: &egui::Response, payload: DragPayload) -> egui::Response {
    let response = response.interact(egui::Sense::drag());
    response.dnd_set_drag_payload(payload);
    response
}

/// Highlights `response` while something is dragged over it, and records a drop on it.
fn drop_target(
    ui: &egui::Ui,
    response: &egui::Response,
    target: DropTarget,
    highlight: DropHighlight,
    changes: &mut Vec<TreeChange>,
) {
    if response.dnd_hover_payload::<DragPayload>().is_some() {
        let stroke = egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color);
        let rect = response.rect;
        match highlight {
            DropHighlight::Area => {
                ui.painter().rect_stroke(rect.expand(1.0), 3.0, stroke);
            }
            DropHighlight::LineAbove => {
                ui.painter().hline(rect.x_range(), rect.top() - 1.0, stroke);
            }
        }
    }
    if let Some(payload) = response.dnd_release_payload::<DragPayload>() {
        changes.push(TreeChange::Drop((*payload).clone(), target));
    }
}

/// Shows the name of the dragged entry next to the pointer.
fn show_drag_preview(context: &egui::Context, project: &Project, dungeons: &[DungeonDefinition]) {
    let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(context) else { return };
    let Some(pointer) = context.pointer_interact_pos() else { return };
    let name = match &*payload {
        DragPayload::Project(id) => project.find(*id).map(|entry| entry.name().to_string()),
        DragPayload::Daggerfall(OpenedItem::DaggerfallRoom(name)) => Some(format!("Copy of {name}")),
        DragPayload::Daggerfall(OpenedItem::DaggerfallDungeon(index)) => {
            dungeons.get(*index).map(|dungeon| format!("Copy of {}", dungeon.name))
        }
        DragPayload::Daggerfall(OpenedItem::Project(_)) => None,
    };
    let Some(name) = name else { return };
    egui::Area::new(egui::Id::new("project_drag_preview"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + egui::vec2(14.0, 10.0))
        .interactable(false)
        .show(context, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(name));
        });
}
