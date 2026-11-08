# Daggerfall Dungeon Maker

A CAD-style 3D viewer and editor for the dungeons and dungeon cells (RDB
blocks) of *The Elder Scrolls II: Daggerfall*, written in Rust.

## Game data

No game data is included in or generated into this repository. On first launch
the program asks for your Daggerfall `ARENA2` folder (for example
`.../steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2`) and reads
`ARCH3D.BSA`, `BLOCKS.BSA`, `MAPS.BSA` (optional, for dungeons), `ART_PAL.COL`
and the `TEXTURE.###` files from it at run time. File names may be upper or
lower case. Common Steam and GOG install locations are detected automatically.

Only the folder path is remembered between runs, in the app's settings
(`%APPDATA%\Daggerfall Dungeon Maker` on Windows,
`~/.local/share/daggerfalldungeonmaker` on Linux); change it with
**File > Set ARENA2 folder…**.

## Building

The program runs on Windows and Linux.

### Windows

```powershell
.uild.ps1
```

This runs `cargo build --release` and produces
`target\release\daggerfall-dungeon-maker.exe`. With the GNU Rust toolchain, MinGW-w64
must be installed (`scoop install mingw`); see `build.ps1` for details.

### Linux

Install Rust from <https://rustup.rs> and the system libraries used for the
window, file dialogs and graphics. On Debian or Ubuntu:

```sh
sudo apt install build-essential pkg-config libgtk-3-dev libxkbcommon-dev   libwayland-dev libx11-dev mesa-vulkan-drivers
```

Then build:

```sh
./build.sh
```

This produces `target/release/daggerfall-dungeon-maker`. A Vulkan or OpenGL
capable graphics driver is needed to run it. Both X11 and Wayland desktops work.

### Command line

Optional command line arguments:

```text
daggerfall-dungeon-maker [project.ddm] [--arena2 <folder>] [--cell <name.RDB>] [--dungeon <name>]
```

## Using the viewer

* The **project panel** (left) lists everything that can be opened. The
  **Daggerfall** list holds the game's own dungeons, grouped by region, and
  cells; below it are the project's own cells and dungeons. Double-click an
  entry to open it; the search box filters every list.
* **File > Open cell…** (Ctrl+O) lists all 187 RDB blocks, filterable by type:
  `N` interior, `W` interior with water, `M` interior variant, `S` special
  (quest) and `B` border caps.
* **File > Open dungeon…** (Ctrl+Shift+O) lists every pre-built dungeon in
  `MAPS.BSA`, searchable and filterable by region. The dungeon is shown with
  all of its cells in place. Hovering a cell highlights the whole cell and its
  block outline; double-clicking it opens that cell in a separate window with
  the full cell editor (closing the previously opened cell window). Changes
  made in that window appear in the dungeon view. As in the game, the exit door
  (model 70300) is only kept in the starting cell, labelled **Start**; opening
  a cell on its own with **Open cell…** still shows all of its doors.
* The dungeon view's properties panel (right) shows the dungeon's name, type,
  region and cells. Hovering a cell in its list highlights it, and
  double-clicking opens it. The name and type of a project dungeon can be
  changed there; the type decides which enemies its random enemy markers become.
* The outliner (left) lists every object grouped by the block's object lists.
  The properties panel (right) edits name, visibility, position and rotation.
* **View** toggles back-face hiding (to look into cells from outside), the grid,
  the block outline, the asset palette, and models, flats, editor markers and lights.

### Projects

Cells and dungeons from Daggerfall open read-only. Trying to change one (moving,
placing, deleting or editing an object) shows *You cannot edit cells from
Daggerfall* (or *dungeons*) with **Create a copy** and **Cancel**. A copy is
named after the original with `-Copy` added, for example `N0000004-Copy` or
`Privateer's Hold-Copy`, and opens in place so the change can be made again.
Right-clicking a Daggerfall entry also offers **Create a copy**.

The cells in a dungeon are Daggerfall's own cells too, so they stay read-only
even in a copied dungeon. Changing a cell in a Daggerfall dungeon copies both
the dungeon and that cell (*You cannot edit dungeons from Daggerfall*);
changing a Daggerfall cell in a copied dungeon copies just the cell for that
dungeon. The cell copy is an ordinary project cell the dungeon now uses, and
each such cell belongs to one dungeon: duplicating the dungeon duplicates its
cells too, and deleting the cell puts the Daggerfall cell back.

Copies belong to the project and are listed loose at the top level of the
project panel. Right-click them to open, rename, duplicate, move or delete them,
and use **New folder** (or right-click a folder) to organise them into folders,
which can be nested and renamed.

Entries can also be dragged: onto a folder to move them into it, onto another
entry to place them just before it, or onto the empty space below the list to
move them to the top level. Dragging a Daggerfall cell or dungeon into the
project adds a copy where it is dropped.

