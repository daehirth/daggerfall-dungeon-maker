# Daggerfall Dungeon Maker

A CAD-style 3D viewer and editor for the dungeons and dungeon cells (RDB
blocks) of *The Elder Scrolls II: Daggerfall*, written in Rust.

## Game data

No game data is included in or generated into this repository. On first launch
the program asks for your Daggerfall `ARENA2` folder (for example
`.../steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2`) and reads
`ARCH3D.BSA`, `BLOCKS.BSA`, `MAPS.BSA` (optional, for dungeons and the world
map), `POLITIC.PAK` (optional, for the regions on the world map), `ART_PAL.COL`
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
./build.ps1
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

* The **project panel** (left) lists everything that can be opened, one kind
  at a time: **Dungeons** or **Cells**, chosen at the top. The **Daggerfall**
  toggle beside them shows or hides the game's own (dungeons grouped by
  region); below them are the project's own, in their folders. Opening
  something of the other kind switches to its section. Click a dungeon or
  double-click a cell to open it; the search box filters the list shown. The
  section and toggle are remembered between runs.
* The **+** beside the search box creates an empty dungeon (in Dungeons) or an
  empty cell (in Cells) named with what is typed, and opens it; Enter in the
  search box does the same. It is greyed out until a name is typed, and while
  the name is taken by anything in the project or by one of Daggerfall's own
  dungeons or cells of that kind (hover it to see why). A new dungeon has no
  cells or location yet; a new cell has no objects.
* **File > Open cell…** (Ctrl+O) lists all 187 RDB blocks, filterable by type:
  `N` interior, `W` interior with water, `M` interior variant, `S` special
  (quest) and `B` border caps.
* **File > Open dungeon…** (Ctrl+Shift+O) lists every pre-built dungeon in
  `MAPS.BSA`, searchable and filterable by region. The dungeon is shown with
  all of its cells in place. Hovering a cell highlights the whole cell and its
  block outline; double-clicking it opens that cell in a separate window with
  the full cell editor (closing the previously opened cell window). The window
  starts with the dungeon selected under **Used in**, so the dungeon's other
  cells show faded around the cell. Changes
  made in that window appear in the dungeon view. As in the game, the exit door
  (model 70300) is only kept in the starting cell, labelled **Start**; opening
  a cell on its own with **Open cell…** still shows all of its doors.
* The dungeon view's properties panel (right) shows the dungeon's name, type,
  location (including its region) and cells. Hovering a cell in its list highlights it, and
  double-clicking opens it. The name and type of a project dungeon can be
  changed there; the type decides which enemies its random enemy markers become.
