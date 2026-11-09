//! Tests that parse a real Daggerfall installation.
//!
//! No game data is stored in this repository. Set the `DAGGERFALL_ARENA2`
//! environment variable to an `ARENA2` folder to run these tests; without it
//! every test returns immediately.

use std::path::PathBuf;

use daggerfall_dungeon_maker::arena2::{Arena2Library, RdbObjectKind};
use daggerfall_dungeon_maker::catalog::{AssetCatalog, ModelCategory};

/// Opens the library pointed to by `DAGGERFALL_ARENA2`, if set.
fn open_library() -> Option<Arena2Library> {
    let folder = PathBuf::from(std::env::var_os("DAGGERFALL_ARENA2")?);
    Some(Arena2Library::open(&folder).expect("ARENA2 folder should load"))
}

#[test]
fn lists_rdb_blocks() {
    let Some(library) = open_library() else { return };

    let block_names = library.rdb_block_names();
    println!("{} RDB blocks, first {:?}", block_names.len(), block_names.first());
    assert!(block_names.len() > 100);
    assert!(block_names.iter().all(|name| name.ends_with(".RDB")));
}

#[test]
fn lists_dungeons_whose_blocks_all_exist() {
    let Some(mut library) = open_library() else { return };

    let dungeons = library.dungeons();
    println!("{} dungeons", dungeons.len());
    assert!(dungeons.len() > 1000);
    assert!(dungeons.iter().any(|dungeon| dungeon.name == "Privateer's Hold"));

    let block_names = library.rdb_block_names();
    for dungeon in dungeons.iter() {
        for placement in &dungeon.blocks {
            assert!(
                block_names.contains(&placement.block_name),
                "{} uses missing {}",
                dungeon.name,
                placement.block_name
            );
        }
    }
}

#[test]
fn dungeons_have_types() {
    let Some(mut library) = open_library() else { return };

    let dungeons = library.dungeons();
    let typed = dungeons.iter().filter(|dungeon| dungeon.dungeon_type.is_some()).count();
    println!("{typed} of {} dungeons have a type", dungeons.len());
    assert!(typed * 100 >= dungeons.len() * 99, "most dungeons should have a type");
    for dungeon_type in daggerfall_dungeon_maker::encounters::DungeonType::all() {
        let examples: Vec<&str> = dungeons
            .iter()
            .filter(|dungeon| dungeon.dungeon_type == Some(dungeon_type))
            .map(|dungeon| dungeon.name.as_str())
            .take(3)
            .collect();
        println!("{}: {}", dungeon_type.name(), examples.join(", "));
        assert!(!examples.is_empty(), "no dungeon is a {}", dungeon_type.name());
    }
}

#[test]
fn fixed_enemy_markers_name_known_enemies() {
    let Some(library) = open_library() else { return };

    let mut markers = Vec::new();
    for name in library.rdb_block_names() {
        let block = library.rdb_block(&name).expect("block should parse");
        let scene = daggerfall_dungeon_maker::scene::Scene::from_rdb_block(&block);
        markers.extend(
            scene.objects.iter().filter_map(|object| object.fixed_enemy()).map(|enemy| (name.clone(), enemy)),
        );
    }
    let unknown: Vec<_> = markers
        .iter()
        .filter(|(_, enemy)| enemy.enemy().is_none() && !enemy.is_switched_off())
        .map(|(name, enemy)| format!("{name}: {}", enemy.mobile_id))
        .collect();
    println!("{} fixed enemy markers, {} name an unknown enemy", markers.len(), unknown.len());
    for (name, enemy) in markers.iter().take(8) {
        println!(
            "  {name}: {} (passive: {}, gender: {:?})",
            enemy.description(),
            enemy.passive,
            enemy.human_gender()
        );
    }
    assert!(!markers.is_empty());
    assert!(unknown.is_empty(), "unknown enemies: {unknown:?}");
}

