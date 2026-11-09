//! Building an exported mod into a `.dfmod` with the Unity editor.
//!
//! A `.dfmod` is a Unity asset bundle, which only Unity can write. Daggerfall
//! Unity's own Mod Builder runs inside the editor; this runs the editor
//! without a window instead (`-batchmode`) and builds the bundle the same
//! way. The mod is all text files, so any Unity project will do: a small one
//! is made the first time and kept for later builds. The editor should be the
//! version Daggerfall Unity is built with ([`UNITY_VERSION`]).
//!
//! Each build target (Windows, Linux, macOS) gets its own `.dfmod`; building
//! for a target needs that target's build support installed in the editor.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

use super::MOD_ASSET_ROOT;

/// Unity version Daggerfall Unity 1.1 is built with.
pub const UNITY_VERSION: &str = "2019.4.40f1";
/// Unity versions whose asset bundles Daggerfall Unity can load.
const COMPATIBLE_VERSION_PREFIX: &str = "2019.4.";
/// Editor script, inside the Unity project, that builds the bundle.
const BUILDER_SCRIPT_PATH: &str = "Assets/Editor/DungeonMakerModBuilder.cs";
/// Where the bundles are built, inside the Unity project, before being copied out.
const BUILD_OUTPUT_FOLDER: &str = "DungeonMakerBuild";

/// The editor script. Run with `-executeMethod DungeonMakerModBuilder.Build`.
const BUILDER_SCRIPT: &str = r#"// Written by Daggerfall Dungeon Maker, which replaces it before each build.
// Builds a Daggerfall Unity mod (.dfmod) from every file in a folder, as
// Daggerfall Unity's Mod Builder does. Images in its Sky folder first get a
// skybox material each, which the mod's sky script shows in game.
using System;
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEngine;

public static class DungeonMakerModBuilder
{
    public static void Build()
    {
        string assetFolder = Argument("-ddmAssets");
        string bundleName = Argument("-ddmBundle");
        string output = Argument("-ddmOutput");
        string[] targets = Argument("-ddmTargets").Split(',');

        AssetDatabase.Refresh(ImportAssetOptions.ForceUpdate);
        MakeSkyboxes(assetFolder + "/Sky");
        string[] assets = Directory.GetFiles(assetFolder, "*", SearchOption.AllDirectories)
            .Where(path => !path.EndsWith(".meta", StringComparison.Ordinal))
            .Select(path => path.Replace('\\', '/'))
            .ToArray();
        if (assets.Length == 0)
            throw new Exception("There are no files to build in " + assetFolder);

        var build = new AssetBundleBuild { assetBundleName = bundleName, assetNames = assets };
        foreach (string target in targets)
        {
            var buildTarget = (BuildTarget)Enum.Parse(typeof(BuildTarget), target);
            string folder = Path.Combine(output, target);
            Directory.CreateDirectory(folder);
            var manifest = BuildPipeline.BuildAssetBundles(
                folder, new[] { build }, BuildAssetBundleOptions.ChunkBasedCompression, buildTarget);
            if (manifest == null)
                throw new Exception("Unity could not build the mod for " + target);
        }
    }

    // A panoramic skybox material next to each image in `folder`, with the
    // image's name: the image is a 360-degree panorama (twice as wide as high).
    static void MakeSkyboxes(string folder)
    {
        if (!Directory.Exists(folder))
            return;
        string[] images = Directory.GetFiles(folder)
            .Select(path => path.Replace('\\', '/'))
            .Where(path => new[] { ".png", ".jpg", ".jpeg" }.Contains(Path.GetExtension(path).ToLowerInvariant()))
            .ToArray();
        if (images.Length == 0)
            return;
        Shader shader = Shader.Find("Skybox/Panoramic");
        if (shader == null)
            throw new Exception("Unity has no Skybox/Panoramic shader to make the skyboxes with");
        foreach (string image in images)
        {
            var importer = AssetImporter.GetAtPath(image) as TextureImporter;
            if (importer != null)
            {
                // Wraps around sideways without a seam, and keeps its detail.
                importer.textureType = TextureImporterType.Default;
                importer.textureShape = TextureImporterShape.Texture2D;
                importer.mipmapEnabled = false;
                importer.wrapModeU = TextureWrapMode.Repeat;
                importer.wrapModeV = TextureWrapMode.Clamp;
                importer.npotScale = TextureImporterNPOTScale.None;
                importer.maxTextureSize = 8192;
                importer.SaveAndReimport();
            }
            var texture = AssetDatabase.LoadAssetAtPath<Texture2D>(image);
            if (texture == null)
                throw new Exception("Unity could not import the skybox image " + image);
            var material = new Material(shader);
            material.SetTexture("_MainTex", texture);
            // Latitude-longitude layout, all 360 degrees.
            material.SetFloat("_Mapping", 1f);
            material.SetFloat("_ImageType", 0f);
            AssetDatabase.CreateAsset(material, Path.ChangeExtension(image, ".mat"));
        }
        AssetDatabase.SaveAssets();
    }