* Under **Location** is where the dungeon is in the world. For Daggerfall's
  dungeons it shows their map position and location type; **Show map** shows
  them on the world map and **View…** their entrance. A project dungeon is meant to be added to the
  game as a new location, so there you choose:
  * **Map position**: its spot on the world map, in map pixels (x 0–999 from
    west to east, y 0–499 from north to south). **Map…** opens the world map in
    a window of its own: each region in its own colour, the sea in blue, every
    location as a dark dot and every dungeon as a red one. Scroll to zoom, drag
    to pan, and click a spot to choose it: an orange marker shows the choice,
    and the bar along the bottom gives its position and region. Nothing changes
    until **Select Location** (bottom right) is pressed, which places the
    dungeon there and closes the map; closing the map instead keeps the old
    spot. Pointing at a spot shows its region and what is already there. Every project dungeon with a
    spot is marked with a diamond and its name: the one being placed in yellow,
    the others in cyan. The x and y fields nudge it afterwards, and **Remove**
    takes it off the map again. The search field above the map finds a
    location by name, suggesting Daggerfall's locations and your dungeons as you
    type; choosing one, or **Find** (or Enter) for the best match, zooms in on it
    and rings it in white. A spot that already has a location (Daggerfall's or
    another of your dungeons) cannot be taken: clicking it says what is there,
    and the x and y fields will not move the dungeon onto it. The exception is
    one of Daggerfall's dungeons: clicking it chooses to **replace** it (the
    bar says "Chosen: replace …", the marker "(replaces …)"), and **Select
    Location** puts the dungeon in its place. A dungeon can
    only be placed once it has an
    entrance (to get in) and an exit (an exit door in its start cell, to get
    out); until then the map can be looked at but not clicked, and it says
    what is missing.
  * **Replaces**: shown when the dungeon replaces one of Daggerfall's, naming
    it, for example "Privateer's Hold (Daggerfall)". In the game the dungeon
    takes that dungeon's place: its spot, and its name, which quests and the
    travel map know it by (the x and y fields cannot move it). **Stop
    replacing** (or **Remove**) takes it off the map so it can be placed on a
    free spot instead.
  * **Region**: the region of its spot, set by the map position (or that of
    the dungeon it replaces).
  * **Location type**: its icon on the travel map (large, medium or small
    dungeon, or graveyard).
  * **Discovered**: whether the travel map shows it from the start of the game.
  * **Entrance**: what stands above ground around the way down (ruins, a
    castle, a graveyard…), named by its kind and block, for example
    "Ruins · RUINAA08". The button opens a window of its own listing every
    entrance Daggerfall's dungeons have: their 4,232 dungeons share 136
    entrances, each a grid of RMB blocks. The list can be searched by kind,
    block or dungeon name. The selected entrance is drawn in 3D on a patch of
    ground, with its dungeon door highlighted (right-drag to turn, scroll to
    zoom); the dungeons that have it are listed above, and **Use this
    entrance** gives it to the dungeon. An entrance without a dungeon door
    model is flagged, since the player may not be able to go in.
  * **Music**: the song played inside. **Chosen by the game** leaves it to the
    game (for a dungeon that replaces one of Daggerfall's, that dungeon's song,
    shown in the list); otherwise one of Daggerfall Unity's 15 dungeon songs,
    "Dungeon 1 (DUNGEON)" to "Dungeon 15 (28)". Daggerfall Unity picks a
    dungeon's song from a value in its location data and the region, so the
    export writes a value that picks the song chosen. With FM music the game
    plays the FM version of the same song; music mods may play something else.
    For Daggerfall's own dungeons the row shows the song they play. **▶ Play**
    plays the song (read from `MIDI.BSA`) to preview it and turns into **■ Stop**
    while it plays; choosing another song stops it. On Windows and macOS it plays through the system's MIDI
    synthesizer; on Linux it needs TiMidity, or FluidSynth with a General MIDI
    sound font, installed.
  * **Custom music**: ticking **Play my own music file** gives the dungeon a
    music file of your own instead (OGG or MP3), chosen in the **Music file**
    box below it (**Browse…** opens a file dialog). The game songs above are
    greyed out then. Only a `.dfmod` can play it: the export copies the file
    into the mod's `Music` folder and adds a C# script
    (`<mod>DungeonMusic.cs.txt`), which Daggerfall Unity compiles when it loads
    the mod. Inside the dungeon (recognised by its region and name) the script
    stops the game choosing songs, fades the song playing out over half a second
    and stops it for good, then plays the file on a loop at the game's music
    volume; leaving the dungeon gives the game's music back. Dynamic Music, which
    plays music with a player of its own, is faded out and paused the same way
    (other music mods that do so may still play over it). With the exported
    files used loose there is no script, so the dungeon plays the song chosen
    under **Music** instead. A file that is missing or not OGG/MP3 is listed as a
    problem and stops the export.

  Warnings below list what is still missing: a spot, a spot that is not in the
  sea or already taken (by a Daggerfall location or another project dungeon,
  since two locations cannot share a map pixel), an entrance and an exit. A copy of a
  Daggerfall dungeon starts with the original's location type and entrance but
  no spot, since the original's is taken; a duplicated dungeon loses its spot
  for the same reason.
