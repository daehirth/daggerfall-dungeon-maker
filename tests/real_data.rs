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