    static string Argument(string name)
    {
        string[] arguments = Environment.GetCommandLineArgs();
        int index = Array.IndexOf(arguments, name);
        if (index < 0 || index + 1 >= arguments.Length)
            throw new Exception("Missing argument " + name);
        return arguments[index + 1];
    }
}
"#;

/// A platform a `.dfmod` can be built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BuildTarget {
    Windows,
    Linux,
    MacOs,
}

impl BuildTarget {
    pub const ALL: [Self; 3] = [Self::Windows, Self::Linux, Self::MacOs];

    /// Name shown to the user, and of the folder its `.dfmod` goes in.
    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
        }
    }

    /// Unity's name for it.
    fn unity_name(self) -> &'static str {
        match self {
            Self::Windows => "StandaloneWindows",
            Self::Linux => "StandaloneLinux64",
            Self::MacOs => "StandaloneOSX",
        }
    }

    /// The platform this program runs on.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Windows
        }
    }
}

/// What to build.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    /// The Unity editor program.
    pub unity_editor: PathBuf,
    /// The Unity project used for building; made if it does not exist yet.
    pub build_project: PathBuf,
    /// The exported mod's folder (see [`super::write_mod`]); the `.dfmod`s go here too.
    pub mod_folder: PathBuf,
    /// The mod's file name (see [`super::ModFiles::file_name`]).
    pub file_name: String,
    pub targets: Vec<BuildTarget>,
    /// Daggerfall Unity's `Mods` folder, to copy this platform's `.dfmod` into.
    pub mods_folder: Option<PathBuf>,
}

/// News from a running build.
#[derive(Debug, Clone)]
pub enum BuildEvent {
    /// What the build is doing now.
    Step(String),
    /// The build ended: the `.dfmod` files made, or what went wrong.
    Finished(Result<Vec<PathBuf>, String>),
}

/// A build running in the background.
pub struct BuildJob {
    events: mpsc::Receiver<BuildEvent>,
}

impl BuildJob {
    /// Starts building `request` on another thread.
    pub fn start(request: BuildRequest) -> Self {
        let (sender, events) = mpsc::channel();
        std::thread::spawn(move || {
            let step = |text: String| {
                let _ = sender.send(BuildEvent::Step(text));
            };
            let result = build(&request, &step);
            let _ = sender.send(BuildEvent::Finished(result));
        });
        Self { events }
    }

    /// The events that arrived since the last call.
    pub fn poll(&self) -> Vec<BuildEvent> {
        self.events.try_iter().collect()
    }
}