#[test]
fn water_cells_have_underwater_enemy_markers() {
    let Some(library) = open_library() else { return };

    let mut cells_with_water = Vec::new();
    let mut underwater_markers = 0;
    let mut random_markers = 0;
    for name in library.rdb_block_names() {
        let block = library.rdb_block(&name).expect("block should parse");
        let scene = daggerfall_dungeon_maker::scene::Scene::from_rdb_block(&block);
        let markers: Vec<_> = scene.objects.iter().filter(|object| object.is_random_enemy_marker()).collect();
        random_markers += markers.len();
        if let Some(level) = scene.water_level() {
            let under = markers.iter().filter(|marker| scene.is_under_water(marker)).count();
            underwater_markers += under;
            cells_with_water.push(format!(
                "{name} (water at {level:.1} m, {under} of {} random markers under)",
                markers.len()
            ));
        }
    }
    println!(
        "{} cells have water; {underwater_markers} of {random_markers} random enemy markers are under it",
        cells_with_water.len()
    );
    for cell in cells_with_water.iter().take(6) {
        println!("  {cell}");
    }
    assert!(!cells_with_water.is_empty());
    assert!(underwater_markers > 0);
    // Water cells are the "W" interiors.
    assert!(cells_with_water.iter().any(|cell| cell.starts_with('W')));
}

#[test]
fn lever_action_links_to_a_door() {
    use daggerfall_dungeon_maker::scene::{Scene, SceneObjectKind};
    let Some(mut library) = open_library() else { return };

    let scene = Scene::from_rdb_block(&library.rdb_block("N0000051.RDB").expect("block should parse"));
    let lever = scene
        .objects
        .iter()
        .find(|object| matches!(object.kind, SceneObjectKind::Model { model_id: 61028, .. }))
        .expect("N0000051 has a lever");
    let SceneObjectKind::Model { data, .. } = lever.kind else { unreachable!() };
    let action = data.action.expect("the lever has an action");
    println!("lever action: {action:?}, trigger {}, lock index {}", data.trigger(), data.lock_index());
    let target =
        scene.object_by_id(lever.next.expect("the lever links to an object")).expect("the target exists");
    let SceneObjectKind::Model { model_id, .. } = target.kind else {
        panic!("the lever should link to a model")
    };
    println!("lever opens model {model_id}");
    let door = library.model(model_id).expect("target model loads");
    assert!(door.door_kind().is_some(), "the lever should link to a door");
}

#[test]
fn parses_every_rdb_block_and_its_models() {
    let Some(mut library) = open_library() else { return };

    let mut object_count = 0;
    let mut missing_models = 0;
    for name in library.rdb_block_names() {
        let block = library.rdb_block(&name).unwrap_or_else(|error| panic!("{name}: {error}"));
        object_count += block.objects.len();
        for object in &block.objects {
            if let RdbObjectKind::Model(model) = &object.kind {
                if library.model(model.model_id).is_none() {
                    missing_models += 1;
                }
            }
        }
    }
    println!("{object_count} objects, {missing_models} missing model references");
    assert!(object_count > 10_000);
    assert_eq!(missing_models, 0);
}

#[test]
fn describes_every_model() {
    let Some(mut library) = open_library() else { return };

    let started = std::time::Instant::now();
    let catalog = AssetCatalog::build(&mut library);
    println!("catalog built in {:?}", started.elapsed());

    assert_eq!(library.texture_archive_name(74), Some("Door Set"));
    let exit_door = catalog.model_description(70300).expect("the exit door model is described");
    println!("70300: {}", exit_door.summary());
    assert_eq!(exit_door.category, ModelCategory::ExitDoor);

    let described = catalog.all_model_ids.iter().filter(|&&id| catalog.model_description(id).is_some());
    assert!(described.count() > catalog.all_model_ids.len() * 9 / 10);
    let mut category_counts = std::collections::BTreeMap::new();
    for (position, model) in catalog.room_models.iter().enumerate() {
        let description = catalog.model_description(model.model_id).expect("room models are described");
        *category_counts.entry(description.category.label()).or_insert(0) += 1;
        if position % 12 == 0 {
            println!("{}: {}", model.model_id, description.summary());
        }
    }
    println!("room model categories: {category_counts:?}");
}