* Right-clicking a cell in the dungeon view or in the Cells list offers
  **Make Start**, which makes it the dungeon's start cell (where the player
  enters) and stops the previous one from being it. Only the start cell keeps
  its exit doors, so Daggerfall cells gain or lose them to match. A cell
  without an exit door (model 70300), from which the player could not leave,
  or without a start marker, where the player arrives, cannot be the start:
  the option is disabled and the menu says "No exit exists for this cell"
  and/or "No start marker exists for this cell". In a dungeon from Daggerfall
  it offers to make a copy first.
* Dragging a cell with the left mouse button moves it around the dungeon: it
  follows the pointer from grid square to grid square, highlighted, with "Move …
  to x, z" beside it. Let go on a free square and it stays there; on a square
  another cell already has ("A cell is already here") it goes back. In a dungeon
  from Daggerfall it goes back and offers to make a copy first.
* Pressing **Delete** while hovering a cell (in the dungeon view, or its row in
  the Cells list), or choosing **Delete** in its right-click menu (in either
  place), takes it out of the dungeon. A project cell it used stays in
  the project; a cell window open on it closes. Taking out the start cell leaves
  the dungeon without one until another is made the start. In a dungeon from
  Daggerfall it offers to make a copy first.
* Dragging a cell from the project panel's **Cells** list (yours, or
  Daggerfall's with the **Daggerfall** switch on) into the dungeon view adds it
  to the dungeon. While dragging, a faded copy of the cell shows on the grid
  square under the pointer, outlined, with "Drop to add … here"; dropping it
  there adds it. A square that already has a cell shows "A cell is already here"
  and takes nothing. A project cell dropped in is used as it is (not copied), so
  changing it changes it in every dungeon that uses it. In a dungeon from
  Daggerfall it offers to make a copy first.
* The outliner (left) lists every object grouped by the block's object lists.
  The properties panel (right) edits name, visibility, position and rotation,
  and under **Game settings** lists every setting Daggerfall stores with the
  object, set or not: for models the trigger, starting lock, sound, action
  (type, axis, duration, magnitude) and the object the action triggers next
  (with **Go to**, which selects and frames it); for flats their flags, action, magnitude, sound and link;
  for lights two values of unknown meaning. All of them can be changed in
  project cells. Action links are kept by object id, so they survive edits and
  are saved in project files. In the viewport each link is a red line, drawn
  over walls, from the object to the one it triggers, with an arrowhead at the
  target (**View > Action links** turns them off). Links stay within one cell.
* **Preview action**, under Game settings, appears for objects whose action
  moves something: it plays the action in the viewport as Daggerfall Unity
  does, with every object down the chain moving at the same time (moves,
  slides, rotations, and doors that an action opens swinging open), holds the
  end for half a second and puts everything back. The cell is not changed.
* Several objects can be selected with Shift+click (in the viewport or the
  outliner; Shift+click a selected object to take it out) or by dragging a box
  over empty space in the viewport: the objects inside it light up, and
  releasing selects them (Shift adds them to the selection). Several selected
  objects can only be moved: the gizmo sits at their middle and moves them all
  together, as one undo step. With grid lock on, the last selected object
  lands on the grid and the others keep their places relative to it.
* A single selected object's gizmo also has three **rotation rings** (red,
  green and blue, around X, Y and Z). Dragging a ring turns the object around
  that axis by the angle the pointer sweeps round the gizmo, in 15° steps with
  grid lock on (Ctrl switches it for that drag); the status bar shows the angle.
* A selected object whose action moves it shows its **trajectory**: a purple
  line along the path its middle takes, from a ring where it is to a dot where
  the action leaves it (a straight line for moves and slides, an arc for
  turns). Every object its action sets moving shows its trajectory too, down
  the chain to its end. It is worked out every frame, so it follows any change to the
  action's type, axis or magnitude, or to the object's position.
* Right-clicking an object (in the viewport or the outliner) opens its menu.
  **Tie action to…** draws a red line from the object to the pointer; the
  object under the pointer is highlighted, and clicking it makes the action
  trigger it (Escape, right-click or clicking empty space cancels). A model
  without an action gets an empty one, to be given a type under Game
  settings. On an object that other objects' actions trigger, **Remove
  actions** removes those actions, as one undo step.