**File > Save project** (Ctrl+S) and **Save project as…** (Ctrl+Shift+S) write
a `.ddm` project file; **Open project…** and **New project** switch projects,
asking first if there are unsaved changes, as closing the program does. The last
project is reopened on the next run, and a `.ddm` file can be passed on the
command line. Project files are JSON. A dungeon refers to the project cells it
uses by id; its other cells are read from the game data when it is opened.

### Enemy markers

Random enemy markers (`TEXTURE.199` record 15) do not name an enemy: the game
picks one from a table for the dungeon's type (crypt, orc stronghold, vampire
haunt, …) and the player's level. Markers below the cell's water level use a
separate underwater table (mostly slaughterfish and dreugh) in every dungeon;
the water level comes from the cell's start marker, as in the game. Every
dungeon's type is read from `MAPS.BSA` and shown in its title and tooltip.
Selecting a random enemy marker lists the enemies it can become, with their
chances for a player level you choose (type it, drag it or scroll over it);
hovering one shows the likeliest three. Above water this needs the cell to be
opened from a dungeon. The chances follow the original game's dungeon pick,
which Daggerfall Unity uses by default and which gives the same enemies each
visit for a given dungeon and player level. The tables are built into the
original executable rather than the data files, so they are taken from
Daggerfall Unity (`src/encounters.rs`).

Fixed enemy markers (record 16) store their enemy in the cell itself. Selecting
one shows what it spawns, whether it is hostile or passive (attacks only when
provoked) and, for human classes, its gender; hovering one shows the enemy too.
The same enemy appears in every dungeon that uses the cell. In a project cell
all three can be changed under **Spawns**, and **Nothing** switches the marker
off without deleting it. Fixed enemy markers placed from the asset palette start
as a rat (enemy 0).

### Customising a cell

The **asset palette** along the bottom lists everything that can be placed:

| Tab | Contents |
| --- | --- |
| Cell models | Every model used in Daggerfall's dungeon cells, most widely used first |
| All models | Every model in `ARCH3D.BSA` |
| Flats | Sprites used in cells: torches, furniture, people, treasure |
| Markers | Editor markers: start/enter points, enemy spawns, quest and treasure markers |
| Lights | A point light |

The list is built from your own game data when the ARENA2 folder is loaded.
Click a tile to arm it; a green ghost follows the mouse over the cell surfaces.
Left click places it (Shift+click to keep placing), R rotates it 90°, Esc
cancels. **Grid lock** on the menu bar (or G) snaps objects to the grid while they are
moved or placed: the position along the dragged axis lands on a grid line,
which is drawn faintly at the chosen step (0.4, 0.8, 1.6 or 3.2 m, that is 16,
32, 64 or 128 game units). Placed objects appear under **Added objects** in the outliner and can
be moved, rotated, duplicated, deleted and undone like any other object.

| Input | Action |
| --- | --- |
| Click an asset tile | Arm it for placement |
| Left click while placing | Place it (Shift to keep placing) |
| R / Esc while placing | Rotate 90° / cancel |
| Left click | Select object |
| Left drag on gizmo arrow | Move along that axis (Ctrl switches grid lock for that drag) |
| G | Turn grid lock on or off |
| Right drag | Orbit |
| Middle drag / Shift + right drag | Pan |
| Mouse wheel | Zoom |
| Right mouse + W A S D Q E | Fly (Shift for faster) |
| F / Home | Frame selection / frame cell |
| R / Shift + R | Rotate selection 90° |
| Ctrl + D / Delete | Duplicate / delete selection |
| Ctrl + Z / Ctrl + Y | Undo / redo |

Edits are kept in memory only; saving changes back to a file is not
implemented yet.

## Code layout

| Path | Purpose |
| --- | --- |
| `src/arena2/` | Readers for BSA archives, ARCH3D models, RDB blocks, textures and palettes |
| `src/scene.rs` | Editable object list built from an RDB block |
| `src/project.rs` | Project files (`.ddm`): the user's cells, dungeons and folders |
| `src/encounters.rs` | Dungeon types and the enemies random enemy markers can become |
| `src/catalog.rs` | Placeable assets, gathered from the game data at run time |
| `src/camera.rs`, `src/picking.rs` | Orbit camera and ray picking |
| `src/render/` | wgpu renderer (models, billboards, helper lines) drawing into an egui texture |
| `src/app/` | egui application: main window, menus, dialogs and cell editor windows |
| `src/app/project_panel.rs` | The project panel: Daggerfall's cells and dungeons and the project tree |
| `src/app/room_editor/` | The cell editor: outliner, properties, asset palette, viewport, undo history |
| `src/app/dungeon_view.rs` | Whole-dungeon view with cell highlighting |
| `tests/real_data.rs` | Tests against a real install; set `DAGGERFALL_ARENA2` to run them |

The file formats follow [Daggerfall Unity](https://github.com/Interkarma/daggerfall-unity)
(MIT), which was used as the reference implementation.