#[test]
fn decodes_room_textures() {
    let Some(mut library) = open_library() else { return };

    let block = library.rdb_block("N0000000.RDB").expect("block exists");
    let mut decoded = 0;
    for object in &block.objects {
        if let RdbObjectKind::Model(model) = &object.kind {
            let mesh = library.model(model.model_id).expect("model exists");
            for submesh in &mesh.submeshes {
                let image = library
                    .texture_image(submesh.texture)
                    .unwrap_or_else(|| panic!("{} should decode", submesh.texture));
                assert_eq!(image.pixels.len(), (image.width * image.height * 4) as usize);
                decoded += 1;
            }
        }
    }
    println!("decoded {decoded} textures");
    assert!(decoded > 0);
}

#[test]
fn dungeons_sit_on_the_world_map_in_their_region() {
    let Some(mut library) = open_library() else { return };

    let world = library.world_map().expect("MAPS.BSA should give a world map");
    assert!(world.has_regions(), "POLITIC.PAK should be read");
    println!("{} locations", world.locations.len());
    assert!(world.locations.len() > 10_000);

    let dungeons = library.dungeons();
    let mut in_region = 0;
    let mut location_types = std::collections::BTreeMap::new();
    for dungeon in dungeons.iter() {
        let pixel = dungeon.map_pixel.unwrap_or_else(|| panic!("{} has no map position", dungeon.name));
        assert!(pixel.is_on_map(), "{} is off the map at {pixel}", dungeon.name);
        assert!(
            world.locations_at(pixel).any(|location| location.name == dungeon.name),
            "{} is not listed at {pixel}",
            dungeon.name
        );
        if world.region_at(pixel) == Some(dungeon.region_index) {
            in_region += 1;
        }
        let type_name = dungeon.location_type.map_or("unknown", |location_type| location_type.name());
        *location_types.entry(type_name).or_insert(0) += 1;
    }
    println!("{in_region} of {} dungeons are on a pixel of their region", dungeons.len());
    println!("location types of dungeons: {location_types:?}");
    assert!(in_region * 100 >= dungeons.len() * 95);
}

#[test]
fn dungeon_entrances_load_and_have_an_entrance_door() {
    use daggerfall_dungeon_maker::arena2::DoorKind;
    use std::collections::BTreeMap;
    let Some(mut library) = open_library() else { return };

    let dungeons = library.dungeons();
    let mut designs: BTreeMap<_, Vec<&str>> = BTreeMap::new();
    for dungeon in dungeons.iter() {
        let exterior =
            dungeon.exterior.as_ref().unwrap_or_else(|| panic!("{} has no exterior", dungeon.name));
        designs
            .entry((exterior.width, exterior.height, exterior.block_names.clone()))
            .or_default()
            .push(&dungeon.name);
    }
    println!("{} dungeons have {} different entrances", dungeons.len(), designs.len());
    let mut without_door = Vec::new();
    for ((width, height, block_names), users) in &designs {
        let mut has_entrance = false;
        for name in block_names {
            let block = library.rmb_block(name).unwrap_or_else(|error| panic!("{name}: {error}"));
            for model in &block.models {
                let mesh = library.model(model.model_id);
                has_entrance |= mesh.and_then(|mesh| mesh.door_kind()) == Some(DoorKind::DungeonEntrance);
            }
        }
        if !has_entrance {
            without_door.push(format!("{width}x{height} {block_names:?} ({})", users[0]));
        }
    }
    println!(
        "{} entrances without an entrance door: {:?}",
        without_door.len(),
        without_door.iter().take(10).collect::<Vec<_>>()
    );
    assert!(without_door.len() * 20 <= designs.len());
}