/// Runs the whole build, reporting each step to `step`.
fn build(request: &BuildRequest, step: &dyn Fn(String)) -> Result<Vec<PathBuf>, String> {
    if request.targets.is_empty() {
        return Err("Choose at least one platform to build for".to_string());
    }
    if !request.unity_editor.is_file() {
        return Err(format!("The Unity editor was not found at '{}'", request.unity_editor.display()));
    }
    let project = &request.build_project;
    if !project.join("Assets").is_dir() || !project.join("ProjectSettings").is_dir() {
        step(
            "Making a Unity project to build with (only the first time; this takes a few minutes)…"
                .to_string(),
        );
        if let Some(parent) = project.parent() {
            create_folder(parent)?;
        }
        let log = project.with_extension("create.log");
        run_unity(
            &request.unity_editor,
            &["-batchmode", "-quit", "-createProject", &path_text(project), "-logFile", &path_text(&log)],
            &log,
        )
        .map_err(|error| format!("Unity could not make the project: {error}"))?;
    }

    step("Copying the mod into the Unity project…".to_string());
    let asset_folder = format!("{MOD_ASSET_ROOT}/{}", request.file_name);
    let asset_path = project.join(&asset_folder);
    if asset_path.exists() {
        std::fs::remove_dir_all(&asset_path)
            .map_err(|error| format!("Could not clear '{}': {error}", asset_path.display()))?;
    }
    copy_mod_files(&request.mod_folder, &asset_path, &request.file_name)?;
    let script = project.join(BUILDER_SCRIPT_PATH);
    if let Some(parent) = script.parent() {
        create_folder(parent)?;
    }
    write_file(&script, BUILDER_SCRIPT)?;

    let output = project.join(BUILD_OUTPUT_FOLDER);
    if output.exists() {
        std::fs::remove_dir_all(&output)
            .map_err(|error| format!("Could not clear '{}': {error}", output.display()))?;
    }
    let names: Vec<&str> = request.targets.iter().map(|target| target.name()).collect();
    step(format!("Building the .dfmod for {} with Unity…", names.join(", ")));
    let bundle_name = format!("{}.dfmod", request.file_name.to_lowercase());
    let targets: Vec<&str> = request.targets.iter().map(|target| target.unity_name()).collect();
    let log = project.join("DungeonMakerBuild.log");
    run_unity(
        &request.unity_editor,
        &[
            "-batchmode",
            "-quit",
            "-projectPath",
            &path_text(project),
            "-executeMethod",
            "DungeonMakerModBuilder.Build",
            "-logFile",
            &path_text(&log),
            "-ddmAssets",
            &asset_folder,
            "-ddmBundle",
            &bundle_name,
            "-ddmOutput",
            &path_text(&output),
            "-ddmTargets",
            &targets.join(","),
        ],
        &log,
    )?;

    step("Copying the .dfmod files out…".to_string());
    let mut built = Vec::new();
    for target in &request.targets {
        let source = output.join(target.unity_name()).join(&bundle_name);
        if !source.is_file() {
            return Err(format!("Unity finished but made no .dfmod for {}", target.name()));
        }
        let folder = request.mod_folder.join(target.name());
        create_folder(&folder)?;
        let destination = folder.join(&bundle_name);
        copy_file(&source, &destination)?;
        if *target == BuildTarget::current() {
            if let Some(mods_folder) = &request.mods_folder {
                create_folder(mods_folder)?;
                copy_file(&source, &mods_folder.join(&bundle_name))?;
            }
        }
        built.push(destination);
    }
    Ok(built)
}

/// Copies the mod's files from `mod_folder` to `destination`: the manifest,
/// the scripts, the world data, the music and the skies (not the readme or earlier builds).
fn copy_mod_files(mod_folder: &Path, destination: &Path, file_name: &str) -> Result<(), String> {
    create_folder(destination)?;
    let manifest = format!("{file_name}{}", super::MANIFEST_EXTENSION);
    copy_file(&mod_folder.join(&manifest), &destination.join(&manifest))?;
    let entries = std::fs::read_dir(mod_folder)
        .map_err(|error| format!("Could not read '{}': {error}", mod_folder.display()))?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().ends_with(".cs.txt") {
            copy_file(&entry.path(), &destination.join(entry.file_name()))?;
        }
    }
    for folder in [super::WORLD_DATA_FOLDER, super::MUSIC_FOLDER, super::SKY_FOLDER] {
        let source = mod_folder.join(folder);
        let Ok(entries) = std::fs::read_dir(&source) else { continue };
        create_folder(&destination.join(folder))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                copy_file(&path, &destination.join(folder).join(entry.file_name()))?;
            }
        }
    }
    Ok(())
}

/// Runs the Unity editor with `arguments` and waits for it. On failure,
/// explains what went wrong from the end of its `log`.
fn run_unity(editor: &Path, arguments: &[&str], log: &Path) -> Result<(), String> {
    let status = Command::new(editor)
        .args(arguments)
        .status()
        .map_err(|error| format!("could not start Unity: {error}"))?;
    if status.success() {
        return Ok(());
    }
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut message = if is_license_failure(&text) {
        LICENSE_HELP.to_string()
    } else {
        let details = log_errors(&text);
        match details.is_empty() {
            true => format!("Unity stopped with {status}."),
            false => format!("Unity stopped with {status}.\n{details}"),
        }
    };
    message.push_str(&format!("\nThe full log is in '{}'.", log.display()));
    Err(message)
}

/// What to do when Unity is not activated, which it must be to run without a window.
const LICENSE_HELP: &str = "Unity is not activated for building without a window. Unity 2019.4 needs a \
     license file (Unity_lic.ulf), which newer Unity versions and Unity Hub's sign-in do not make on \
     their own. To get one: in Unity Hub, sign in, add a free Personal license under Preferences > \
     Licenses, then open Unity 2019.4 once from Unity Hub (any project, even a new empty one) and close \
     it. Then export again. (Files only works without Unity.)";

