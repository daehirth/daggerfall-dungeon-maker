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