#[test]
fn doors_and_pieces_join_in_daggerfalls_own_cells() {
    use daggerfall_dungeon_maker::connectors::{fit, open_faces, DoorSlots};
    use daggerfall_dungeon_maker::scene::{Scene, SceneObjectKind};
    let Some(mut library) = open_library() else { return };

    let mut door_slots = DoorSlots::default();
    let (mut joined, mut pieces) = (0, 0);
    for name in library.rdb_block_names() {
        let scene = Scene::from_rdb_block(&library.rdb_block(&name).unwrap());
        door_slots.learn(&scene, &mut library);
        let mut faces = Vec::new();
        for (index, object) in scene.objects.iter().enumerate() {
            let SceneObjectKind::Model { model_id, .. } = object.kind else { continue };
            let Some(mesh) = library.model(model_id) else { continue };
            for face in open_faces(&mesh).iter().filter(|face| !face.is_plug) {
                faces.push((index, face.transformed(object.rotation(), object.position)));
            }
        }
        let mut with_faces = std::collections::HashSet::new();
        let mut matched = std::collections::HashSet::new();
        for (first, face) in &faces {
            with_faces.insert(*first);
            // A face that meets another piece's face of the same size, centre to centre.
            let meets = faces.iter().any(|(second, other)| {
                first != second
                    && face.center.distance(other.center) < 0.1
                    && fit(face, other).is_some_and(|m| m < 0.1)
            });
            if meets {
                matched.insert(*first);
            }
        }
        pieces += with_faces.len();
        joined += matched.len();
    }
    println!(
        "{joined} of {pieces} pieces with openings meet a neighbour exactly; {} pieces have doorways",
        door_slots.piece_count()
    );
    assert!(door_slots.piece_count() > 200);
    assert!(joined * 4 > pieces, "at least a quarter of the pieces should meet a neighbour face to face");
}

#[test]
fn exported_blocks_match_daggerfalls_own() {
    use std::collections::HashMap;

    use daggerfall_dungeon_maker::export::json::Block;
    use daggerfall_dungeon_maker::scene::Scene;
    let Some(mut library) = open_library() else { return };

    let tags = library.model_tags();
    assert_eq!(tags.get(&55004).map(String::as_str), Some("DOR"), "a common door is tagged as one");
    for name in library.rdb_block_names() {
        let block = library.rdb_block(&name).expect("block should parse");
        let scene = Scene::from_rdb_block(&block);
        let value = serde_json::to_value(Block::from_cell(&name, 0, &scene.objects, &tags)).unwrap();
        let models = value["RdbBlock"]["ModelReferenceList"].as_array().unwrap();
        let exported: HashMap<i64, &serde_json::Value> = value["RdbBlock"]["ObjectRootList"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|root| root["RdbObjects"].as_array())
            .flatten()
            .map(|object| (object["Position"].as_i64().unwrap(), object))
            .collect();
        assert_eq!(exported.len(), block.objects.len(), "{name}: every object is exported");
        // Exported objects are numbered 16, 32, … in block order.
        let position_of = |offset: i32| {
            block
                .objects
                .iter()
                .position(|object| object.offset == offset)
                .map_or(-1, |index| (index as i64 + 1) * 16)
        };
        for (index, original) in block.objects.iter().enumerate() {
            let object = exported[&((index as i64 + 1) * 16)];
            let place =
                [&object["XPos"], &object["YPos"], &object["ZPos"]].map(|value| value.as_i64().unwrap());
            let original_place =
                [original.position.x, original.position.y, original.position.z].map(i64::from);
            assert_eq!(place, original_place, "{name}");
            let resources = &object["Resources"];
            match &original.kind {
                RdbObjectKind::Model(model) => {
                    let resource = &resources["ModelResource"];
                    let reference = &models[resource["ModelIndex"].as_u64().unwrap() as usize];
                    assert_eq!(reference["ModelIdNum"], model.model_id, "{name}");
                    let rotation =
                        ["XRotation", "YRotation", "ZRotation"].map(|key| resource[key].as_i64().unwrap());
                    let original_rotation =
                        [model.rotation.x, model.rotation.y, model.rotation.z].map(i64::from);
                    assert_eq!(rotation, original_rotation, "{name}");
                    assert_eq!(resource["TriggerFlag_StartingLock"], model.trigger_and_lock, "{name}");
                    assert_eq!(resource["SoundIndex"], model.sound, "{name}");
                    let action = &resource["ActionResource"];
                    match model.action {
                        Some(original_action) => {
                            assert_eq!(action["Flags"], original_action.action_type, "{name}");
                            assert_eq!(action["Axis"], original_action.axis, "{name}");
                            assert_eq!(action["Duration"], original_action.duration, "{name}");
                            assert_eq!(action["Magnitude"], original_action.magnitude, "{name}");
                            let next = position_of(original_action.next_offset);
                            assert_eq!(action["NextObjectOffset"], next, "{name}");
                        }
                        None => assert_eq!(action["Flags"], 0, "{name}"),
                    }
                }
                RdbObjectKind::Flat(flat) => {
                    let resource = &resources["FlatResource"];
                    assert_eq!(resource["TextureArchive"], flat.texture.archive, "{name}");
                    assert_eq!(resource["TextureRecord"], flat.texture.record, "{name}");
                    assert_eq!(resource["Flags"], flat.flags, "{name}");
                    assert_eq!(resource["FactionOrMobileId"], flat.faction_or_mobile_id, "{name}");
                    assert_eq!(resource["Action"], flat.action, "{name}");
                    assert_eq!(resource["NextObjectOffset"], position_of(flat.next_offset), "{name}");
                }
                RdbObjectKind::Light(light) => {
                    let resource = &resources["LightResource"];
                    assert_eq!(resource["Radius"], light.radius, "{name}");
                    assert_eq!(resource["Unknown1"], light.unknown_1, "{name}");
                    assert_eq!(resource["Unknown2"], light.unknown_2, "{name}");
                }
            }
        }
    }
}

