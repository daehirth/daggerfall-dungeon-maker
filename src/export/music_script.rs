//! The C# script a mod carries to play dungeons' own music.
//!
//! Daggerfall Unity compiles C# source found in a mod (`*.cs.txt`) when it
//! loads the mod, and runs methods marked `[Invoke]`. The script made here
//! watches where the player is. Inside one of the mod's dungeons with custom
//! music it pauses the game's choice of songs (its `SongManager`), fades the
//! song playing out over [`FADE_SECONDS`] and stops it for good, then plays the
//! dungeon's music file, an audio clip in the mod, on a loop at the game's
//! music volume. (Stopping the game's song player is not enough on its own:
//! it starts its song again by itself, so its song is cleared too.) Music
//! mods that play music with players of their own ([`OTHER_MUSIC_PLAYERS`],
//! such as Dynamic Music) are faded out and paused the same way. Leaving the
//! dungeon stops it and lets the music play again. A dungeon is recognised by
//! its region and name.

/// Class names of music mods' components that the script pauses while a
/// dungeon plays its own music: Dynamic Music's controller, which chooses
/// tracks, and its song player, which plays them.
pub const OTHER_MUSIC_PLAYERS: [&str; 2] = ["DynamicMusic", "DynamicSongPlayer"];

/// How long the game's music takes to fade out before the dungeon's own
/// music starts, in seconds.
pub const FADE_SECONDS: f32 = 0.5;

/// One dungeon whose music the script plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DungeonMusic {
    pub region_index: usize,
    /// The location's name, as the game knows it.
    pub location_name: String,
    /// The music file's name in the mod (for example `mymod_music_0.ogg`).
    pub asset_name: String,
}