* With nothing selected, the properties panel shows the **cell**: name,
  category, source, size, object and object list counts, and its water level
  and castle flag (which live on the cell's start marker). A cell with water
  shows its water surface as a see-through blue plane across its square, in
  the cell editor and the dungeon view.
* Every cell covers a 51.2 × 51.2 m square of the dungeon grid. Objects that
  reach outside it (by their model's box, or their position) can overlap the
  next cell in a dungeon, so they are marked ⚠ in the outliner, warned about
  in their properties and in the status bar after moving them, and counted in
  the cell's properties, with **Select them** to select them all. (Daggerfall's
  own cells have 210 such objects in 68 cells, mostly by under half a metre.)
* Selecting a marker adds a **Marker** section naming its kind and what the game
  reads from it: water level and castle flag on start markers, slot and spawn
  distance on random enemy markers, spawn distance (as well as the enemy) on
  fixed enemy markers. Enter, quest, quest item, treasure and teleport markers
  carry no data; the game uses only their position. A teleport marker is where
  the player lands when an object whose action is Teleport, tied to the marker,
  is triggered.
* Resting the pointer on any property or button for half a second explains
  what it does.
* **Used in**, at the bottom of the properties panel, lists every dungeon that
  uses the cell (for a cell copy, the project dungeons using that copy).
  Clicking one shows that dungeon's other cells, faded and outlined, around
  the cell being edited (click it again to hide them); double-clicking opens it
  in the dungeon view. Hovering a faded cell outlines it and names it;
  double-clicking it opens that cell instead, with the same dungeon still
  selected and shown around it (in a cell window of the open dungeon, the cell
  opens in a cell window as if double-clicked in the dungeon view).
* **View** toggles back-face hiding (to look into cells from outside), the grid,
  the block outline, the asset palette, models, flats, editor markers, lights
  and action links.

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
project adds a copy where it is dropped. Cells can also be dragged into an open dungeon (see
above).

**File > Save project** (Ctrl+S) and **Save project as…** (Ctrl+Shift+S) write
a `.ddm` project file; **Open project…** and **New project** switch projects,
asking first if there are unsaved changes, as closing the program does. The last
project is reopened on the next run, and a `.ddm` file can be passed on the
command line. Project files are JSON. A dungeon refers to the project cells it
uses by id; its other cells are read from the game data when it is opened. A
dungeon's location (map position, location type, discovered and entrance) is
saved with it.

### Exporting to Daggerfall Unity

**File > Export to Daggerfall Unity…** turns the project into a
[Daggerfall Unity](https://www.dfworkshop.net/) mod that adds the project
dungeons you choose to the game as new locations. The window has:

* **Mod**: the mod's title, version, author, contact and description, shown in
  Daggerfall Unity's mod list. They are saved with the project.
* **Dungeons**: every project dungeon with a tick box; the ticked ones go into
  the mod. Only dungeons placed on the world map can be ticked (placed ones start
  ticked). A ticked dungeon also needs an entrance, a dungeon type, one start cell
  with an exit door, a free spot in a region, and a name no other location of its
  region has. Anything missing is listed under it, and the export waits until it
  is fixed or the dungeon is unticked. **Check again** re-checks after changes.
* **Output**: the folder to export into. A folder named after the mod is made
  there, holding:
  * `WorldData/location-<region>-<index>.json` for each dungeon that replaces
    one of Daggerfall's: the file Daggerfall Unity reads instead of that
    location. It keeps the location's index, map id, location ids, position,
    climate and name, so quests and saved games still find it, with the
    project dungeon's cells, entrance, type and settings;
  * `WorldData/locationnew-<dungeon>-<region>.json` for each other dungeon: its
    location, entrance (the above-ground blocks of the Daggerfall dungeon chosen
    as entrance), climate and grid of cells;
  * `WorldData/<BLOCK>.RDB.json` for each project cell the dungeons use, under a
    new block name: the letter of the cell it was copied from and a number from
    9000000 up, for example `N9000004.RDB`. Daggerfall's own cells are used by
    their own names and are not exported;
  * `<mod>.dfmod.json`, the mod's manifest, and a `README.txt` listing what is in it.

  The `WorldData` files work as they are when copied into Daggerfall Unity's
  `DaggerfallUnity_Data/StreamingAssets/WorldData` folder, without a mod.
* **Files only** or **Build a .dfmod with Unity**: whether to also pack the
  files into a `.dfmod` (a Unity asset bundle). With **Files only**, if a ticked
  dungeon has custom music, the window warns that custom music is not supported
  in loose files and those dungeons fall back to the game's own music. Tick the platforms to build for;
  each gets its own `.dfmod` in a `Windows`, `Linux` or `macOS` folder next to the
  files, and **Copy to Mods folder** also puts this computer's build into
  Daggerfall Unity's `Mods` folder.
* **With Unity**, the `.dfmod` is built by running the Unity editor in the
  background, as Daggerfall Unity's Mod Builder does. It needs Unity 2019.4 (Daggerfall Unity uses 2019.4.40f1),
  installed with Unity Hub and signed in to a licence. **Find** looks for
  editors installed by Unity Hub. Each platform needs its build support installed
  in the editor. The first build makes a small Unity project to build in (in the
  local application data folder) and takes a few minutes; later builds take about
  a minute.

The output folder, Unity editor and platforms are remembered between runs.
Block names, location ids and the mod's GUID stay the same from one export to
the next, so saved games keep working with an updated mod.

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
| ★ Favorites | The assets you marked as favorites, of every kind, in the order marked |
| Cell models | Every model used in Daggerfall's dungeon cells, most widely used first |
| All models | Every model in `ARCH3D.BSA` |
| Flats | Sprites used in cells: torches, furniture, people, treasure |
| Markers | Editor markers: start/enter points, enemy spawns, quest and treasure markers |
| Lights | A point light |

The list is built from your own game data when the ARENA2 folder is loaded.
Any tile can be marked as a **favorite**: hover it and click the ☆ in its
top-right corner (or right-click it and choose **Add to favorites**). Favorites
keep a gold ★ on every tab and are listed together on the **★ Favorites** tab;
clicking the ★ again (or **Remove from favorites**) unmarks one. Favorites are
saved with your settings, like your names for cells and models.
Click a tile to arm it; a green ghost follows the mouse over the cell surfaces.
Left click places it (Shift+click to keep placing), Q and E (or R) turn it
90° anticlockwise and clockwise, Esc cancels.

A model being placed **snaps** to the models already in the cell where their
open faces meet, and the face it snaps to is outlined in green. Open faces are
worked out from each model: the parts of its upright sides that its own
polygons leave uncovered, such as a corridor's ends or a doorway in a room's
wall. Two openings facing each other join flush, with their floors level; a
narrower opening can slide along a wider one. Thin models such as doors fit
into an opening, standing on its floor, and into the doorways inside pieces
where Daggerfall's own cells put doors (learned from every cell when the game
data loads), turned the way those doors are. The closest match in size wins,
then the nearest. A face something is already joined to (another piece, or a
door standing in it) is taken and not snapped to again. Hold Alt to place
without snapping. **Grid lock** on the menu bar (or G) snaps objects to the grid while they are
moved or placed: the position along the dragged axis lands on a grid line,
which is drawn faintly at the chosen step (0.4, 0.8, 1.6 or 3.2 m, that is 16,
32, 64 or 128 game units). Placed objects appear under **Added objects** in the outliner and can
be moved, rotated, duplicated, deleted and undone like any other object.

An opening of a hall or room piece that joins nothing and leads out into
empty space gets a **yellow pane** over it, in the cell editor and in the
dungeon view, and the dungeon's properties warn **Not all rooms are closed**,
naming the cells. It is only a warning: saving and exporting work as before.
An opening counts as closed when another piece's opening is joined to it, or
when most looks out through it (from its middle and four points around it) meet
a wall of the piece's own just inside it or any other model within 60 m. A door
standing in an opening does not close it, since at the dungeon's edge it would
open onto nothing. Only pieces at least 3 m tall and wide are checked, and only
openings a person could walk through. Some pieces have openings that look open by
this test but that Daggerfall's own dungeons leave so too (the space under a
ramp, the sides of cave chunks); these are learned from every Daggerfall dungeon
the first time the game data is loaded (it takes about half a minute in the
background, during which the properties say "Checking that every room is
closed…") and saved with the settings, and are not warned about. A cell edited
on its own does not warn about openings on its edges, where it meets its
neighbours; opened from a dungeon, the cells around it are taken into account.

| Input | Action |
| --- | --- |
| Click an asset tile | Arm it for placement |
| Left click while placing | Place it (Shift to keep placing) |
| Q / E or R while placing | Turn 90° anticlockwise / clockwise |
| Alt while placing | Place without snapping to open faces |
| Esc while placing | Cancel |
| Left click | Select object |
| Shift + left click | Add to or take out of the selection |
| Left drag on empty space | Select the objects inside a box (Shift adds them) |
| Left drag on gizmo arrow | Move along that axis (Ctrl switches grid lock for that drag) |
| Left drag on gizmo ring | Turn the selected object around that axis (15° steps with grid lock; Ctrl switches) |
| G | Turn grid lock on or off |
| Right drag | Orbit |
| Middle drag / Shift + right drag | Pan |
| Mouse wheel | Zoom |
| W A S D | Move forward / left / back / right along the ground, with the pointer over the view (Shift for faster) |
| Space / Ctrl | Raise / lower the camera, with the pointer over the view (right mouse + E / Q too). Ctrl lowers only when held on its own |
| F / Home | Frame selection / frame cell |
| E or R / Q or Shift + R | Turn the selected object 90° clockwise / anticlockwise (not a group) |
| Ctrl + D / Delete | Duplicate / delete selection |
| Ctrl + Z / Ctrl + Y | Undo / redo |

Edits are kept in memory only; saving changes back to a file is not
implemented yet.

## Code layout

| Path | Purpose |
| --- | --- |
| `src/arena2/` | Readers for BSA archives, ARCH3D models, RDB and RMB blocks, textures, palettes and the world map |
| `src/scene.rs` | Editable object list built from an RDB block |
| `src/project.rs` | Project files (`.ddm`): the user's cells, dungeons and folders |
| `src/encounters.rs` | Dungeon types and the enemies random enemy markers can become |
| `src/music.rs`, `src/arena2/hmi.rs` | Dungeon music: how the game picks a song, reading Daggerfall's HMI songs, and playing them |
| `src/export/` | Exporting the project as a Daggerfall Unity mod: world data JSON, manifest, music script, and the .dfmod built with Unity |
| `src/catalog.rs` | Placeable assets, gathered from the game data at run time |
| `src/connectors.rs` | Open faces and doorways of models, for snapping pieces together |
| `src/camera.rs`, `src/picking.rs` | Orbit camera and ray picking |
| `src/render/` | wgpu renderer (models, billboards, helper lines) drawing into an egui texture |
| `src/app/` | egui application: main window, menus, dialogs and cell editor windows |
| `src/app/project_panel.rs` | The project panel: dungeons or cells, Daggerfall's and the project's |
| `src/app/room_editor/` | The cell editor: outliner, properties, asset palette, viewport, undo history |
| `src/app/dungeon_view.rs` | Whole-dungeon view with cell highlighting, and the dungeon's properties |
| `src/app/world_map.rs` | The world map window for choosing a dungeon's spot |
| `src/app/entrance_picker.rs` | The entrance window: every dungeon entrance, with a 3D preview |
| `src/app/export_window.rs` | The Export to Daggerfall Unity window |
| `tests/real_data.rs` | Tests against a real install; set `DAGGERFALL_ARENA2` to run them |

The file formats follow [Daggerfall Unity](https://github.com/Interkarma/daggerfall-unity)
(MIT), which was used as the reference implementation.