#[test]
fn exports_a_project_dungeon_as_a_new_location() {
    use daggerfall_dungeon_maker::arena2::MapPixel;
    use daggerfall_dungeon_maker::export::{build_mod, check_project};
    use daggerfall_dungeon_maker::project::{
        DungeonLocation, ModDetails, Project, ProjectDungeonRoom, ProjectEntry, ProjectRoom,
    };
    use daggerfall_dungeon_maker::scene::Scene;
    let Some(mut library) = open_library() else { return };

    let dungeons = library.dungeons();
    let original =
        dungeons.iter().find(|dungeon| dungeon.name == "Privateer's Hold").expect("Privateer's Hold");
    let world = library.world_map().unwrap();
    let region = original.region_index;
    // A free spot of the same region near the original.
    let home = original.map_pixel.unwrap();
    let pixel = (-20..=20)
        .flat_map(|dx| (-20..=20).map(move |dy| MapPixel::new(home.x + dx, home.y + dy)))
        .find(|&pixel| world.region_at(pixel) == Some(region) && world.locations_at(pixel).next().is_none())
        .expect("a free spot nearby");

    let mut project = Project::default();
    let room_id = project.new_id();
    let start = original.blocks.iter().find(|block| block.is_starting_block).unwrap();
    let scene = Scene::from_rdb_block(&library.rdb_block(&start.block_name).unwrap());
    project.add(
        ProjectEntry::Room(ProjectRoom {
            id: room_id,
            name: "My start".to_string(),
            based_on: Some(start.block_name.clone()),
            objects: scene.objects,
        }),
        None,
    );
    let dungeon_id = project.add_empty_dungeon("Test Hold of Export");
    let dungeon = project.dungeon_mut(dungeon_id).unwrap();
    dungeon.dungeon_type = original.dungeon_type;
    dungeon.rooms = ProjectDungeonRoom::layout_of(original);
    for room in &mut dungeon.rooms {
        if room.is_starting_block {
            room.room = Some(room_id);
        }
    }
    dungeon.location = DungeonLocation { map_pixel: Some(pixel), ..DungeonLocation::copied_from(original) };

    let checks = check_project(&project, &mut library);
    assert!(checks[0].is_ready(), "{:?}", checks[0].problems);
    let details = ModDetails { title: "Export Test".to_string(), ..ModDetails::default() };
    let mod_files =
        build_mod(&project, &mut library, &details, "Fallback", &[dungeon_id]).expect("the mod builds");
    let paths: Vec<&str> = mod_files.files.iter().map(|file| file.path.as_str()).collect();
    println!("{paths:#?}");
    assert_eq!(mod_files.file_name, "Export_Test");
    assert_eq!(mod_files.cells.len(), 1);
    let block_name = &mod_files.cells[0].block_name;
    assert!(block_name.starts_with(&start.block_name[..1]) && block_name.ends_with(".RDB"), "{block_name}");
    assert!(paths.contains(&format!("WorldData/{block_name}.json").as_str()));
    let location_file = format!("WorldData/locationnew-Test_Hold_of_Export-{region}.json");
    let location = mod_files.files.iter().find(|file| file.path == location_file).expect("the location file");
    let location: serde_json::Value = serde_json::from_str(&location.contents).unwrap();
    let blocks = location["Dungeon"]["Blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), original.blocks.len());
    let start_block = blocks.iter().find(|block| block["IsStartingBlock"] == true).unwrap();
    assert_eq!(start_block["BlockName"], block_name.as_str());
    let exterior_blocks = location["Exterior"]["ExteriorData"]["BlockNames"].as_array().unwrap();
    assert_eq!(exterior_blocks.len(), original.exterior.as_ref().unwrap().block_names.len());
    assert_eq!(location["RegionIndex"], region);
    println!("climate {}", location["Climate"]);
    assert!((223..=232).contains(&location["Climate"]["WorldClimate"].as_i64().unwrap()));
    let manifest = mod_files.files.iter().find(|file| file.path == "Export_Test.dfmod.json").unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest.contents).unwrap();
    assert_eq!(manifest["Files"].as_array().unwrap().len(), mod_files.files.len());
    assert!(manifest["Files"][0].as_str().unwrap().starts_with("Assets/DungeonMaker/Export_Test/"));

    // Written to disk: the world data, the manifest and a readme.
    let folder = std::env::temp_dir().join(format!("ddm-export-{}", std::process::id()));
    let mod_folder =
        daggerfall_dungeon_maker::export::write_mod(&mod_files, &folder).expect("the mod is written");
    let written = |path: &str| mod_folder.join(path).is_file();
    let all_written = mod_files.files.iter().all(|file| written(&file.path)) && written("README.txt");
    std::fs::remove_dir_all(&folder).unwrap();
    assert!(all_written);

    // Its name may not repeat a location of its region.
    project.dungeon_mut(dungeon_id).unwrap().name = original.name.clone();
    let checks = check_project(&project, &mut library);
    let problems = &checks[0].problems;
    assert!(problems.iter().any(|problem| problem.contains("already has a location")), "{problems:?}");
}

