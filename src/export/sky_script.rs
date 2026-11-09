//! The C# script a mod carries to show dungeons' skyboxes.
//!
//! Daggerfall Unity draws every dungeon against black: on going inside, its
//! `CameraClearManager` makes the main camera clear to black, hiding the sky.
//! The script made here watches where the player is. Inside one of the mod's
//! dungeons with a skybox it makes the camera draw a skybox instead, the
//! dungeon's own, seen through any openings to the outside. It does that every
//! frame after the game's own camera handling, so the game cannot undo it.
//! Leaving the dungeon puts back the game's skybox and camera setting.
//!
//! Inside dungeons the game also turns on a dense black fog, and tells its
//! post-processing fog (`PostProcessLayer.fog`) not to leave out the skybox
//! (`excludeSkybox`). The sky is infinitely far away, so the fog would hide it
//! completely; the script keeps the fog off the sky while it shows one. It
//! finds that setting by name at run time, so it needs no reference to the
//! post-processing package and still works (without it) where there is none.
//!
//! The skybox is a material in the mod, which the Unity build makes from the
//! dungeon's image (see [`super::unity`]). If the material is missing the
//! script makes one from the image itself. A dungeon is recognised by its
//! region and name.

use super::music_script::{identifier, string_literal, CURRENT_DUNGEON_METHOD};

/// Unity's shader for skyboxes made from one panoramic image.
pub const PANORAMIC_SHADER: &str = "Skybox/Panoramic";

/// One dungeon whose skybox the script shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DungeonSky {
    pub region_index: usize,
    /// The location's name, as the game knows it.
    pub location_name: String,
    /// The skybox material's name in the mod (for example `mymod_sky_0.mat`).
    pub material_name: String,
    /// The image's name in the mod (for example `mymod_sky_0.png`).
    pub image_name: String,
}

