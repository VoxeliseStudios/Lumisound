//! Presets: themes, sound effects and visualizer styles. Replaces the old Lua
//! scripts: every preset is a JSON definition the core resolves. The bundled
//! ones live in this folder (themes.json, effects.json, visualizers.json);
//! people share their own as JSON files, which the app keeps under
//! `Documents/Presets/{Themes,Effects,Visualizers}`.
//!
//! A definition can compute values instead of listing them:
//! - themes: any colour may be `{"shade_of": "background", "amount": 0.18}`
//!   (lighten toward white for > 0, darken toward black for < 0);
//! - effects: `eq_curve` shapes (smiley, notch, bass, tape, blend),
//!   `genre_curves` (EQ picked from the playing track's genre) and a tremolo
//!   that can follow the track's tempo (`pulses_per_beat`);
//! - visualizers: `hsl_ramp`, `gradient` or `alternate` colour generators.

use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// MARK: - Public API

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetKind {
    Theme,
    Effect,
    Visualizer,
}

/// One row of a preset picker.
#[derive(uniffi::Record, Clone, Debug)]
pub struct PresetEntry {
    /// Bundled: the preset's id (`forest_calm`). Imported: `user:<file stem>`.
    pub id: String,
    pub name: String,
    pub subtitle: String,
    /// SF Symbol name.
    pub icon: String,
    pub bundled: bool,
    /// Preview colours: theme = [background, accent], visualizer = its bar
    /// colours, effect = none.
    pub swatch: Vec<String>,
    /// Set when an imported file can't be read any more (the row still shows
    /// so it can be deleted).
    pub error: Option<String>,
}

/// What an effect can react to about the playing track.
#[derive(uniffi::Record, Clone, Debug, Default)]
pub struct PresetTrack {
    pub genre: String,
    pub bpm: Option<f64>,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum PresetSpecialMode {
    Rotation { hz: f64 },
    Tremolo { freq: f64, depth: f64 },
    Vibrato { freq: f64, pitch_depth: f64 },
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct PresetEffect {
    pub id: String,
    pub name: String,
    pub icon: String,
    /// Always 10 bands, dB.
    pub eq_bands: Vec<f32>,
    pub eq_enabled: bool,
    pub speed: f32,
    pub pitch_semitones: f32,
    pub special: Option<PresetSpecialMode>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct PresetVisualizer {
    pub id: String,
    pub name: String,
    pub colors: Vec<String>,
    pub sensitivity: f64,
    pub bar_corner_radius: f64,
    pub bar_spacing: f64,
    pub mirrored: bool,
}

/// A resolved theme. Colours are always set; every other field is `None`
/// when the preset leaves that setting alone.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct PresetTheme {
    pub id: String,
    pub name: String,
    pub description: String,
    pub accent: String,
    pub accent_secondary: String,
    pub text_primary: String,
    pub text_secondary: String,
    pub background: String,
    pub surface: String,
    pub elevated_surface: String,
    pub font_style: Option<String>,
    pub panel_material: Option<String>,
    pub launch_screen_style: Option<String>,
    pub tab_transition_style: Option<String>,
    pub artwork_style: Option<String>,
    pub seeker_style: Option<String>,
    pub card_style: Option<String>,
    pub panel_opacity: Option<f64>,
    pub corner_radius_scale: Option<f64>,
    pub spacing_scale: Option<f64>,
    pub glass_tint_strength: Option<f64>,
    pub glass_tint_hue: Option<f64>,
    pub glass_use_accent_tint: Option<bool>,
    pub glass_translucency: Option<f64>,
    pub reduce_motion: Option<bool>,
    pub show_bpm_badges: Option<bool>,
    pub aggressive_prefetch: Option<bool>,
    pub haptic_feedback: Option<bool>,
    pub library_default_sort: Option<String>,
    pub pin_favorites_first: Option<bool>,
    pub library_default_columns: Option<i32>,
    pub layout_default_panel: Option<String>,
    pub layout_default_display_mode: Option<String>,
    pub layout_show_queue_preview: Option<bool>,
    pub layout_eq_expanded: Option<bool>,
}

/// Bundled presets first (in their curated order), then imported ones by file name.
#[uniffi::export]
pub fn presets_list(kind: PresetKind, documents_dir: String) -> Vec<PresetEntry> {
    let mut out: Vec<PresetEntry> = bundled(kind).iter().map(|(id, def)| entry_for(kind, id, def, true)).collect();
    for (stem, path) in user_files(kind, Path::new(&documents_dir)) {
        let id = format!("{USER_PREFIX}{stem}");
        match read_user_def(&path) {
            Ok(def) => out.push(entry_for(kind, &id, &def, false)),
            Err(e) => out.push(PresetEntry {
                id,
                name: display_name(&stem),
                subtitle: String::new(),
                icon: "exclamationmark.triangle".into(),
                bundled: false,
                swatch: vec![],
                error: Some(e.to_string()),
            }),
        }
    }
    out
}

/// Checks a shared preset (JSON text) and saves it under the user's presets.
/// The saved file is the definition itself (pretty-printed), so genre curves
/// and tempo sync keep working after import.
#[uniffi::export]
pub fn presets_import(kind: PresetKind, documents_dir: String, source: String, suggested_name: String) -> CoreResult<PresetEntry> {
    let mut def: Value = serde_json::from_str(source.trim()).map_err(|e| {
        if looks_like_lua(&source) {
            CoreError::invalid("Lua scripts aren't supported any more. Presets are JSON now: open a built-in preset and use Share to see the format.")
        } else {
            CoreError::invalid(format!("Not a valid preset (JSON): {e}"))
        }
    })?;
    if !def.is_object() {
        return Err(CoreError::invalid("A preset must be a JSON object."));
    }
    // Name: the definition's own, else the suggested one.
    let fallback = suggested_name.trim();
    let name_ptr = if kind == PresetKind::Theme { "/meta/name" } else { "/name" };
    let has_name = def.pointer(name_ptr).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
    if !has_name {
        let n = if fallback.is_empty() { "Custom preset" } else { fallback };
        if kind == PresetKind::Theme {
            let meta = def.as_object_mut().unwrap().entry("meta").or_insert_with(|| Value::Object(Default::default()));
            if let Some(m) = meta.as_object_mut() {
                m.insert("name".into(), Value::String(n.into()));
            }
        } else {
            def.as_object_mut().unwrap().insert("name".into(), Value::String(n.into()));
        }
    }
    // Validate by resolving it exactly as it would be applied.
    validate(kind, &def)?;

    let dir = user_dir(kind, Path::new(&documents_dir));
    std::fs::create_dir_all(&dir).map_err(io)?;
    let base = {
        let raw = if fallback.is_empty() { def_name(kind, &def) } else { fallback.to_string() };
        let s = sanitize_stem(&raw);
        if s.is_empty() { "custom".to_string() } else { s }
    };
    let mut stem = base.clone();
    let mut n = 1;
    while dir.join(format!("{stem}.json")).exists() {
        stem = format!("{base}_{n}");
        n += 1;
    }
    let path = dir.join(format!("{stem}.json"));
    let text = serde_json::to_string_pretty(&def).map_err(|e| CoreError::invalid(e.to_string()))?;
    write_atomic(&path, text.as_bytes())?;
    Ok(entry_for(kind, &format!("{USER_PREFIX}{stem}"), &def, false))
}

/// Removes an imported preset. Bundled ones can't be deleted.
#[uniffi::export]
pub fn presets_delete(kind: PresetKind, documents_dir: String, id: String) -> CoreResult<()> {
    let Some(stem) = id.strip_prefix(USER_PREFIX) else {
        return Err(CoreError::invalid("Built-in presets can't be deleted."));
    };
    let path = user_dir(kind, Path::new(&documents_dir)).join(format!("{}.json", sanitize_stem(stem)));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
    }
}

/// The definition as shareable JSON (bundled presets too, as a template).
#[uniffi::export]
pub fn presets_source(kind: PresetKind, documents_dir: String, id: String) -> CoreResult<String> {
    let def = load_def(kind, Path::new(&documents_dir), &id)?;
    serde_json::to_string_pretty(&def).map_err(|e| CoreError::invalid(e.to_string()))
}

#[uniffi::export]
pub fn presets_theme(documents_dir: String, id: String) -> CoreResult<PresetTheme> {
    let def = load_def(PresetKind::Theme, Path::new(&documents_dir), &id)?;
    let mut t = resolve_theme(&def)?;
    t.meta.id = canonical_id(PresetKind::Theme, &id);
    Ok(t.into_record())
}

#[uniffi::export]
pub fn presets_effect(documents_dir: String, id: String, track: Option<PresetTrack>) -> CoreResult<PresetEffect> {
    let def = load_def(PresetKind::Effect, Path::new(&documents_dir), &id)?;
    let mut e = resolve_effect(&def, track.as_ref())?;
    e.id = canonical_id(PresetKind::Effect, &id);
    Ok(e.into_record())
}

#[uniffi::export]
pub fn presets_visualizer(documents_dir: String, id: String) -> CoreResult<PresetVisualizer> {
    let def = load_def(PresetKind::Visualizer, Path::new(&documents_dir), &id)?;
    let mut v = resolve_visualizer(&def)?;
    v.id = canonical_id(PresetKind::Visualizer, &id);
    Ok(v)
}

/// Maps ids saved by older app versions (the Lua theme enum's camelCase
/// names, `lua:`-prefixed effect ids) to today's ids.
#[uniffi::export]
pub fn presets_canonical_id(kind: PresetKind, id: String) -> String {
    canonical_id(kind, &id)
}

// MARK: - Storage

const USER_PREFIX: &str = "user:";

fn subdir(kind: PresetKind) -> &'static str {
    match kind {
        PresetKind::Theme => "Themes",
        PresetKind::Effect => "Effects",
        PresetKind::Visualizer => "Visualizers",
    }
}