#[test]
fn reads_what_identifies_a_location_as_daggerfall_unity_does() {
    let Some(mut library) = open_library() else { return };

    // Daggerfall Unity's own reading of this location, saved by a published mod
    // (drcarademono/beautiful-villages-locations, location-0-2.json).
    let dungeons = library.dungeons();
    let ruins = dungeons
        .iter()
        .find(|dungeon| dungeon.region_index == 0 && dungeon.name == "Ruins of The Caorchom Farmstead")
        .expect("the ruins are in the Alik'r Desert");
    let classic = ruins.classic.expect("its game record is read");
    assert_eq!(classic.location_index, 2);
    assert_eq!(classic.map_id, 2_484_750);
    assert_eq!((classic.longitude, classic.latitude), (76_584, 14_376));
    assert_eq!((classic.location_id, classic.dungeon_location_id), (26_219, 26_220));
    assert_eq!((classic.world_x, classic.world_y), (19_609_600, 3_684_353));
    assert!(!classic.discovered);
    assert_eq!(classic.key, 0);
    let world = library.world_map().unwrap();
    assert_eq!(world.location_climate_at(ruins.map_pixel.unwrap()), Some(224));
}

#[test]
fn exports_a_dungeon_replacing_one_of_daggerfalls() {
    use daggerfall_dungeon_maker::export::{build_mod, check_project};
    use daggerfall_dungeon_maker::project::{
        DungeonLocation, DungeonReference, ModDetails, Project, ProjectDungeonRoom,
    };
    let Some(mut library) = open_library() else { return };

    let dungeons = library.dungeons();
    let original =
        dungeons.iter().find(|dungeon| dungeon.name == "Privateer's Hold").expect("Privateer's Hold");
    let classic = original.classic.expect("its game record is read");
    let mut project = Project::default();
    let dungeon_id = project.add_empty_dungeon("My Hold");
    let dungeon = project.dungeon_mut(dungeon_id).unwrap();
    dungeon.dungeon_type = original.dungeon_type;
    dungeon.rooms = ProjectDungeonRoom::layout_of(original);
    dungeon.location = DungeonLocation {
        map_pixel: original.map_pixel,
        replaces: Some(DungeonReference::to(original)),
        ..DungeonLocation::copied_from(original)
    };

    let checks = check_project(&project, &mut library);
    assert!(checks[0].is_ready(), "{:?}", checks[0].problems);
    let mod_files = build_mod(&project, &mut library, &ModDetails::default(), "Replace", &[dungeon_id])
        .expect("the mod builds");
    let path = format!("WorldData/location-{}-{}.json", original.region_index, classic.location_index);
    let file = mod_files.files.iter().find(|file| file.path == path).expect("the replacement file");
    let location: serde_json::Value = serde_json::from_str(&file.contents).unwrap();
    assert_eq!(location["Name"], "Privateer's Hold", "a replacement keeps the replaced name");
    assert_eq!(location["LocationIndex"], classic.location_index);
    assert_eq!(location["MapTableData"]["MapId"], classic.map_id);
    assert_eq!(location["Exterior"]["RecordElement"]["Header"]["LocationId"], classic.location_id);
    assert_eq!(location["Dungeon"]["RecordElement"]["Header"]["LocationId"], classic.dungeon_location_id);
    assert_eq!(mod_files.dungeons[0].replaces.as_deref(), Some("Privateer's Hold"));
    assert!(!mod_files.files.iter().any(|file| file.path.contains("locationnew-")));

    // With music of its own, the file goes into the mod with a script that plays it there.
    let music = std::env::temp_dir().join(format!("ddm-music-{}.ogg", std::process::id()));
    std::fs::write(&music, b"OggS").unwrap();
    let location = &mut project.dungeon_mut(dungeon_id).unwrap().location;
    location.custom_music = true;
    location.custom_music_file = music.display().to_string();
    let with_music = build_mod(&project, &mut library, &ModDetails::default(), "Replace", &[dungeon_id])
        .expect("the mod builds");
    assert_eq!(with_music.copied_files.len(), 1);
    assert_eq!(with_music.copied_files[0].path, "Music/replace_music_0.ogg");
    let script =
        with_music.files.iter().find(|file| file.path.ends_with(".cs.txt")).expect("the music script");
    assert!(script.contents.contains(&format!("{{ {} }}", original.region_index)), "{}", script.contents);
    assert!(script.contents.contains(r#"{ "Privateer's Hold" }"#));
    assert!(script.contents.contains(r#"{ "replace_music_0.ogg" }"#));
    let manifest = with_music.files.iter().find(|file| file.path.ends_with(".dfmod.json")).unwrap();
    assert!(manifest.contents.contains("Music/replace_music_0.ogg") && manifest.contents.contains(".cs.txt"));
    std::fs::remove_file(&music).unwrap();
    // A music file that is not there stops the export.
    let checks = check_project(&project, &mut library);
    assert!(
        checks[0].problems.iter().any(|problem| problem.contains("was not found")),
        "{:?}",
        checks[0].problems
    );
    project.dungeon_mut(dungeon_id).unwrap().location.custom_music = false;

    // Another dungeon cannot take the same place.
    let other_id = project.add_empty_dungeon("Second Hold");
    let replaced = project.dungeon(dungeon_id).unwrap().location.clone();
    let other = project.dungeon_mut(other_id).unwrap();
    other.dungeon_type = original.dungeon_type;
    other.rooms = ProjectDungeonRoom::layout_of(original);
    other.location = replaced;
    let checks = check_project(&project, &mut library);
    let problems = &checks[1].problems;
    assert!(problems.iter().any(|problem| problem.contains("already at its spot")), "{problems:?}");
}

#[test]
fn finds_openings_that_lead_to_empty_space() {
    use std::collections::HashMap;

    use daggerfall_dungeon_maker::connectors::unclosed_faces;
    use daggerfall_dungeon_maker::scene::SceneObject;
    let Some(mut library) = open_library() else { return };

    let mut cache = HashMap::new();

    // A corridor on its own leads to empty space at both ends.
    let corridor = SceneObject {
        id: 1,
        name: String::new(),
        group_index: 0,
        kind: daggerfall_dungeon_maker::scene::SceneObjectKind::Model {
            model_id: 67001,
            data: Default::default(),
        },
        position: glam::Vec3::new(25.6, 0.0, 25.6),
        rotation_degrees: glam::Vec3::ZERO,
        visible: true,
        next: None,
    };
    let found = unclosed_faces(std::slice::from_ref(&corridor), |_| true, &mut library, &mut cache);
    let normals: Vec<glam::Vec3> = found.iter().map(|face| face.face.normal.round()).collect();
    assert_eq!(found.len(), 2, "{normals:?}");
    assert!(normals.contains(&glam::Vec3::Z) && normals.contains(&-glam::Vec3::Z), "{normals:?}");

    // Another corridor joined to one end closes it; the far ends stay open.
    // (Each corridor's far end is all that is left.)
    let joined = SceneObject {
        id: 2,
        position: corridor.position + glam::Vec3::new(0.0, 0.0, 12.8),
        ..corridor.clone()
    };
    let found = unclosed_faces(&[corridor, joined], |_| true, &mut library, &mut cache);
    assert_eq!(found.len(), 2);
}

#[test]
fn reads_daggerfalls_dungeon_songs() {
    use daggerfall_dungeon_maker::arena2::parse_hmi;
    use daggerfall_dungeon_maker::music::{song_for_seed, DUNGEON_SONGS};
    let Some(mut library) = open_library() else { return };

    for song in DUNGEON_SONGS {
        let data = library.midi_song(song.file).expect("the song is in MIDI.BSA");
        let parsed = parse_hmi(&data).expect("the song reads");
        let duration = parsed.duration();
        let notes = parsed.timeline().iter().filter(|(_, message)| message[0] & 0xF0 == 0x90).count();
        println!("{}: {notes} note events, {duration:.0} s", song.label());
        assert!(notes > 50, "{} has notes", song.file);
        assert!((10.0..900.0).contains(&duration), "{} lasts {duration} s", song.file);
    }

    // Daggerfall's dungeons between them play every dungeon song.
    let dungeons = library.dungeons();
    let mut played = std::collections::BTreeMap::new();
    for dungeon in dungeons.iter() {
        if let Some(classic) = dungeon.classic {
            *played
                .entry(song_for_seed(classic.dungeon_music_seed, dungeon.region_index).name)
                .or_insert(0) += 1;
        }
    }
    println!("{played:?}");
    assert_eq!(played.len(), DUNGEON_SONGS.len());
}