/// Whether a Unity log says it stopped because Unity is not activated.
fn is_license_failure(log: &str) -> bool {
    log.contains("has not been activated with a valid License")
        || log.contains("Failed to activate/update license")
        || log.contains("No valid Unity Editor license found")
}

/// The lines of a Unity log that explain a failure, or its last lines.
fn log_errors(log: &str) -> String {
    let lines: Vec<&str> = log.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let interesting: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains("error")
                || lower.contains("exception")
                || lower.contains("licens")
                || lower.contains("not installed")
                || lower.contains("unable to")
        })
        .collect();
    let chosen =
        if interesting.is_empty() { &lines[lines.len().saturating_sub(8)..] } else { &interesting[..] };
    chosen.iter().rev().take(12).rev().copied().collect::<Vec<_>>().join("\n")
}

/// Unity editors installed in Unity Hub's usual places, compatible
/// versions first and newest first.
pub fn find_unity_editors() -> Vec<PathBuf> {
    let mut hubs: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(folder) = std::env::var_os(variable) {
                hubs.push(PathBuf::from(folder).join("Unity").join("Hub").join("Editor"));
            }
        }
    } else if cfg!(target_os = "macos") {
        hubs.push(PathBuf::from("/Applications/Unity/Hub/Editor"));
    } else if let Some(home) = std::env::var_os("HOME") {
        hubs.push(PathBuf::from(home).join("Unity").join("Hub").join("Editor"));
    }
    let mut editors: Vec<(String, PathBuf)> = Vec::new();
    for hub in hubs {
        let Ok(entries) = std::fs::read_dir(&hub) else { continue };
        for entry in entries.flatten() {
            let version = entry.file_name().to_string_lossy().to_string();
            let program = if cfg!(windows) {
                entry.path().join("Editor").join("Unity.exe")
            } else if cfg!(target_os = "macos") {
                entry.path().join("Unity.app").join("Contents").join("MacOS").join("Unity")
            } else {
                entry.path().join("Editor").join("Unity")
            };
            if program.is_file() && !editors.iter().any(|(_, known)| *known == program) {
                editors.push((version, program));
            }
        }
    }
    editors.sort_by(|(first, _), (second, _)| {
        let compatible = |version: &str| version.starts_with(COMPATIBLE_VERSION_PREFIX);
        compatible(second).cmp(&compatible(first)).then_with(|| second.cmp(first))
    });
    editors.into_iter().map(|(_, program)| program).collect()
}

/// The Unity version of the editor at `editor`, from the folder Unity Hub
/// installs it in (for example `2019.4.40f1`), if it can be told.
pub fn editor_version(editor: &Path) -> Option<String> {
    editor
        .ancestors()
        .filter_map(|folder| folder.file_name()?.to_str())
        .find(|name| name.chars().next().is_some_and(|first| first.is_ascii_digit()) && name.contains('.'))
        .map(str::to_string)
}

/// Whether Daggerfall Unity can load bundles made by Unity `version`.
pub fn is_compatible_version(version: &str) -> bool {
    version.starts_with(COMPATIBLE_VERSION_PREFIX)
}

/// Where the Unity project used for building is kept: in the user's local
/// application data (cache) folder.
pub fn default_build_project() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("DaggerfallDungeonMaker").join("UnityModBuilder")
}

fn path_text(path: &Path) -> String {
    path.display().to_string()
}

fn create_folder(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| format!("Could not create '{}': {error}", path.display()))
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|error| format!("Could not write '{}': {error}", path.display()))
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!("Could not copy '{}' to '{}': {error}", source.display(), destination.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_editor_versions_from_hub_folders() {
        let editor = Path::new("C:/Program Files/Unity/Hub/Editor/2019.4.40f1/Editor/Unity.exe");
        assert_eq!(editor_version(editor).as_deref(), Some("2019.4.40f1"));
        assert!(is_compatible_version("2019.4.41f2"));
        assert!(!is_compatible_version("2022.3.1f1"));
    }

    #[test]
    fn recognises_licence_failures() {
        let log = "[Licensing::Module] Successfully launched\n\
                   BatchMode: Unity has not been activated with a valid License. Could be a new activation";
        assert!(is_license_failure(log));
        assert!(!is_license_failure("Error: Asset not found"));
    }

    #[test]
    fn explains_failures_from_the_log() {
        let log = "Loading\nSomething fine\nError: No valid Unity Editor license found\nQuitting\n";
        assert_eq!(log_errors(log), "Error: No valid Unity Editor license found");
        let quiet = "a\nb\nc";
        assert_eq!(log_errors(quiet), "a\nb\nc");
    }
}