fn user_dir(kind: PresetKind, documents: &Path) -> PathBuf {
    documents.join("Presets").join(subdir(kind))
}

fn user_files(kind: PresetKind, documents: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(user_dir(kind, documents)) else { return vec![] };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p)))
        .collect();
    v.sort();
    v
}

fn read_user_def(path: &Path) -> CoreResult<Value> {
    let text = std::fs::read_to_string(path).map_err(io)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| CoreError::invalid(format!("Not valid JSON: {e}")))?;
    if !v.is_object() {
        return Err(CoreError::invalid("A preset must be a JSON object."));
    }
    Ok(v)
}

fn load_def(kind: PresetKind, documents: &Path, id: &str) -> CoreResult<Value> {
    let id = canonical_id(kind, id);
    if let Some(stem) = id.strip_prefix(USER_PREFIX) {
        return read_user_def(&user_dir(kind, documents).join(format!("{}.json", sanitize_stem(stem))));
    }
    bundled(kind)
        .iter()
        .find(|(bid, _)| *bid == id)
        .map(|(_, d)| d.clone())
        .ok_or_else(|| CoreError::invalid(format!("No preset named “{id}”.")))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

fn io(e: std::io::Error) -> CoreError {
    CoreError::invalid(e.to_string())
}

fn sanitize_stem(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').chars().take(60).collect()
}

fn display_name(stem: &str) -> String {
    stem.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_like_lua(s: &str) -> bool {
    s.contains("local ") || s.contains("function") || s.contains("= {") || s.contains("--")
}

fn canonical_id(kind: PresetKind, id: &str) -> String {
    let id = id.strip_prefix("lua:").unwrap_or(id);
    if id.starts_with(USER_PREFIX) {
        return id.to_string();
    }
    if kind == PresetKind::Theme {
        // The old Swift `LuaPreset` raw values.
        let legacy = match id {
            "highContrast" => "high_contrast",
            "vibrantEnergetic" => "vibrant_energetic",
            "amoledDark" => "amoled_dark",
            "retroSkeuo" => "retro_skeuomorphic",
            "pastelDream" => "pastel_dream",
            "forestCalm" => "forest_calm",
            "cyberpunkNeon" => "cyberpunk_neon",
            "monochromeEditorial" => "monochrome_editorial",
            "synthwaveSunset" => "synthwave_sunset",
            other => other,
        };
        return legacy.to_string();
    }
    id.to_string()
}

fn bundled(kind: PresetKind) -> &'static [(String, Value)] {
    static THEMES: OnceLock<Vec<(String, Value)>> = OnceLock::new();
    static EFFECTS: OnceLock<Vec<(String, Value)>> = OnceLock::new();
    static VISUALIZERS: OnceLock<Vec<(String, Value)>> = OnceLock::new();
    let (cell, src, ptr) = match kind {
        PresetKind::Theme => (&THEMES, include_str!("themes.json"), "/meta/id"),
        PresetKind::Effect => (&EFFECTS, include_str!("effects.json"), "/id"),
        PresetKind::Visualizer => (&VISUALIZERS, include_str!("visualizers.json"), "/id"),
    };
    cell.get_or_init(|| {
        let list: Vec<Value> = serde_json::from_str(src).expect("bundled presets are valid JSON");
        list.into_iter()
            .map(|v| (v.pointer(ptr).and_then(Value::as_str).unwrap_or_default().to_string(), v))
            .collect()
    })
}

fn def_name(kind: PresetKind, def: &Value) -> String {
    let p = if kind == PresetKind::Theme { "/meta/name" } else { "/name" };
    def.pointer(p).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn entry_for(kind: PresetKind, id: &str, def: &Value, is_bundled: bool) -> PresetEntry {
    let meta = if kind == PresetKind::Theme { def.get("meta").cloned().unwrap_or(Value::Null) } else { def.clone() };
    let s = |k: &str| meta.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut subtitle = s("subtitle");
    if subtitle.is_empty() {
        subtitle = s("description");
    }
    let mut icon = s("icon");
    let (swatch, error) = match kind {
        PresetKind::Theme => match resolve_theme(def) {
            Ok(t) => (vec![t.colors.background, t.colors.accent], None),
            Err(e) => (vec![], Some(e.to_string())),
        },
        PresetKind::Visualizer => match resolve_visualizer(def) {
            Ok(v) => (v.colors, None),
            Err(e) => (vec![], Some(e.to_string())),
        },
        PresetKind::Effect => (vec![], resolve_effect(def, None).err().map(|e| e.to_string())),
    };
    if icon.is_empty() {
        icon = match kind {
            PresetKind::Theme => "paintpalette",
            PresetKind::Effect => "slider.horizontal.3",
            PresetKind::Visualizer => "waveform.path.ecg.rectangle",
        }
        .into();
    }
    let mut name = s("name");
    if name.is_empty() {
        name = display_name(id.strip_prefix(USER_PREFIX).unwrap_or(id));
    }
    PresetEntry { id: id.to_string(), name, subtitle, icon, bundled: is_bundled, swatch, error }
}

fn validate(kind: PresetKind, def: &Value) -> CoreResult<()> {
    match kind {
        PresetKind::Theme => resolve_theme(def).map(|_| ()),
        PresetKind::Effect => resolve_effect(def, None).map(|_| ()),
        PresetKind::Visualizer => resolve_visualizer(def).map(|_| ()),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(def: &Value) -> CoreResult<T> {
    T::deserialize(def).map_err(|e| CoreError::invalid(format!("Preset: {e}")))
}

// MARK: - Colour helpers

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn norm_hex(s: &str, field: &str) -> CoreResult<String> {
    parse_hex(s).map(hex).ok_or_else(|| CoreError::invalid(format!("{field}: “{s}” is not a colour like #RRGGBB")))
}

/// Lighten toward white (amount > 0) or darken toward black (amount < 0).
pub fn shade(color: &str, amount: f64) -> Option<String> {
    let (r, g, b) = parse_hex(color)?;
    let adj = |c: u8| {
        let c = c as f64;
        let v = if amount >= 0.0 { c + (255.0 - c) * amount } else { c + c * amount };
        (v + 0.5).floor().clamp(0.0, 255.0) as u8
    };
    Some(hex((adj(r), adj(g), adj(b))))
}

fn hsl_to_hex(h: f64, s: f64, l: f64) -> String {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let to255 = |v: f64| ((v + m) * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
    hex((to255(r), to255(g), to255(b)))
}

fn mix_hex(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> String {
    let l = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t + 0.5).floor().clamp(0.0, 255.0) as u8;
    hex((l(a.0, b.0), l(a.1, b.1), l(a.2, b.2)))
}

fn relative_luminance(c: (u8, u8, u8)) -> f64 {
    let ch = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c.0) + 0.7152 * ch(c.1) + 0.0722 * ch(c.2)
}

// MARK: - Themes

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
#[serde(default)]
struct ThemeMeta {
    id: String,
    name: String,
    description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    subtitle: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    icon: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
enum ColorRef {
    Hex(String),
    Shade { shade_of: String, amount: f64 },
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ThemeColorsDef {
    background: Option<ColorRef>,
    surface: Option<ColorRef>,
    elevated_surface: Option<ColorRef>,
    accent: Option<ColorRef>,
    accent_secondary: Option<ColorRef>,
    text_primary: Option<ColorRef>,
    text_secondary: Option<ColorRef>,
}

#[derive(Serialize, Clone, Debug)]
struct ThemeColors {
    accent: String,
    accent_secondary: String,
    text_primary: String,
    text_secondary: String,
    background: String,
    surface: String,
    elevated_surface: String,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
struct ThemeStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    font_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    panel_material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_screen_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tab_transition_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artwork_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seeker_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    card_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    panel_opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    corner_radius_scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spacing_scale: Option<f64>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
struct ThemeGlass {
    #[serde(skip_serializing_if = "Option::is_none")]
    tint_strength: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tint_hue: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    use_accent_tint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    translucency: Option<f64>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
struct ThemeFlags {
    #[serde(skip_serializing_if = "Option::is_none")]
    reduce_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_bpm_badges: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aggressive_prefetch: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    haptic_feedback: Option<bool>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
struct ThemeHooks {
    #[serde(skip_serializing_if = "Option::is_none")]
    library_default_sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pin_favorites_first: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    library_default_columns: Option<i32>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
struct ThemeLayout {
    #[serde(skip_serializing_if = "Option::is_none")]
    default_panel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_display_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_queue_preview: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eq_expanded_by_default: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ThemeDef {
    meta: ThemeMeta,
    colors: ThemeColorsDef,
    style: ThemeStyle,
    glass: ThemeGlass,
    flags: ThemeFlags,
    hooks: ThemeHooks,
    layout: Option<ThemeLayout>,
}

#[derive(Serialize, Clone, Debug)]
struct ResolvedTheme {
    meta: ThemeMeta,
    colors: ThemeColors,
    style: ThemeStyle,
    glass: ThemeGlass,
    flags: ThemeFlags,
    hooks: ThemeHooks,
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<ThemeLayout>,
}

const SORTS: [&str; 5] = ["title", "artist", "dateAdded", "playCount", "duration"];
const PANELS: [&str; 5] = ["Controls", "Sound", "Queue", "Lyrics", "Marks"];
const DISPLAY_MODES: [&str; 2] = ["artwork", "lyrics"];

fn one_of(v: &Option<String>, allowed: &[&str], field: &str) -> CoreResult<()> {
    match v {
        Some(s) if !allowed.contains(&s.as_str()) => {
            Err(CoreError::invalid(format!("{field}: “{s}” isn't one of {}", allowed.join(", "))))
        }
        _ => Ok(()),
    }
}

fn clamp_opt(v: &mut Option<f64>, lo: f64, hi: f64) {
    if let Some(x) = v {
        *x = if x.is_finite() { x.clamp(lo, hi) } else { lo };
    }
}

fn resolve_theme(def: &Value) -> CoreResult<ResolvedTheme> {
    let d: ThemeDef = parse(def)?;
    let c = &d.colors;
    let mut named: Vec<(&str, String)> = Vec::with_capacity(7);
    let resolve = |name: &str, r: &ColorRef, named: &[(&str, String)]| -> CoreResult<String> {
        match r {
            ColorRef::Hex(h) => norm_hex(h, &format!("colors.{name}")),
            ColorRef::Shade { shade_of, amount } => {
                let base = named.iter().find(|(n, _)| *n == shade_of.as_str()).map(|(_, v)| v).ok_or_else(|| {
                    CoreError::invalid(format!("colors.{name}: shade_of “{shade_of}” must name a colour defined above it"))
                })?;
                if !amount.is_finite() || !(-1.0..=1.0).contains(amount) {
                    return Err(CoreError::invalid(format!("colors.{name}: amount must be between -1 and 1")));
                }
                Ok(shade(base, *amount).expect("named colours are valid"))
            }
        }
    };
    let background = match &c.background {
        Some(ColorRef::Hex(h)) => norm_hex(h, "colors.background")?,
        Some(_) => return Err(CoreError::invalid("colors.background must be a colour like #RRGGBB")),
        None => return Err(CoreError::invalid("colors.background is required")),
    };
    let dark = relative_luminance(parse_hex(&background).unwrap()) < 0.4;
    named.push(("background", background.clone()));
    let default_shade = |amount: f64| ColorRef::Shade { shade_of: "background".into(), amount };
    let surface = resolve("surface", c.surface.as_ref().unwrap_or(&default_shade(if dark { 0.10 } else { -0.04 })), &named)?;
    named.push(("surface", surface.clone()));
    let elevated = resolve("elevated_surface", c.elevated_surface.as_ref().unwrap_or(&default_shade(if dark { 0.22 } else { -0.08 })), &named)?;
    named.push(("elevated_surface", elevated.clone()));
    let accent = resolve("accent", c.accent.as_ref().ok_or_else(|| CoreError::invalid("colors.accent is required"))?, &named)?;
    named.push(("accent", accent.clone()));
    let accent_secondary = match &c.accent_secondary {
        Some(r) => resolve("accent_secondary", r, &named)?,
        None => accent.clone(),
    };
    named.push(("accent_secondary", accent_secondary.clone()));
    let text_primary = match &c.text_primary {
        Some(r) => resolve("text_primary", r, &named)?,
        None => (if dark { "#FFFFFF" } else { "#1C1C1E" }).to_string(),
    };
    named.push(("text_primary", text_primary.clone()));
    let text_secondary = match &c.text_secondary {
        Some(r) => resolve("text_secondary", r, &named)?,
        None => (if dark { "#A0A0A8" } else { "#6E6E73" }).to_string(),
    };

    let mut style = d.style.clone();
    clamp_opt(&mut style.panel_opacity, 0.0, 1.0);
    clamp_opt(&mut style.corner_radius_scale, 0.25, 3.0);
    clamp_opt(&mut style.spacing_scale, 0.5, 2.0);
    let mut glass = d.glass.clone();
    clamp_opt(&mut glass.tint_strength, 0.0, 1.0);
    clamp_opt(&mut glass.tint_hue, 0.0, 1.0);
    clamp_opt(&mut glass.translucency, 0.0, 1.0);
    let mut hooks = d.hooks.clone();
    one_of(&hooks.library_default_sort, &SORTS, "hooks.library_default_sort")?;
    if let Some(c) = hooks.library_default_columns.as_mut() {
        *c = (*c).clamp(1, 3);
    }
    if let Some(l) = &d.layout {
        one_of(&l.default_panel, &PANELS, "layout.default_panel")?;
        one_of(&l.default_display_mode, &DISPLAY_MODES, "layout.default_display_mode")?;
    }
    let mut meta = d.meta.clone();
    if meta.name.trim().is_empty() {
        return Err(CoreError::invalid("meta.name is required"));
    }
    meta.name = meta.name.trim().chars().take(60).collect();
    Ok(ResolvedTheme {
        meta,
        colors: ThemeColors { accent, accent_secondary, text_primary, text_secondary, background, surface, elevated_surface: elevated },
        style,
        glass,
        flags: d.flags.clone(),
        hooks,
        layout: d.layout.clone(),
    })
}

impl ResolvedTheme {
    fn into_record(self) -> PresetTheme {
        let l = self.layout.unwrap_or_default();
        PresetTheme {
            id: self.meta.id,
            name: self.meta.name,
            description: self.meta.description,
            accent: self.colors.accent,
            accent_secondary: self.colors.accent_secondary,
            text_primary: self.colors.text_primary,
            text_secondary: self.colors.text_secondary,
            background: self.colors.background,
            surface: self.colors.surface,
            elevated_surface: self.colors.elevated_surface,
            font_style: self.style.font_style,
            panel_material: self.style.panel_material,
            launch_screen_style: self.style.launch_screen_style,
            tab_transition_style: self.style.tab_transition_style,
            artwork_style: self.style.artwork_style,
            seeker_style: self.style.seeker_style,
            card_style: self.style.card_style,
            panel_opacity: self.style.panel_opacity,
            corner_radius_scale: self.style.corner_radius_scale,
            spacing_scale: self.style.spacing_scale,
            glass_tint_strength: self.glass.tint_strength,
            glass_tint_hue: self.glass.tint_hue,
            glass_use_accent_tint: self.glass.use_accent_tint,
            glass_translucency: self.glass.translucency,
            reduce_motion: self.flags.reduce_motion,
            show_bpm_badges: self.flags.show_bpm_badges,
            aggressive_prefetch: self.flags.aggressive_prefetch,
            haptic_feedback: self.flags.haptic_feedback,
            library_default_sort: self.hooks.library_default_sort,
            pin_favorites_first: self.hooks.pin_favorites_first,
            library_default_columns: self.hooks.library_default_columns,
            layout_default_panel: l.default_panel,
            layout_default_display_mode: l.default_display_mode,
            layout_show_queue_preview: l.show_queue_preview,
            layout_eq_expanded: l.eq_expanded_by_default,
        }
    }
}

// MARK: - Effects

#[derive(Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
enum EqCurve {
    /// Cosine V: lifted ends, scooped middle.
    Smiley { depth: f64, #[serde(default = "one")] width: f64 },
    /// Raised-cosine cut centred on a band (1-10).
    Notch { center: f64, depth: f64, #[serde(default = "one")] width: f64 },
    /// Sub-bass boost that decays per band, small low-mid scoop.
    Bass { intensity: f64, #[serde(default = "bass_decay")] decay: f64 },
    /// Worn tape: warm lows, rolled-off highs (age 0-1).
    Tape { age: f64 },
    /// Linear mix of two 10-band curves.
    Blend { from: Vec<f64>, to: Vec<f64>, amount: f64 },
}

fn one() -> f64 {
    1.0
}
fn bass_decay() -> f64 {
    0.55
}

#[derive(Deserialize)]
struct GenreCurve {
    keywords: Vec<String>,
    eq_bands: Vec<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SpecialDef {
    #[serde(rename = "type")]
    kind: String,
    hz: Option<f64>,
    freq: Option<f64>,
    depth: Option<f64>,
    pitch_depth: Option<f64>,
    /// Tremolo only: pulse at the track's tempo when it's known.
    pulses_per_beat: Option<f64>,
}

#[derive(Deserialize)]
struct EffectDef {
    #[serde(default)]
    name: String,
    #[serde(default)]
    icon: String,
    eq_bands: Option<Vec<f64>>,
    eq_curve: Option<EqCurve>,
    #[serde(default = "yes")]
    eq_enabled: bool,
    #[serde(default = "one")]
    speed: f64,
    #[serde(default)]
    pitch_semitones: f64,
    special_mode: Option<SpecialDef>,
    #[serde(default)]
    genre_curves: Vec<GenreCurve>,
}

fn yes() -> bool {
    true
}

struct ResolvedEffect {
    id: String,
    name: String,
    icon: String,
    eq_bands: Vec<f64>,
    eq_enabled: bool,
    speed: f64,
    pitch_semitones: f64,
    special: Option<PresetSpecialMode>,
}

impl ResolvedEffect {
    fn into_record(self) -> PresetEffect {
        PresetEffect {
            id: self.id,
            name: self.name,
            icon: self.icon,
            eq_bands: self.eq_bands.iter().map(|&b| b as f32).collect(),
            eq_enabled: self.eq_enabled,
            speed: self.speed as f32,
            pitch_semitones: self.pitch_semitones as f32,
            special: self.special,
        }
    }
}

const BANDS: usize = 10;

fn curve_bands(c: &EqCurve) -> CoreResult<Vec<f64>> {
    let n = BANDS as f64;
    let v: Vec<f64> = match c {
        EqCurve::Smiley { depth, width } => {
            let center = (n + 1.0) / 2.0;
            (1..=BANDS)
                .map(|i| {
                    let d = ((i as f64 - center).abs() / (center - 1.0) * width).min(1.0);
                    -depth * (d * std::f64::consts::PI).cos()
                })
                .collect()
        }
        EqCurve::Notch { center, depth, width } => {
            if *width <= 0.0 {
                return Err(CoreError::invalid("eq_curve.width must be above 0"));
            }
            (1..=BANDS)
                .map(|i| {
                    let d = ((i as f64 - center).abs() / 2.0 / width).min(1.0);
                    -depth * (0.5 + 0.5 * (d * std::f64::consts::PI).cos())
                })
                .collect()
        }
        EqCurve::Bass { intensity, decay } => {
            let peak = 10.0 * intensity;
            (1..=BANDS)
                .map(|i| {
                    if i <= 3 {
                        peak * decay.powf((i - 1) as f64)
                    } else if i <= 6 {
                        -1.5 * intensity * (1.0 - (i - 3) as f64 / 4.0)
                    } else {
                        0.0
                    }
                })
                .collect()
        }
        EqCurve::Tape { age } => (1..=BANDS)
            .map(|i| {
                if i <= 4 {
                    2.5 * age * (1.0 - (i - 1) as f64 / 4.0)
                } else if i >= 8 {
                    -3.0 * age
                } else {
                    0.0
                }
            })
            .collect(),
        EqCurve::Blend { from, to, amount } => {
            check_bands(from, "eq_curve.from")?;
            check_bands(to, "eq_curve.to")?;
            from.iter().zip(to).map(|(a, b)| a + (b - a) * amount).collect()
        }
    };
    Ok(v)
}

fn check_bands(b: &[f64], field: &str) -> CoreResult<()> {
    if b.len() != BANDS {
        return Err(CoreError::invalid(format!("{field} has {} bands; it needs exactly {BANDS}", b.len())));
    }
    if b.iter().any(|x| !x.is_finite()) {
        return Err(CoreError::invalid(format!("{field} has a value that isn't a number")));
    }
    Ok(())
}

fn resolve_effect(def: &Value, track: Option<&PresetTrack>) -> CoreResult<ResolvedEffect> {
    let d: EffectDef = parse(def)?;
    if d.name.trim().is_empty() {
        return Err(CoreError::invalid("name is required"));
    }
    let base = match (&d.eq_bands, &d.eq_curve) {
        (Some(b), _) => {
            check_bands(b, "eq_bands")?;
            b.clone()
        }
        (None, Some(c)) => curve_bands(c)?,
        (None, None) => vec![0.0; BANDS],
    };
    for (i, g) in d.genre_curves.iter().enumerate() {
        check_bands(&g.eq_bands, &format!("genre_curves[{i}].eq_bands"))?;
        if g.keywords.iter().all(|k| k.trim().is_empty()) {
            return Err(CoreError::invalid(format!("genre_curves[{i}] needs at least one keyword")));
        }
    }
    let genre = track.map(|t| t.genre.to_lowercase()).unwrap_or_default();
    let picked = if genre.is_empty() {
        None
    } else {
        d.genre_curves
            .iter()
            .find(|g| g.keywords.iter().any(|k| !k.trim().is_empty() && genre.contains(&k.trim().to_lowercase())))
    };
    let eq_bands: Vec<f64> = picked.map(|g| g.eq_bands.clone()).unwrap_or(base).into_iter().map(|b| b.clamp(-24.0, 24.0)).collect();

    let special = match &d.special_mode {
        None => None,
        Some(s) => {
            let bpm = track.and_then(|t| t.bpm).filter(|b| b.is_finite() && *b > 20.0 && *b < 400.0);
            Some(match s.kind.as_str() {
                "rotation" => PresetSpecialMode::Rotation { hz: s.hz.unwrap_or(0.18).clamp(0.01, 5.0) },
                "tremolo" => {
                    let synced = s.pulses_per_beat.zip(bpm).map(|(p, b)| b / 60.0 * p);
                    PresetSpecialMode::Tremolo {
                        freq: synced.or(s.freq).unwrap_or(4.0).clamp(0.05, 20.0),
                        depth: s.depth.unwrap_or(0.45).clamp(0.0, 1.0),
                    }
                }
                "vibrato" => PresetSpecialMode::Vibrato {
                    freq: s.freq.unwrap_or(4.5).clamp(0.05, 20.0),
                    pitch_depth: s.pitch_depth.unwrap_or(0.35).clamp(0.0, 2.0),
                },
                other => {
                    return Err(CoreError::invalid(format!(
                        "special_mode.type “{other}” isn't one of rotation, tremolo, vibrato"
                    )))
                }
            })
        }
    };
    if !d.speed.is_finite() || !d.pitch_semitones.is_finite() {
        return Err(CoreError::invalid("speed and pitch_semitones must be numbers"));
    }
    Ok(ResolvedEffect {
        id: String::new(),
        name: d.name.trim().chars().take(60).collect(),
        icon: if d.icon.is_empty() { "slider.horizontal.3".into() } else { d.icon },
        eq_bands,
        eq_enabled: d.eq_enabled,
        speed: d.speed.clamp(0.25, 4.0),
        pitch_semitones: d.pitch_semitones.clamp(-24.0, 24.0),
        special,
    })
}

// MARK: - Visualizers

#[derive(Deserialize)]
struct HslRamp {
    hue_from: f64,
    hue_to: f64,
    saturation: f64,
    lightness_from: f64,
    lightness_to: f64,
    stops: usize,
}

#[derive(Deserialize)]
struct Gradient {
    from: String,
    to: String,
    stops: usize,
}

#[derive(Deserialize)]
struct Alternate {
    colors: Vec<String>,
    stops: usize,
}

#[derive(Deserialize)]
struct VisualizerDef {
    #[serde(default)]
    name: String,
    colors: Option<Vec<String>>,
    hsl_ramp: Option<HslRamp>,
    gradient: Option<Gradient>,
    alternate: Option<Alternate>,
    #[serde(default = "one")]
    sensitivity: f64,
    #[serde(default = "two")]
    bar_corner_radius: f64,
    #[serde(default = "two")]
    bar_spacing: f64,
    #[serde(default)]
    mirrored: bool,
}

fn two() -> f64 {
    2.0
}

const MAX_STOPS: usize = 16;

fn check_stops(n: usize, field: &str) -> CoreResult<()> {
    if (1..=MAX_STOPS).contains(&n) {
        Ok(())
    } else {
        Err(CoreError::invalid(format!("{field} must be 1-{MAX_STOPS}")))
    }
}

fn ramp_t(i: usize, stops: usize) -> f64 {
    if stops <= 1 { 0.0 } else { i as f64 / (stops - 1) as f64 }
}

fn resolve_visualizer(def: &Value) -> CoreResult<PresetVisualizer> {
    let d: VisualizerDef = parse(def)?;
    let colors: Vec<String> = if let Some(c) = &d.colors {
        check_stops(c.len(), "colors (count)")?;
        c.iter().enumerate().map(|(i, h)| norm_hex(h, &format!("colors[{i}]"))).collect::<CoreResult<_>>()?
    } else if let Some(r) = &d.hsl_ramp {
        check_stops(r.stops, "hsl_ramp.stops")?;
        let s = r.saturation.clamp(0.0, 1.0);
        (0..r.stops)
            .map(|i| {
                let t = ramp_t(i, r.stops);
                let hue = (r.hue_from + (r.hue_to - r.hue_from) * t).rem_euclid(360.0);
                let l = (r.lightness_from + (r.lightness_to - r.lightness_from) * t).clamp(0.0, 1.0);
                hsl_to_hex(hue, s, l)
            })
            .collect()
    } else if let Some(g) = &d.gradient {
        check_stops(g.stops, "gradient.stops")?;
        let a = parse_hex(&g.from).ok_or_else(|| CoreError::invalid("gradient.from is not a colour like #RRGGBB"))?;
        let b = parse_hex(&g.to).ok_or_else(|| CoreError::invalid("gradient.to is not a colour like #RRGGBB"))?;
        (0..g.stops).map(|i| mix_hex(a, b, ramp_t(i, g.stops))).collect()
    } else if let Some(a) = &d.alternate {
        check_stops(a.stops, "alternate.stops")?;
        if a.colors.is_empty() {
            return Err(CoreError::invalid("alternate.colors needs at least one colour"));
        }
        let cs: Vec<String> = a.colors.iter().enumerate().map(|(i, h)| norm_hex(h, &format!("alternate.colors[{i}]"))).collect::<CoreResult<_>>()?;
        (0..a.stops).map(|i| cs[i % cs.len()].clone()).collect()
    } else {
        return Err(CoreError::invalid("A visualizer needs colors, hsl_ramp, gradient or alternate"));
    };
    let f = |v: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { lo };
    Ok(PresetVisualizer {
        id: String::new(),
        name: d.name.trim().to_string(),
        colors,
        sensitivity: f(d.sensitivity, 0.1, 4.0),
        bar_corner_radius: f(d.bar_corner_radius, 0.0, 20.0),
        bar_spacing: f(d.bar_spacing, 0.0, 20.0),
        mirrored: d.mirrored,
    })
}

// MARK: - Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(name: &str) -> Value {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/presets").join(name);
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
    }

    /// Compare JSON with a float tolerance (the Lua output went through
    /// `%.17g`; the Rust maths is the same formulas but may differ by an ulp).
    fn same(a: &Value, b: &Value, path: &str) {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
                assert!((x - y).abs() < 1e-9, "{path}: {x} != {y}");
            }
            (Value::Object(x), Value::Object(y)) => {
                let mut kx: Vec<_> = x.keys().collect();
                let mut ky: Vec<_> = y.keys().collect();
                kx.sort();
                ky.sort();
                assert_eq!(kx, ky, "{path}: keys");
                for k in kx {
                    same(&x[k], &y[k], &format!("{path}.{k}"));
                }
            }
            (Value::Array(x), Value::Array(y)) => {
                assert_eq!(x.len(), y.len(), "{path}: length");
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    same(p, q, &format!("{path}[{i}]"));
                }
            }
            _ => assert_eq!(a, b, "{path}"),
        }
    }

    #[test]
    fn themes_match_the_old_lua_output() {
        assert_eq!(bundled(PresetKind::Theme).len(), 14);
        for (id, def) in bundled(PresetKind::Theme) {
            let mut got = serde_json::to_value(resolve_theme(def).unwrap()).unwrap();
            let meta = got["meta"].as_object_mut().unwrap();
            assert!(!meta.remove("subtitle").unwrap().as_str().unwrap().is_empty(), "{id} subtitle");
            assert!(!meta.remove("icon").unwrap().as_str().unwrap().is_empty(), "{id} icon");
            same(&got, &fixture(&format!("theme_{id}.json")), id);
        }
    }

    fn effect_json(e: &ResolvedEffect, id: &str) -> Value {
        let mut v = json!({
            "id": id, "name": e.name, "icon": e.icon, "eq_bands": e.eq_bands, "eq_enabled": e.eq_enabled,
            "speed": e.speed, "pitch_semitones": e.pitch_semitones,
        });
        match &e.special {
            Some(PresetSpecialMode::Rotation { hz }) => v["special_mode"] = json!({"type": "rotation", "hz": hz}),
            Some(PresetSpecialMode::Tremolo { freq, depth }) => v["special_mode"] = json!({"type": "tremolo", "freq": freq, "depth": depth}),
            Some(PresetSpecialMode::Vibrato { freq, pitch_depth }) => {
                v["special_mode"] = json!({"type": "vibrato", "freq": freq, "pitch_depth": pitch_depth})
            }
            None => {}
        }
        v
    }

    #[test]
    fn effects_match_the_old_lua_output() {
        // Old file names → effect ids (smiley's script set id smiley_curve).
        let files = [
            ("analog_wow_flutter", "analog_wow_flutter"),
            ("auto_genre_match", "auto_genre_match"),
            ("de_esser", "de_esser"),
            ("genre_blend", "genre_blend"),
            ("heartbeat_pulse", "heartbeat_pulse"),
            ("parametric_bass_boost", "parametric_bass_boost"),
            ("smiley_curve_generator", "smiley_curve"),
            ("wide_8d_spin", "wide_8d_spin"),
        ];
        assert_eq!(bundled(PresetKind::Effect).len(), files.len());
        for (file, id) in files {
            let def = &bundled(PresetKind::Effect).iter().find(|(b, _)| b == id).unwrap().1;
            let e = resolve_effect(def, None).unwrap();
            same(&effect_json(&e, id), &fixture(&format!("fx_{file}.json")), id);
        }
    }

    #[test]
    fn genre_curves_follow_the_track() {
        let def = &bundled(PresetKind::Effect).iter().find(|(b, _)| b == "auto_genre_match").unwrap().1;
        let bands = |g: &str| resolve_effect(def, Some(&PresetTrack { genre: g.into(), bpm: None })).unwrap().eq_bands;
        assert_eq!(bands("Alternative Rock"), vec![4.0, 2.5, -1.0, -2.0, -1.0, 1.0, 2.5, 3.0, 2.0, 1.5]);
        assert_eq!(bands("EDM/Electronic"), vec![5.0, 3.5, 0.0, -1.5, -1.0, 0.5, 2.0, 3.5, 4.0, 3.0]);
        assert_eq!(bands("Folk"), vec![1.0, 0.5, 0.0, 0.5, 1.0, 1.0, 0.5, 0.0, 0.5, 1.0]);
        assert_eq!(bands("Classical"), vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.5, 1.0, 1.5]);
        assert_eq!(bands("Jazz"), vec![0.0; 10]);
        assert_eq!(bands(""), vec![0.0; 10]);
    }

    #[test]
    fn tremolo_can_follow_the_tempo() {
        let def = json!({"name": "Pump", "special_mode": {"type": "tremolo", "freq": 3, "depth": 0.5, "pulses_per_beat": 2}});
        let at = |bpm| resolve_effect(&def, Some(&PresetTrack { genre: String::new(), bpm })).unwrap().special;
        assert_eq!(at(Some(120.0)), Some(PresetSpecialMode::Tremolo { freq: 4.0, depth: 0.5 }));
        assert_eq!(at(None), Some(PresetSpecialMode::Tremolo { freq: 3.0, depth: 0.5 }));
    }

    #[test]
    fn visualizers_match_the_old_lua_output() {
        for (id, def) in bundled(PresetKind::Visualizer) {
            let v = resolve_visualizer(def).unwrap();
            let got = json!({"colors": v.colors, "sensitivity": v.sensitivity, "bar_corner_radius": v.bar_corner_radius,
                             "bar_spacing": v.bar_spacing, "mirrored": v.mirrored});
            same(&got, &fixture(&format!("vis_{id}.json")), id);
        }
    }

    #[test]
    fn import_list_resolve_delete() {
        let dir = std::env::temp_dir().join(format!("lumi-presets-{}", std::process::id()));
        let docs = dir.to_string_lossy().to_string();
        let src = r##"{"meta": {"name": "Night Mint"}, "colors": {"background": "#0b1f1a", "accent": "#3fe0b0",
                        "elevated_surface": {"shade_of": "surface", "amount": 0.1}}, "flags": {"reduce_motion": true}}"##;
        let e = presets_import(PresetKind::Theme, docs.clone(), src.into(), "".into()).unwrap();
        assert_eq!(e.id, "user:night_mint");
        assert_eq!(e.swatch, vec!["#0B1F1A", "#3FE0B0"]);
        let again = presets_import(PresetKind::Theme, docs.clone(), src.into(), "".into()).unwrap();
        assert_eq!(again.id, "user:night_mint_1");
        let list = presets_list(PresetKind::Theme, docs.clone());
        assert_eq!(list.len(), 16);
        assert!(list[..14].iter().all(|e| e.bundled && e.error.is_none()));
        let t = presets_theme(docs.clone(), "user:night_mint".into()).unwrap();
        assert_eq!(t.id, "user:night_mint");
        assert_eq!(t.surface, shade("#0B1F1A", 0.10).unwrap());
        assert_eq!(t.elevated_surface, shade(&t.surface, 0.1).unwrap());
        assert_eq!(t.text_primary, "#FFFFFF");
        assert_eq!(t.reduce_motion, Some(true));
        assert_eq!(t.font_style, None);
        presets_delete(PresetKind::Theme, docs.clone(), "user:night_mint".into()).unwrap();
        assert!(presets_delete(PresetKind::Theme, docs.clone(), "forest_calm".into()).is_err());
        assert_eq!(presets_list(PresetKind::Theme, docs.clone()).len(), 15);

        // Bad input is refused with a reason, and nothing is written.
        let lua = "local x = 1\ntheme = { }";
        assert!(presets_import(PresetKind::Theme, docs.clone(), lua.into(), "x".into()).unwrap_err().to_string().contains("Lua"));
        let bad = r##"{"name": "x", "eq_bands": [1, 2]}"##;
        assert!(presets_import(PresetKind::Effect, docs.clone(), bad.into(), "".into()).unwrap_err().to_string().contains("10"));
        let bad_ref = r##"{"meta": {"name": "x"}, "colors": {"background": "#000", "accent": "#fff", "surface": {"shade_of": "accent", "amount": 0.1}}}"##;
        assert!(presets_import(PresetKind::Theme, docs.clone(), bad_ref.into(), "".into()).is_err());
        assert!(presets_list(PresetKind::Effect, docs.clone()).iter().all(|e| e.bundled));

        // A shared bundled preset round-trips as a template.
        let fx = presets_source(PresetKind::Effect, docs.clone(), "lua:de_esser".into()).unwrap();
        let e = presets_import(PresetKind::Effect, docs.clone(), fx, "My De-Ess".into()).unwrap();
        assert_eq!(e.id, "user:my_de_ess");
        let a = presets_effect(docs.clone(), "de_esser".into(), None).unwrap();
        let b = presets_effect(docs.clone(), e.id.clone(), None).unwrap();
        assert_eq!(a.eq_bands, b.eq_bands);
        assert_eq!(b.id, "user:my_de_ess");

        // Legacy ids from the Lua era still resolve.
        assert_eq!(presets_theme(docs.clone(), "retroSkeuo".into()).unwrap().id, "retro_skeuomorphic");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