/// The script for the mod `mod_file_name`, playing `dungeons`' music.
pub fn music_script(mod_file_name: &str, dungeons: &[DungeonMusic]) -> String {
    let class_name = format!("{}DungeonMusic", identifier(mod_file_name));
    let list = |items: Vec<String>| items.join(", ");
    let regions = list(dungeons.iter().map(|dungeon| dungeon.region_index.to_string()).collect());
    let names = list(dungeons.iter().map(|dungeon| string_literal(&dungeon.location_name)).collect());
    let assets = list(dungeons.iter().map(|dungeon| string_literal(&dungeon.asset_name)).collect());
    let fade_seconds = FADE_SECONDS;
    format!(
        r#"// Written by Daggerfall Dungeon Maker for the mod "{mod_file_name}".
// Plays each of the mod's dungeons' own music inside it, instead of the game's.
using System.Collections.Generic;
using UnityEngine;
using DaggerfallWorkshop;
using DaggerfallWorkshop.Game;
using DaggerfallWorkshop.Game.Utility.ModSupport;

public class {class_name} : MonoBehaviour
{{
    // The dungeons, by region and name, and the music each plays (an audio clip in the mod).
    static readonly int[] Regions = {{ {regions} }};
    static readonly string[] LocationNames = {{ {names} }};
    static readonly string[] Clips = {{ {assets} }};

    // Seconds the game's music fades out for before the dungeon's music starts.
    const float FadeSeconds = {fade_seconds:.1}f;
    // Music mods' components (by class name) paused while the dungeon's music plays.
    static readonly string[] OtherMusicControllers = {{ "DynamicMusic" }};
    static readonly string[] OtherMusicPlayers = {{ "DynamicSongPlayer" }};

    static Mod mod;
    AudioSource source;
    SongManager songManager;
    // The dungeon whose music plays (or is about to), or -1.
    int playing = -1;
    // Its music, or null if the mod lacks it (the other music then keeps playing).
    AudioClip clip;
    // When the other music started fading out (unscaled time), or -1 once it has.
    float fadeStarted = -1f;
    // What was paused, to be given back on leaving: components, and audio
    // sources with the volume each had.
    readonly List<Behaviour> pausedControllers = new List<Behaviour>();
    readonly List<Behaviour> pausedPlayers = new List<Behaviour>();
    readonly List<AudioSource> fadedSources = new List<AudioSource>();
    readonly List<float> fadedVolumes = new List<float>();

    [Invoke(StateManager.StateTypes.Start, 0)]
    public static void Init(InitParams initParams)
    {{
        mod = initParams.Mod;
        GameObject player = new GameObject(mod.Title + " dungeon music");
        DontDestroyOnLoad(player);
        player.AddComponent<{class_name}>();
        mod.IsReady = true;
    }}

    void Awake()
    {{
        source = gameObject.AddComponent<AudioSource>();
        source.loop = true;
        source.playOnAwake = false;
        source.spatialBlend = 0;
    }}

    void Update()
    {{
        int dungeon = CurrentDungeon();
        if (dungeon != playing)
        {{
            Stop();
            if (dungeon >= 0)
                Begin(dungeon);
        }}
    }}

    // After every Update, so the volumes set here are the ones heard.
    void LateUpdate()
    {{
        if (playing < 0 || clip == null)
            return;
        if (fadeStarted >= 0f)
        {{
            float progress = (Time.unscaledTime - fadeStarted) / FadeSeconds;
            if (progress < 1f)
            {{
                for (int i = 0; i < fadedSources.Count; i++)
                {{
                    if (fadedSources[i] != null)
                        fadedSources[i].volume = fadedVolumes[i] * (1f - progress);
                }}
                return;
            }}
            // Faded out: stop the other music for good, then start the dungeon's.
            DaggerfallSongPlayer gamePlayer = GameSongPlayer();
            if (gamePlayer != null)
            {{
                gamePlayer.Stop();
                gamePlayer.Song = SongFiles.song_none;
            }}
            foreach (Behaviour player in pausedPlayers)
            {{
                if (player != null)
                    player.enabled = false;
            }}
            foreach (AudioSource faded in fadedSources)
            {{
                if (faded != null)
                    faded.mute = true;
            }}
            fadeStarted = -1f;
            source.clip = clip;
            source.Play();
        }}
        source.volume = DaggerfallUnity.Settings.MusicVolume;
    }}

    // The index of the dungeon the player is inside, or -1.
    static int CurrentDungeon()
    {{
        GameManager game = GameManager.Instance;
        if (game == null || game.PlayerEnterExit == null || game.PlayerGPS == null)
            return -1;
        if (!game.PlayerEnterExit.IsPlayerInsideDungeon || !game.PlayerGPS.HasCurrentLocation)
            return -1;
        var location = game.PlayerGPS.CurrentLocation;
        for (int i = 0; i < Regions.Length; i++)
        {{
            if (Regions[i] == location.RegionIndex && LocationNames[i] == location.Name)
                return i;
        }}
        return -1;
    }}

    void Begin(int dungeon)
    {{
        playing = dungeon;
        clip = mod.GetAsset<AudioClip>(Clips[dungeon]);
        if (clip == null)
        {{
            // Without the clip the other music keeps playing.
            Debug.LogWarningFormat("{{0}}: music {{1}} was not found in the mod", mod.Title, Clips[dungeon]);
            return;
        }}
        // Whatever chooses songs stops now; the players fade out before this music starts.
        SongManager manager = GameSongManager();
        if (manager != null && manager.enabled)
        {{
            manager.enabled = false;
            pausedControllers.Add(manager);
        }}
        DaggerfallSongPlayer gamePlayer = GameSongPlayer();
        if (gamePlayer != null)
            AddFaded(gamePlayer.AudioSource);
        foreach (MonoBehaviour behaviour in FindObjectsOfType<MonoBehaviour>())
        {{
            if (behaviour == null || !behaviour.enabled)
                continue;
            string name = behaviour.GetType().Name;
            if (System.Array.IndexOf(OtherMusicControllers, name) >= 0)
            {{
                behaviour.enabled = false;
                pausedControllers.Add(behaviour);
            }}
            else if (System.Array.IndexOf(OtherMusicPlayers, name) >= 0)
            {{
                pausedPlayers.Add(behaviour);
                foreach (AudioSource playerSource in behaviour.GetComponents<AudioSource>())
                    AddFaded(playerSource);
            }}
        }}
        fadeStarted = Time.unscaledTime;
    }}

    void AddFaded(AudioSource faded)
    {{
        if (faded == null || fadedSources.Contains(faded))
            return;
        fadedSources.Add(faded);
        fadedVolumes.Add(faded.volume);
    }}

    void Stop()
    {{
        if (playing < 0)
            return;
        source.Stop();
        source.clip = null;
        clip = null;
        fadeStarted = -1f;
        playing = -1;
        // Everything paused plays again, at the volume it had.
        for (int i = 0; i < fadedSources.Count; i++)
        {{
            if (fadedSources[i] != null)
            {{
                fadedSources[i].mute = false;
                fadedSources[i].volume = fadedVolumes[i];
            }}
        }}
        foreach (Behaviour player in pausedPlayers)
        {{
            if (player != null)
                player.enabled = true;
        }}
        foreach (Behaviour controller in pausedControllers)
        {{
            if (controller != null)
                controller.enabled = true;
        }}
        fadedSources.Clear();
        fadedVolumes.Clear();
        pausedPlayers.Clear();
        pausedControllers.Clear();
    }}

    SongManager GameSongManager()
    {{
        if (songManager == null)
            songManager = FindObjectOfType<SongManager>();
        return songManager;
    }}

    DaggerfallSongPlayer GameSongPlayer()
    {{
        SongManager manager = GameSongManager();
        return manager != null ? manager.SongPlayer : null;
    }}
}}
"#
    )
}