/// The script for the mod `mod_file_name`, showing `dungeons`' skyboxes.
pub fn sky_script(mod_file_name: &str, dungeons: &[DungeonSky]) -> String {
    let class_name = format!("{}DungeonSky", identifier(mod_file_name));
    let list = |items: Vec<String>| items.join(", ");
    let regions = list(dungeons.iter().map(|dungeon| dungeon.region_index.to_string()).collect());
    let names = list(dungeons.iter().map(|dungeon| string_literal(&dungeon.location_name)).collect());
    let materials = list(dungeons.iter().map(|dungeon| string_literal(&dungeon.material_name)).collect());
    let images = list(dungeons.iter().map(|dungeon| string_literal(&dungeon.image_name)).collect());
    let shader = string_literal(PANORAMIC_SHADER);
    let current_dungeon = CURRENT_DUNGEON_METHOD;
    format!(
        r#"// Written by Daggerfall Dungeon Maker for the mod "{mod_file_name}".
// Shows each of the mod's dungeons' own skybox behind it, instead of black.
using UnityEngine;
using DaggerfallWorkshop;
using DaggerfallWorkshop.Game;
using DaggerfallWorkshop.Game.Utility.ModSupport;

public class {class_name} : MonoBehaviour
{{
    // The dungeons, by region and name, and the skybox each shows: a material
    // in the mod, made from the image when the mod was built, or else the image.
    static readonly int[] Regions = {{ {regions} }};
    static readonly string[] LocationNames = {{ {names} }};
    static readonly string[] Skyboxes = {{ {materials} }};
    static readonly string[] Images = {{ {images} }};

    static Mod mod;
    // The dungeon whose skybox shows, or -1.
    int showing = -1;
    // Its skybox, or null if the mod lacks it (the dungeon then stays black).
    Material skybox;
    // Whether the skybox was made here from the image, to be destroyed after.
    bool madeSkybox;
    // The skybox the game had, given back on leaving.
    Material gameSkybox;
    // Whether the camera was switched to the skybox, and whether the log was
    // told something else switched it back, or that there was no camera.
    bool applied;
    bool reportedOverride;
    bool reportedNoCamera;
    // The camera's post-processing fog settings and their excludeSkybox field,
    // once looked for (null if there are none), and the value the game had.
    bool fogChecked;
    object fogSettings;
    System.Reflection.FieldInfo excludeSkyboxField;
    bool gameExcludedSkybox;

    [Invoke(StateManager.StateTypes.Start, 0)]
    public static void Init(InitParams initParams)
    {{
        mod = initParams.Mod;
        GameObject sky = new GameObject(mod.Title + " dungeon sky");
        DontDestroyOnLoad(sky);
        sky.AddComponent<{class_name}>();
        mod.IsReady = true;
        Debug.LogFormat("{{0}}: dungeon skybox script started for {{1}} dungeon(s)", mod.Title, Regions.Length);
    }}

    // After the game's own camera handling (in Update), so the skybox stays.
    void LateUpdate()
    {{
        int dungeon = CurrentDungeon();
        if (dungeon != showing)
        {{
            Stop();
            if (dungeon >= 0)
                Begin(dungeon);
        }}
        if (showing < 0 || skybox == null)
            return;
        Camera camera = MainCamera();
        if (camera == null)
        {{
            if (!reportedNoCamera)
                Debug.LogWarningFormat("{{0}}: no main camera to show the skybox with", mod.Title);
            reportedNoCamera = true;
            return;
        }}
        if (camera.clearFlags != CameraClearFlags.Skybox)
        {{
            // Something else (the game, or another mod) switched it back.
            if (applied && !reportedOverride)
            {{
                Debug.LogWarningFormat("{{0}}: camera {{1}} was switched from the skybox to {{2}} by something else; switching it back every frame", mod.Title, camera.name, camera.clearFlags);
                reportedOverride = true;
            }}
            camera.clearFlags = CameraClearFlags.Skybox;
        }}
        applied = true;
        if (RenderSettings.skybox != skybox)
            RenderSettings.skybox = skybox;
        // The dungeon's black fog must not cover the sky.
        if (!fogChecked)
        {{
            FindFog(camera);
            Debug.LogFormat("{{0}}: {{1}}", mod.Title, fogSettings != null
                ? "keeping the dungeon's fog off the skybox"
                : "found no post-processing fog to keep off the skybox");
        }}
        if (fogSettings != null && !(bool)excludeSkyboxField.GetValue(fogSettings))
            excludeSkyboxField.SetValue(fogSettings, true);
    }}

    // Finds the post-processing fog settings of `camera` (its PostProcessLayer's
    // fog), by name, and remembers the game's excludeSkybox value.
    void FindFog(Camera camera)
    {{
        fogChecked = true;
        foreach (MonoBehaviour behaviour in camera.GetComponents<MonoBehaviour>())
        {{
            if (behaviour == null || behaviour.GetType().Name != "PostProcessLayer")
                continue;
            System.Reflection.FieldInfo fogField = behaviour.GetType().GetField("fog");
            object fog = fogField != null ? fogField.GetValue(behaviour) : null;
            System.Reflection.FieldInfo exclude = fog != null ? fog.GetType().GetField("excludeSkybox") : null;
            if (exclude != null && exclude.FieldType == typeof(bool))
            {{
                fogSettings = fog;
                excludeSkyboxField = exclude;
                gameExcludedSkybox = (bool)exclude.GetValue(fog);
            }}
            return;
        }}
    }}

{current_dungeon}
    void Begin(int dungeon)
    {{
        showing = dungeon;
        gameSkybox = RenderSettings.skybox;
        skybox = mod.GetAsset<Material>(Skyboxes[dungeon]);
        if (skybox == null)
        {{
            Texture2D image = mod.GetAsset<Texture2D>(Images[dungeon]);
            Shader shader = Shader.Find({shader});
            if (image != null && shader != null)
            {{
                skybox = new Material(shader);
                skybox.SetTexture("_MainTex", image);
                madeSkybox = true;
            }}
        }}
        if (skybox == null)
        {{
            Debug.LogWarningFormat("{{0}}: skybox {{1}} was not found in the mod", mod.Title, Skyboxes[dungeon]);
            return;
        }}
        Debug.LogFormat("{{0}}: showing skybox {{1}} in {{2}} ({{3}}; shader {{4}}, supported: {{5}})", mod.Title,
            Skyboxes[dungeon], LocationNames[dungeon], madeSkybox ? "made from the image" : "from the mod",
            skybox.shader != null ? skybox.shader.name : "none", skybox.shader != null && skybox.shader.isSupported);
    }}

    void Stop()
    {{
        if (showing < 0)
            return;
        showing = -1;
        if (skybox != null)
        {{
            RenderSettings.skybox = gameSkybox;
            Camera camera = MainCamera();
            if (camera != null)
                camera.clearFlags = GameClearFlags();
        }}
        // Still inside (another dungeon), the fog covers the sky again as the game had it.
        // Outside, the game has already set the fog for outdoors, which is left alone.
        if (fogSettings != null)
        {{
            GameManager game = GameManager.Instance;
            if (game != null && game.PlayerEnterExit != null && game.PlayerEnterExit.IsPlayerInside)
                excludeSkyboxField.SetValue(fogSettings, gameExcludedSkybox);
        }}
        fogChecked = false;
        fogSettings = null;
        excludeSkyboxField = null;
        if (madeSkybox)
            Destroy(skybox);
        skybox = null;
        madeSkybox = false;
        gameSkybox = null;
        applied = false;
        reportedOverride = false;
    }}

    // How the game clears the camera where the player is now, as its
    // CameraClearManager does: black inside, the sky showing outside.
    static CameraClearFlags GameClearFlags()
    {{
        if (DaggerfallUnity.Settings.RetroRenderingMode > 0)
            return CameraClearFlags.Depth;
        GameManager game = GameManager.Instance;
        bool inside = game != null && game.PlayerEnterExit != null && game.PlayerEnterExit.IsPlayerInside;
        CameraClearManager manager = FindObjectOfType<CameraClearManager>();
        if (manager != null)
            return inside ? manager.cameraClearInterior : manager.cameraClearExterior;
        return inside ? CameraClearFlags.SolidColor : CameraClearFlags.Depth;
    }}

    static Camera MainCamera()
    {{
        GameManager game = GameManager.Instance;
        return game != null ? game.MainCamera : null;
    }}
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<DungeonSky> {
        vec![
            DungeonSky {
                region_index: 17,
                location_name: "Privateer's \"Hold\"".to_string(),
                material_name: "my_mod_sky_0.mat".to_string(),
                image_name: "my_mod_sky_0.png".to_string(),
            },
            DungeonSky {
                region_index: 5,
                location_name: "Crypt".to_string(),
                material_name: "my_mod_sky_1.mat".to_string(),
                image_name: "my_mod_sky_1.jpg".to_string(),
            },
        ]
    }

    #[test]
    fn writes_the_dungeons_into_the_script() {
        let script = sky_script("My_Mod", &sample());
        assert!(script.contains("public class My_ModDungeonSky : MonoBehaviour"));
        assert!(script.contains("static readonly int[] Regions = { 17, 5 };"));
        assert!(script.contains(r#"{ "Privateer's \"Hold\"", "Crypt" }"#));
        assert!(script.contains(r#"{ "my_mod_sky_0.mat", "my_mod_sky_1.mat" }"#));
        assert!(script.contains(r#"{ "my_mod_sky_0.png", "my_mod_sky_1.jpg" }"#));
        assert!(script.contains("sky.AddComponent<My_ModDungeonSky>();"));
        assert!(script.contains("static int CurrentDungeon()"));
        assert!(script.contains("camera.clearFlags = CameraClearFlags.Skybox;"));
        assert!(script.contains(r#"Shader.Find("Skybox/Panoramic")"#));
        // The dungeon's fog is kept off the sky.
        assert!(script.contains(r#"GetField("excludeSkybox")"#));
        assert!(script.contains(r#"!= "PostProcessLayer""#));
        // Braces of the C# code survive the formatting.
        assert_eq!(script.matches('{').count(), script.matches('}').count());
    }

    /// Writes a sample script to the path in `DDM_SKY_SCRIPT_OUT`, for
    /// compiling it with other tools: `cargo test sample_sky_script -- --ignored`.
    #[test]
    #[ignore]
    fn sample_sky_script() {
        let Some(path) = std::env::var_os("DDM_SKY_SCRIPT_OUT") else { return };
        std::fs::write(path, sky_script("Sample", &sample())).unwrap();
    }
}
