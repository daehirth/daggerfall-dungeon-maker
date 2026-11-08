# Daggerfall Dungeon Maker

A CAD-style 3D viewer and editor for the dungeons and dungeon rooms (RDB
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
daggerfall-dungeon-maker [--arena2 <folder>] [--room <name.RDB>] [--dungeon <name>]
```

## Using the viewer

* **File > Open room…** (Ctrl+O) lists all 187 RDB blocks, filterable by type:
  `N` interior, `W` interior with water, `M` interior variant, `S` special
  (quest) and `B` border caps.
* **File > Open dungeon…** (Ctrl+Shift+O) lists every pre-built dungeon in
  `MAPS.BSA`, searchable and filterable by region. The dungeon is shown with
  all of its rooms in place. Hovering a room highlights the whole room and its
  block outline; double-clicking it opens that room in a separate window with
  the full room editor (closing the previously opened room window). Changes
  made in that window appear in the dungeon view. As in the game, the exit door
  (model 70300) is only kept in the starting room, labelled **Start**; opening
  a room on its own with **Open room…** still shows all of its doors.
* The outliner (left) lists every object grouped by the block's object lists.
  The properties panel (right) edits name, visibility, position and rotation.
* **View** toggles back-face hiding (to look into rooms from outside), the grid,
  the block outline, the asset palette, and models, flats, editor markers and lights.

### Customising a room

The **asset palette** along the bottom lists everything that can be placed:

| Tab | Contents |
| --- | --- |
| Room models | Every model used in Daggerfall's dungeon rooms, most widely used first |
| All models | Every model in `ARCH3D.BSA` |
| Flats | Sprites used in rooms: torches, furniture, people, treasure |
| Markers | Editor markers: start/enter points, enemy spawns, quest and treasure markers |
| Lights | A point light |

The list is built from your own game data when the ARENA2 folder is loaded.
Click a tile to arm it; a green ghost follows the mouse over the room surfaces.
Left click places it (Shift+click to keep placing), R rotates it 90°, Esc
cancels. Placed objects appear under **Added objects** in the outliner and can
be moved, rotated, duplicated, deleted and undone like any other object.

| Input | Action |
| --- | --- |
| Click an asset tile | Arm it for placement |
| Left click while placing | Place it (Shift to keep placing) |
| R / Esc while placing | Rotate 90° / cancel |
| Left click | Select object |
| Left drag on gizmo arrow | Move along that axis (Ctrl toggles snapping) |
| Right drag | Orbit |
| Middle drag / Shift + right drag | Pan |
| Mouse wheel | Zoom |
| Right mouse + W A S D Q E | Fly (Shift for faster) |
| F / Home | Frame selection / frame room |
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
| `src/catalog.rs` | Placeable assets, gathered from the game data at run time |
| `src/camera.rs`, `src/picking.rs` | Orbit camera and ray picking |
| `src/render/` | wgpu renderer (models, billboards, helper lines) drawing into an egui texture |
| `src/app/` | egui application: main window, menus, dialogs and room editor windows |
| `src/app/room_editor/` | The room editor: outliner, properties, asset palette, viewport, undo history |
| `src/app/dungeon_view.rs` | Whole-dungeon view with room highlighting |
| `tests/real_data.rs` | Tests against a real install; set `DAGGERFALL_ARENA2` to run them |

The file formats follow [Daggerfall Unity](https://github.com/Interkarma/daggerfall-unity)
(MIT), which was used as the reference implementation.