/// `name` as a C# identifier: letters, digits and underscores, not starting with a digit.
fn identifier(name: &str) -> String {
    let mut identifier: String = name
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character } else { '_' })
        .collect();
    if !identifier.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_') {
        identifier.insert(0, '_');
    }
    identifier
}

/// `text` as a C# string literal.
fn string_literal(text: &str) -> String {
    let mut literal = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            character if (character as u32) < 0x20 => {
                literal.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => literal.push(character),
        }
    }
    literal.push('"');
    literal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_dungeons_into_the_script() {
        let script = music_script(
            "My_Mod",
            &[
                DungeonMusic {
                    region_index: 17,
                    location_name: "Privateer's \"Hold\"".to_string(),
                    asset_name: "my_mod_music_0.ogg".to_string(),
                },
                DungeonMusic {
                    region_index: 5,
                    location_name: "Crypt".to_string(),
                    asset_name: "my_mod_music_1.mp3".to_string(),
                },
            ],
        );
        assert!(script.contains("public class My_ModDungeonMusic : MonoBehaviour"));
        assert!(script.contains("static readonly int[] Regions = { 17, 5 };"));
        assert!(script.contains(r#"{ "Privateer's \"Hold\"", "Crypt" }"#));
        assert!(script.contains(r#"{ "my_mod_music_0.ogg", "my_mod_music_1.mp3" }"#));
        assert!(script.contains("player.AddComponent<My_ModDungeonMusic>();"));
        // The game's music fades out for half a second, and its song is cleared so it cannot start again.
        assert!(script.contains("const float FadeSeconds = 0.5f;"));
        assert!(script.contains("gamePlayer.Song = SongFiles.song_none;"));
        // Music mods with players of their own are paused too.
        for name in OTHER_MUSIC_PLAYERS {
            assert!(script.contains(&format!("\"{name}\"")), "{name}");
        }
        // Braces of the C# code survive the formatting.
        assert_eq!(script.matches('{').count(), script.matches('}').count());
    }

    /// Writes a sample script to the path in `DDM_SCRIPT_OUT`, for compiling
    /// it with other tools: `cargo test sample_script -- --ignored`.
    #[test]
    #[ignore]
    fn sample_script() {
        let Some(path) = std::env::var_os("DDM_SCRIPT_OUT") else { return };
        let dungeon = DungeonMusic {
            region_index: 17,
            location_name: "Privateer's Hold".to_string(),
            asset_name: "sample_music_0.ogg".to_string(),
        };
        std::fs::write(path, music_script("Sample", &[dungeon])).unwrap();
    }

    #[test]
    fn makes_identifiers() {
        assert_eq!(identifier("2nd mod"), "_2nd_mod");
        assert_eq!(identifier("Mod_1"), "Mod_1");
    }
}
