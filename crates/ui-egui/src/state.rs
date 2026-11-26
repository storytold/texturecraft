//! UI state (serde, so the control channel can read and drive it).

use serde::{Deserialize, Serialize};
use texturecraft_engine::core::{BakedMap, Channel, ChannelBlend, NormalFormat};

use crate::docking::Dock;
use crate::keymap::Keymap;
use crate::saved_search::SavedSearch;
use crate::shelf_panes::ShelfPanes;
use crate::shelf_view::ShelfView;

mod import_resources;
mod import_row;
mod library_location;
mod project_config;
mod resource_usage;
pub use import_resources::ImportResources;
pub use import_row::ImportRow;
pub use library_location::LibraryLocation;
pub use project_config::ProjectConfig;
pub use resource_usage::ResourceUsage;

mod console_line;
mod export_job;
mod export_set_plan;
mod file_type;
mod map_export;
mod set_export;
pub use console_line::ConsoleLine;
pub use export_job::ExportJob;
pub use export_set_plan::ExportSetPlan;
pub use file_type::FileType;
pub use map_export::MapExport;
pub use set_export::SetExport;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    #[default]
    Painting,
    Baking,
    /// Path-traced view (F10).
    Rendering,
}

/// The Alpha tab's text: what to draw and in which font (empty: the default font).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextUi {
    pub text: String,
    pub font: String,
    pub style: String,
    /// Installed and built-in families with their styles (`text.fonts`), read when first shown.
    #[serde(skip)]
    pub fonts: Vec<(String, Vec<String>)>,
}

/// Rendering mode settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderUi {
    pub max_samples: u32,
    pub bounces: u32,
    pub denoise: bool,
    /// Path trace on the CPU even where the GPU tracer runs.
    pub cpu: bool,
}

impl Default for RenderUi {
    fn default() -> Self {
        // The web traces on one thread: fewer bounces keep it interactive
        RenderUi { max_samples: 256, bounces: if cfg!(target_arch = "wasm32") { 3 } else { 6 }, denoise: true, cpu: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewLayout {
    /// 3D and 2D side by side.
    #[default]
    Split,
    Only3D,
    Only2D,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PropsTab {
    #[default]
    Brush,
    Alpha,
    Material,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Windows {
    pub export: bool,
    pub new_project: bool,
    pub about: bool,
    pub shortcuts: bool,
    #[serde(default)]
    pub settings: bool,
    /// Browser File › Export Mesh: the download format choice.
    #[serde(default)]
    pub export_mesh: bool,
    /// File › Export Mesh: the Mesh Export Options prompt before the file choice
    #[serde(skip)]
    pub mesh_export_options: Option<u64>,
    /// Edit > Project Configuration: what is chosen so far, applied by OK.
    #[serde(skip)]
    pub project_config: Option<ProjectConfig>,
    /// Edit > Settings: the page and the settings edited so far, applied by OK.
    #[serde(skip)]
    pub settings_draft: Option<crate::settings_window::SettingsDraft>,
    /// The page Edit > Settings was on when it closed; the window opens there again.
    #[serde(skip)]
    pub settings_page: crate::settings_window::SettingsPage,
    /// File > Import resources: the files added so far, imported by Import.
    #[serde(skip)]
    pub import_resources: Option<ImportResources>,
    /// What waits on the unsaved-changes dialog: save, don't save or cancel.
    #[serde(skip)]
    pub unsaved: Option<Unsaved>,
    /// Window ▸ Reset UI is waiting for its answer.
    #[serde(skip)]
    pub confirm_reset_ui: bool,
    /// The generator effect whose preview is open enlarged in its own window
    #[serde(skip)]
    pub generator_preview: Option<u64>,
}

/// An action that replaces or ends the project, held while the project has unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Unsaved {
    Quit,
    /// A menu command (Open, Close, Open sample project).
    Command(String),
    /// Create from the New project form that requested confirmation.
    CreateProject(u64),
    /// Open this project or mesh file.
    #[cfg(not(target_arch = "wasm32"))]
    Open(String),
    /// Open a project or mesh file's bytes (picked or dropped in the browser): its name and bytes.
    Load(String, std::sync::Arc<Vec<u8>>),
}

/// Edit › Settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Interface zoom (1 = 100%).
    pub ui_scale: f32,
    /// Minutes between autosaves of a modified project (0: off; desktop).
    pub autosave_minutes: u32,
    /// Maximum recent projects retained (0 disables the history, capped at 100).
    pub recent_projects_limit: u16,
    /// UI language code (`i18n::LANGUAGES`), or `i18n::SYSTEM_LANGUAGE` to follow the system.
    pub language: String,
    /// Viewport texture resolution: 0 full, 1 half, 2 quarter of each texture set's size (lighter
    /// on GPU memory and uploads for 4K and 8K sets; painting and export keep full resolution).
    pub viewport_texture: u8,
    /// Interface brightness (the views keep their own background).
    pub brightness: crate::theme::Brightness,
    /// The colour of what is on, chosen or focused.
    pub accent: crate::theme::Accent,
    /// The system draws the window's title bar instead of TextureCraft (desktop; read at startup).
    pub system_title_bar: bool,
    /// Where autosaves, smart materials, presets, templates and layouts go (desktop; read at
    /// startup). Empty or relative: `Documents/TextureCraft`.
    pub data_folder: String,
    /// Cores a project save leaves to the UI; Settings bounds this to the available logical CPUs
    /// (desktop). At least one save worker always runs.
    pub reserved_cores: u32,
    /// Maximum decompressed project size accepted on open, in GiB (at least one).
    pub project_open_limit_gib: u32,
    /// Percent of the free RAM a project save may hold at once (desktop; 1 to 100).
    pub save_ram_percent: u8,
    /// Worker threads for painting, baking and export (desktop; 0: one per logical CPU). Read at
    /// startup, before the first parallel job.
    pub work_threads: u32,
    /// 3D view orbit speed; [`DEFAULT_ROTATION_SPEED`] is the built-in speed.
    pub rotation_speed: f32,
    /// 3D view drag zoom speed (1 is the built-in speed) and whether it runs the other way.
    pub zoom_speed: f32,
    pub zoom_inverse: bool,
    /// 3D view mouse wheel zoom speed (1 is the built-in speed) and whether it runs the other way.
    pub wheel_speed: f32,
    pub wheel_inverse: bool,
    /// Bakes cast their rays on the GPU when one is available (else on the CPU).
    pub gpu_raytracing: bool,
    /// Library locations the user added (Settings > Libraries), after the built-in ones.
    pub libraries: Vec<LibraryLocation>,
    /// The plain background of the 3D and 2D views at Environment Opacity 0 (Settings > General >
    /// Background color): the color at the top and at the bottom, linear RGB.
    pub background: [[f32; 3]; 2],
}

/// The orbit speed the built-in navigation runs at.
pub const DEFAULT_ROTATION_SPEED: f32 = 2.0;
/// The range of the camera speed sliders.
pub const CAMERA_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.1..=10.0;

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            ui_scale: 1.0,
            autosave_minutes: 10,
            recent_projects_limit: 10,
            language: "en".into(),
            viewport_texture: 0,
            brightness: crate::theme::Brightness::default(),
            accent: crate::theme::Accent::default(),
            system_title_bar: false,
            data_folder: String::new(),
            reserved_cores: 1,
            project_open_limit_gib: texturecraft_engine::formats::DEFAULT_PROJECT_OPEN_LIMIT_GIB,
            save_ram_percent: 50,
            work_threads: 0,
            rotation_speed: DEFAULT_ROTATION_SPEED,
            zoom_speed: 1.0,
            zoom_inverse: false,
            wheel_speed: 1.0,
            wheel_inverse: false,
            gpu_raytracing: true,
            libraries: Vec::new(),
            background: texturecraft_render::DEFAULT_BACKGROUND,
        }
    }
}

/// `value` as a camera speed: inside the sliders' range, the default for a value that is not a number.
fn camera_speed(value: f32, default: f32) -> f32 {
    if value.is_finite() { value.clamp(*CAMERA_SPEED_RANGE.start(), *CAMERA_SPEED_RANGE.end()) } else { default }
}

impl Prefs {
    /// Apply resource preferences before commands or file workers can start.
    pub fn apply_resource_limits(&self) {
        texturecraft_engine::formats::set_project_open_limit_gib(self.project_open_limit_gib);
        #[cfg(not(target_arch = "wasm32"))]
        texturecraft_engine::formats::set_save_resources(self.reserved_cores as usize, self.save_ram_percent);
        texturecraft_engine::bake::set_gpu_raytracing(self.gpu_raytracing);
    }

    /// Size the shared worker pool to `work_threads` (0 keeps one thread per logical CPU). Only
    /// the first call of a process can do this, before any parallel work: true when it did.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn apply_work_threads(&self) -> bool {
        if self.work_threads == 0 {
            return false;
        }

        let available_threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let work_threads = (self.work_threads as usize).min(available_threads);
        rayon::ThreadPoolBuilder::new().num_threads(work_threads).build_global().is_ok()
    }

    /// The 3D view's orbit per dragged point, relative to the built-in speed.
    pub fn orbit_scale(&self) -> f32 {
        camera_speed(self.rotation_speed, DEFAULT_ROTATION_SPEED) / DEFAULT_ROTATION_SPEED
    }

    /// The 3D view's drag zoom relative to the built-in speed; negative when inverted.
    pub fn drag_zoom_scale(&self) -> f32 {
        let speed = camera_speed(self.zoom_speed, 1.0);
        if self.zoom_inverse { -speed } else { speed }
    }

    /// The 3D view's wheel zoom relative to the built-in speed; negative when inverted.
    pub fn wheel_zoom_scale(&self) -> f32 {
        let speed = camera_speed(self.wheel_speed, 1.0);
        if self.wheel_inverse { -speed } else { speed }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewProject {
    pub resolution: u32,
    /// The mesh file chosen with Select... (its path on the desktop, its name in the browser).
    pub file: Option<String>,
    /// The browser's copy of the chosen mesh file, read when Create is pressed.
    #[serde(skip)]
    pub file_bytes: Option<std::sync::Arc<Vec<u8>>>,
    /// The active form opening, used to reject late file picker answers.
    #[serde(skip)]
    pub opening_id: u64,
    /// The latest mesh pick requested by this form.
    #[serde(skip)]
    pub mesh_request_id: u64,
    /// Use UV Tile workflow: Some(true) makes UV tile (UDIM) texture sets; otherwise each set paints tile 1001.
    pub uv_tiles: Option<bool>,
    /// Project template name (built-in or the user's).
    #[serde(default = "default_template")]
    pub template: String,
    /// Normal map format of the project and its imported normal maps; None keeps the template's
    #[serde(default)]
    pub normal_format: Option<NormalFormat>,
    /// Import settings > Import cameras: keep the cameras stored in the mesh file
    #[serde(default = "import_cameras_default")]
    pub import_cameras: bool,
    /// Import settings > Rebuild mesh UVs.
    #[serde(default)]
    pub rebuild_uvs: bool,
    /// Color management as chosen in the window (the project is always created as Legacy)
    #[serde(skip)]
    pub color_management: crate::project_form::ColorManagement,
    /// Baked map files to import at creation, matched to sets and mesh maps by name (desktop only)
    #[serde(skip)]
    pub baked_maps: Vec<String>,
}

fn default_template() -> String {
    texturecraft_engine::template_cmds::DEFAULT_TEMPLATE.into()
}

fn import_cameras_default() -> bool {
    true
}

impl NewProject {
    pub(crate) fn clear_mesh(&mut self) {
        self.file = None;
        self.file_bytes = None;
        self.opening_id = 0;
        self.mesh_request_id = 0;
    }
}

impl Default for NewProject {
    fn default() -> Self {
        NewProject {
            resolution: 1024,
            file: None,
            file_bytes: None,
            opening_id: 0,
            mesh_request_id: 0,
            uv_tiles: None,
            template: default_template(),
            normal_format: None,
            import_cameras: true,
            rebuild_uvs: false,
            color_management: crate::project_form::ColorManagement::Legacy,
            baked_maps: Vec::new(),
        }
    }
}

/// `ExportUi::size` for "Maximum size for all Texture Sets".
pub const SIZE_OF_LARGEST_SET: u32 = u32::MAX;

/// The export dialog's settings (remembered between sessions).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportUi {
    pub dir: String,
    pub preset: String,
    /// File extension override ("" = the preset's formats).
    pub format: String,
    /// Bit depth override (0 = the preset's).
    pub bit_depth: u8,
    /// Dither the 8-bit files of the file type override.
    #[serde(default)]
    pub dithering: bool,
    /// none | dilation | dilationDefault | diffusion | infinite
    pub padding: String,
    pub dilation: u32,
    /// Output size override (0 = each texture set's size, [`SIZE_OF_LARGEST_SET`] = the largest set's).
    pub size: u32,
    /// "" (project), openGl or directX.
    pub normal_format: String,
    /// "" (by file format: sRGB, or linear for float files), or a colour space name.
    #[serde(default)]
    pub color_space: String,
    /// Also write `<mesh>.usd`: the mesh and a material per texture set that reads the exported
    /// textures (`usdAsset` of `file.exportTextures`).
    #[serde(default)]
    pub usd_asset: bool,
    /// Also write an OpenPBR MaterialX (`.mtlx`) file per texture set (`materialX` of
    /// `file.exportTextures`).
    #[serde(default)]
    pub materialx: bool,
    /// Texture sets excluded from export.
    pub skip: Vec<usize>,
    /// Per texture set (by index) overrides of the settings above.
    #[serde(default)]
    pub sets: std::collections::BTreeMap<usize, SetExport>,
    /// Whose `skip` and `sets` these are: the project name, then its texture set names in order.
    /// Another project drops them; texture sets that moved take theirs along, removed ones lose them.
    #[serde(default)]
    pub owner: Vec<String>,
    /// The session's project id `owner` was taken in (None after a restart: the name decides).
    #[serde(skip)]
    pub owner_id: Option<u64>,
    pub tab: u8,
    /// SETTINGS tab: the texture set whose page shows (None: Global settings).
    #[serde(skip)]
    pub page: Option<usize>,
    /// Output templates tab: the selected user preset being edited (saved whenever it is valid).
    #[serde(skip)]
    pub draft: Option<texturecraft_engine::formats::ExportPreset>,
    /// Output templates tab: for each map of `draft`, its index in the saved template (None: added
    /// since), so the texture set overrides of a map follow it when the draft is saved.
    #[serde(skip)]
    pub draft_maps: Vec<Option<usize>>,
    /// Output templates tab: the saved template `draft_maps` counts from.
    #[serde(skip)]
    pub draft_base: Option<texturecraft_engine::formats::ExportPreset>,
    /// Output templates tab: for each map of `draft`, whether its R, G and B show as separate
    /// channel swatches (R+G+B) instead of one RGB swatch.
    #[serde(skip)]
    pub draft_split: Vec<bool>,
    /// LIST OF EXPORTS: the console's width, set by dragging the strip at its left edge.
    #[serde(skip, default = "default_console_width")]
    pub console_width: f32,
    /// Output templates tab: a map dropped on an output channel, waiting for the source channel
    /// ("Which part of <map>?").
    #[serde(skip)]
    pub pending_drop: Option<crate::export_templates::PendingDrop>,
    /// Output templates tab: the new name while renaming the selected preset.
    #[serde(skip)]
    pub renaming: Option<String>,
    /// Output templates tab: F2 asked to rename the selected preset.
    #[serde(skip)]
    pub rename_key: bool,
    /// Output templates tab: the question waiting for an answer (delete, or drop an unsaved edit).
    #[serde(skip)]
    pub ask: Option<crate::export_templates::Ask>,
    /// The settings when the window opened: Cancel goes back to them.
    #[serde(skip)]
    pub opened_with: Option<Box<ExportUi>>,
    /// The export running in the window.
    #[serde(skip)]
    pub job: Option<ExportJob>,
    /// LIST OF EXPORTS: the console's lines and whether it is hidden.
    #[serde(skip)]
    pub console: Vec<ConsoleLine>,
    #[serde(skip)]
    pub console_hidden: bool,
}

impl Default for ExportUi {
    fn default() -> Self {
        ExportUi {
            dir: default_export_dir(),
            preset: texturecraft_engine::formats::DEFAULT_PRESET.into(),
            format: String::new(),
            bit_depth: 0,
            dithering: false,
            padding: "infinite".into(),
            dilation: 16,
            size: 0,
            normal_format: String::new(),
            color_space: String::new(),
            usd_asset: false,
            materialx: false,
            skip: Vec::new(),
            sets: Default::default(),
            owner: Vec::new(),
            owner_id: None,
            tab: 0,
            page: None,
            draft: None,
            draft_maps: Vec::new(),
            draft_base: None,
            draft_split: Vec::new(),
            console_width: default_console_width(),
            pending_drop: None,
            renaming: None,
            rename_key: false,
            ask: None,
            opened_with: None,
            job: None,
            console: Vec::new(),
            console_hidden: false,
        }
    }
}

/// The LIST OF EXPORTS console's width when the window opens, as in the reference
fn default_console_width() -> f32 {
    300.0
}

fn default_export_dir() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::user_dir("export").display().to_string()
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

/// The split layout starts with the 3D and 2D views at equal width
fn default_split_ratio() -> f32 {
    0.5
}

/// Saved searches start with this share of the Assets panel's pane column while Filter by path is open too
pub(crate) fn default_shelf_panes_split() -> f32 {
    0.45
}

fn deserialize_shelf_panes_split<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let share = f32::deserialize(deserializer)?;

    // A finite JSON number can overflow f32 and prevent the next settings save
    Ok(if share.is_finite() { share } else { default_shelf_panes_split() })
}

/// "Wireframe opacity" until changed: 40 of 100, as in the reference app
fn default_wireframe_opacity() -> f32 {
    40.0
}

/// The Layers panel's channel list starts on Material, so the Material tab's orb shows the whole material
fn default_layer_material_view() -> bool {
    true
}

/// Settings that start on, for saved states that predate them
fn default_true() -> bool {
    true
}

/// The bake layout's version: saved bake docks from an older one load as `Dock::for_baking`
const BAKE_LAYOUT_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiState {
    /// Layer menu ▸ Blending options ▸ Copy: each channel's blend mode and opacity of the copied layer.
    #[serde(skip)]
    pub blend_clipboard: Option<Vec<(Channel, ChannelBlend)>>,
    pub mode: Mode,
    pub layout: ViewLayout,
    /// The split layout shows the 2D view on the left and the 3D view on the right
    #[serde(default)]
    pub swap_views: bool,
    /// The 3D view's share of the viewport width in the split layout
    #[serde(default = "default_split_ratio")]
    pub split_ratio: f32,
    /// The docked, tabbed and floating panels around the viewport.
    #[serde(default)]
    pub dock: Dock,
    /// The panel open against the icon rail, if any; not saved, so the app starts with the rail closed
    #[serde(skip)]
    pub rail_panel: Option<crate::docking::Panel>,
    #[serde(default = "Dock::for_baking")]
    pub(crate) inactive_mode_dock: Dock,
    #[serde(default)]
    pub(crate) dock_is_baking: bool,
    /// Rendering's saved dock, or Painting's while Rendering is active. Older states start from Painting.
    #[serde(default)]
    pub(crate) inactive_render_dock: Option<Dock>,
    #[serde(default)]
    pub(crate) dock_is_rendering: bool,
    /// Window ▸ Toolbars ▸ Docks: the right bar of panel icons
    #[serde(default = "default_true")]
    pub toolbar_docks: bool,
    /// Window ▸ Toolbars ▸ Tools: the tool buttons on the left
    #[serde(default = "default_true")]
    pub toolbar_tools: bool,
    /// The bake layout version the bake dock was saved with; 0 before versions existed.
    #[serde(default)]
    pub(crate) bake_layout_version: u32,
    /// The mesh map whose settings Mesh Map Settings shows; `None` follows the first enabled map.
    #[serde(default)]
    pub(crate) selected_baked_map: Option<BakedMap>,
    /// Window ▸ Hide UI (Tab): only the viewport.
    pub hide_ui: bool,
    pub windows: Windows,
    pub uv_zoom: f32,
    pub uv_pan: [f32; 2],
    /// The 2D view's turn, counter-clockwise on screen in radians (Alt+left drag)
    #[serde(default)]
    pub uv_rotation: f32,
    /// Channel whose blending the layer rows show.
    pub layer_channel: Channel,
    /// The Layers panel's channel list is on Material: the Material tab's orb shows the whole material, else `layer_channel` alone
    #[serde(default = "default_layer_material_view")]
    pub layer_material_view: bool,
    pub shelf_filter: String,
    /// The asset shelf's picked type: an index into the shelf's types, or past them for every type
    #[serde(default)]
    pub shelf_tab: usize,
    /// The asset shelf's library folder: a library id, then folder names (empty: all libraries)
    #[serde(default)]
    pub shelf_path: Vec<String>,
    /// The asset shelf's tile size, or its list
    #[serde(default)]
    pub shelf_view: ShelfView,
    /// The asset shelf's open side panes
    #[serde(default)]
    pub shelf_panes: ShelfPanes,
    /// The asset shelf's saved queries, in the order they were saved
    #[serde(default)]
    pub shelf_saved_searches: Vec<SavedSearch>,
    /// Saved searches' share of the pane column above Filter by path, set by the splitter between them
    #[serde(default = "default_shelf_panes_split", deserialize_with = "deserialize_shelf_panes_split")]
    pub shelf_panes_split: f32,
    pub props_tab: PropsTab,
    #[serde(default)]
    pub text: TextUi,
    /// Layer being renamed and the edit buffer.
    pub rename: Option<(u64, String)>,
    pub new_project: NewProject,
    /// Show mesh wireframe: the mesh's edges over the 3D view and its UV edges in the 2D view (off by default)
    #[serde(default, alias = "wireframe_3d")]
    pub mesh_wireframe: bool,
    /// Wireframe opacity, 0 to 100
    #[serde(default = "default_wireframe_opacity")]
    pub wireframe_opacity: f32,
    /// The 2D view of a UV tile project shows the UDIM numbers and the hover label (the button left of the view mode)
    #[serde(default = "default_true")]
    pub uv_tile_info: bool,
    /// Show the orbit, pan and zoom buttons in the corner of the 3D and 2D views (Viewport menu)
    #[serde(default = "default_true")]
    pub nav_buttons: bool,
    #[serde(default)]
    pub render: RenderUi,
    #[serde(default)]
    pub prefs: Prefs,
    #[serde(default)]
    pub export: ExportUi,
    /// Recently opened or saved projects (desktop).
    #[serde(default)]
    pub recent: Vec<String>,
    /// Custom keyboard shortcuts (item id → "Ctrl+Shift+E", "" = none) over the defaults.
    #[serde(default)]
    pub keymap: Keymap,
    /// The brush that paints the skew correction map in the 2D view of bake mode.
    #[serde(default)]
    pub bake_paint_brush: crate::bake_mode::BakePaintBrush,
    /// How the 3D view shows symmetry
    #[serde(default)]
    pub symmetry: crate::symmetry::SymmetryDisplay,
    /// The warp tool bar: the gizmo's mode and space, its size, and the steps its drags snap to
    #[serde(default)]
    pub warp_toolbar: crate::frame_gizmo::warp_toolbar::WarpToolbarSettings,
    /// The brush, symmetry and other tool settings of the last session, which the next one starts with
    #[serde(default)]
    pub tool_settings: texturecraft_engine::ToolSettings,
    /// The Baking Visualization panel's settings, under a key of their own since the reference defaults: older saved ones start over
    #[serde(default, rename = "baking_visualization")]
    pub bake_visualization: crate::bake_mode::BakeVisualizationToggles,
    /// Panel arrangements saved under a name (Window > Layouts), in the order they were saved
    #[serde(default)]
    pub named_layouts: Vec<crate::named_layouts::NamedLayout>,
    /// Bake settings saved under a name (Mesh Map Bakers > Presets), in the order they were saved
    #[serde(default)]
    pub bake_presets: Vec<crate::bake_presets::BakePreset>,
    /// The name typed in the bake presets menu
    #[serde(skip)]
    pub bake_preset_name: String,
    /// The save, replace or manage dialog of the named layouts
    #[serde(skip)]
    pub layout_dialog: Option<crate::named_layouts::LayoutDialog>,
    /// The Log panel's shown channel tab
    #[serde(default)]
    pub log_channel: crate::log_panel::LogChannel,
    /// The project template the last New project was created from (name or label)
    #[serde(default)]
    pub last_template: String,
    /// The selected Log lines (indices into the session log); not saved
    #[serde(skip)]
    pub log_selection: std::collections::BTreeSet<usize>,
    /// The Log line a Shift+click extends from
    #[serde(skip)]
    pub log_anchor: Option<usize>,
    /// Identity of the log that owns the selected rows.
    #[serde(skip)]
    pub log_id: u64,
}

pub(crate) const MAX_RECENT_PROJECTS: u16 = 100;

impl UiState {
    /// Keep recent-file history bounded, including values supplied through automation or old settings.
    pub(crate) fn trim_recent(&mut self) {
        self.prefs.recent_projects_limit = self.prefs.recent_projects_limit.min(MAX_RECENT_PROJECTS);
        self.recent.truncate(usize::from(self.prefs.recent_projects_limit));
    }

    /// Replace an outdated or unreadable saved bake dock by the bake layout, then give the mode its dock
    pub(crate) fn sync_mode_dock(&mut self) {
        if self.dock_is_rendering {
            if let Some(other) = self.inactive_render_dock.as_mut() {
                std::mem::swap(&mut self.dock, other);
            }
            self.dock_is_rendering = false;
        }
        self.reset_outdated_bake_dock();

        let baking = self.mode == Mode::Baking;
        if baking != self.dock_is_baking {
            std::mem::swap(&mut self.dock, &mut self.inactive_mode_dock);
            self.dock_is_baking = baking;
        }
        if self.mode == Mode::Rendering {
            let other = self.inactive_render_dock.get_or_insert_with(|| self.dock.clone());
            std::mem::swap(&mut self.dock, other);
            self.dock_is_rendering = true;
        }
    }

    /// Replace a bake dock saved with an older bake layout, or one that could not be read, by the bake default
    fn reset_outdated_bake_dock(&mut self) {
        let bake_dock = if self.dock_is_baking { &mut self.dock } else { &mut self.inactive_mode_dock };
        if self.bake_layout_version >= BAKE_LAYOUT_VERSION && !bake_dock.replaced_unreadable() {
            return;
        }

        *bake_dock = Dock::for_baking();
        self.bake_layout_version = BAKE_LAYOUT_VERSION;
    }

    /// Put the panels, tabs and floating panels of every mode back to their defaults; the view split,
    /// zoom, wireframes, symmetry display, mode and every setting stay as they are
    pub fn reset_panel_layout(&mut self) {
        let (painting, baking) = (Dock::default(), Dock::for_baking());
        (self.dock, self.inactive_mode_dock) = if self.dock_is_baking { (baking, painting) } else { (painting, baking) };
        self.inactive_render_dock = self.dock_is_rendering.then(Dock::default);
        self.bake_layout_version = BAKE_LAYOUT_VERSION;
        self.rail_panel = None;
    }

    /// Take `layout`'s panels, windows and view settings, keeping this state's dialogs, settings, files and shelf query
    pub fn adopt_layout(&mut self, layout: UiState) {
        let keep = std::mem::replace(self, layout);
        self.new_project = keep.new_project;
        if !self.windows.new_project {
            self.new_project.clear_mesh();

            // Keep a failed import queued when the form was already closed
            if keep.windows.new_project {
                self.new_project.baked_maps = Vec::new();
            }
        }

        self.export = keep.export;
        self.recent = keep.recent;
        self.last_template = keep.last_template;
        self.prefs = keep.prefs;
        self.nav_buttons = keep.nav_buttons;
        self.keymap = keep.keymap;
        self.tool_settings = keep.tool_settings;
        self.warp_toolbar = keep.warp_toolbar;
        self.named_layouts = keep.named_layouts;
        self.bake_presets = keep.bake_presets;
        self.layout_dialog = None;
        self.rename = None;

        // The shelf's search, type and folder are one query, and saved searches are the user's own
        self.shelf_filter = keep.shelf_filter;
        self.shelf_tab = keep.shelf_tab;
        self.shelf_path = keep.shelf_path;
        self.shelf_saved_searches = keep.shelf_saved_searches;
    }
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            mode: Mode::Painting,
            layout: ViewLayout::Split,
            swap_views: false,
            split_ratio: default_split_ratio(),
            dock: Dock::default(),
            rail_panel: None,
            inactive_mode_dock: Dock::for_baking(),
            dock_is_baking: false,
            inactive_render_dock: None,
            dock_is_rendering: false,
            toolbar_docks: true,
            toolbar_tools: true,
            bake_layout_version: BAKE_LAYOUT_VERSION,
            selected_baked_map: None,
            hide_ui: false,
            windows: Windows::default(),
            uv_zoom: 1.0,
            uv_pan: [0.0, 0.0],
            uv_rotation: 0.0,
            layer_channel: Channel::BaseColor,
            blend_clipboard: None,
            layer_material_view: true,
            shelf_filter: String::new(),
            shelf_tab: 0,
            shelf_path: Vec::new(),
            shelf_view: ShelfView::default(),
            shelf_panes: ShelfPanes::default(),
            shelf_saved_searches: Vec::new(),
            shelf_panes_split: default_shelf_panes_split(),
            props_tab: PropsTab::Brush,
            text: TextUi::default(),
            rename: None,
            new_project: NewProject::default(),
            mesh_wireframe: false,
            wireframe_opacity: default_wireframe_opacity(),
            uv_tile_info: true,
            nav_buttons: true,
            render: RenderUi::default(),
            prefs: Prefs::default(),
            export: ExportUi::default(),
            recent: Vec::new(),
            keymap: Keymap::new(),
            bake_paint_brush: crate::bake_mode::BakePaintBrush::default(),
            symmetry: crate::symmetry::SymmetryDisplay::default(),
            warp_toolbar: crate::frame_gizmo::warp_toolbar::WarpToolbarSettings::default(),
            tool_settings: Default::default(),
            bake_visualization: crate::bake_mode::BakeVisualizationToggles::default(),
            named_layouts: Vec::new(),
            bake_presets: Vec::new(),
            bake_preset_name: String::new(),
            layout_dialog: None,
            log_channel: crate::log_panel::LogChannel::default(),
            last_template: String::new(),
            log_selection: Default::default(),
            log_anchor: None,
            log_id: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_speeds_scale_the_built_in_navigation_and_inverse_flips_it() {
        let mut prefs = Prefs::default();
        assert_eq!((prefs.orbit_scale(), prefs.drag_zoom_scale(), prefs.wheel_zoom_scale()), (1.0, 1.0, 1.0), "the defaults keep the built-in speeds");

        prefs.rotation_speed = 4.0;
        prefs.zoom_speed = 0.5;
        prefs.zoom_inverse = true;
        prefs.wheel_inverse = true;
        assert_eq!((prefs.orbit_scale(), prefs.drag_zoom_scale(), prefs.wheel_zoom_scale()), (2.0, -0.5, -1.0));

        // Values from an edited settings file stay usable
        prefs.rotation_speed = f32::NAN;
        prefs.wheel_speed = 1e9;
        assert_eq!(prefs.orbit_scale(), 1.0);
        assert_eq!(prefs.wheel_zoom_scale(), -*CAMERA_SPEED_RANGE.end());
    }

    #[test]
    fn settings_saved_before_the_new_options_load_their_defaults() {
        let prefs: Prefs = serde_json::from_str(r#"{"autosave_minutes": 5}"#).unwrap();
        assert_eq!(prefs.autosave_minutes, 5);
        assert_eq!((prefs.rotation_speed, prefs.zoom_speed, prefs.wheel_speed), (2.0, 1.0, 1.0));
        assert!(prefs.gpu_raytracing && !prefs.zoom_inverse && !prefs.wheel_inverse);
        assert_eq!(prefs.work_threads, 0);
        assert!(prefs.libraries.is_empty());
    }

    #[test]
    fn the_first_run_follows_the_system_language() {
        let mut session = texturecraft_engine::Session::closed().unwrap();
        assert_eq!(crate::start_ui(None, &mut session).prefs.language, crate::i18n::SYSTEM_LANGUAGE);
    }

    #[test]
    fn adopting_a_layout_keeps_recent_files_and_user_settings() {
        use crate::frame_gizmo::warp_toolbar::{GizmoMode, GizmoSpace, WarpToolbarSettings};

        let warp_toolbar = WarpToolbarSettings { mode: GizmoMode::Rotate, space: GizmoSpace::World, manipulator_size: 0.4, grid_steps: 0.5, angle_steps: 30.0 };
        let mut ui = UiState { recent: vec!["a.tcraft".into()], ..UiState::default() };
        ui.warp_toolbar = warp_toolbar;
        ui.export.preset = "Mine".into();
        ui.keymap.insert("file.save".into(), "F5".into());
        let mut layout = UiState::default();
        layout.dock.close(crate::docking::Panel::Assets);

        ui.adopt_layout(layout);

        assert!(!ui.dock.is_open(crate::docking::Panel::Assets));
        assert_eq!(ui.recent, vec!["a.tcraft".to_string()]);
        assert_eq!(ui.export.preset, "Mine");
        assert_eq!(ui.keymap.get("file.save").map(String::as_str), Some("F5"));
        assert_eq!(ui.warp_toolbar, warp_toolbar, "loading a layout keeps the user's warp settings");
    }

    #[test]
    fn resetting_the_panel_layout_restores_both_docks_and_keeps_the_rest() {
        for baking in [false, true] {
            let mode = if baking { Mode::Baking } else { Mode::Painting };
            let mut ui = UiState { mode, ..UiState::default() };
            ui.sync_mode_dock();
            ui.dock.close(crate::docking::Panel::Log);
            ui.inactive_mode_dock.close(crate::docking::Panel::Log);
            ui.layout = ViewLayout::Only3D;
            ui.uv_zoom = 3.0;
            ui.mesh_wireframe = true;
            ui.recent = vec!["a.tcraft".into()];

            ui.reset_panel_layout();

            let mut fresh = UiState { mode, ..UiState::default() };
            fresh.sync_mode_dock();
            assert_eq!((&ui.dock, &ui.inactive_mode_dock), (&fresh.dock, &fresh.inactive_mode_dock), "baking: {baking}");
            assert_eq!((ui.layout, ui.uv_zoom, ui.mesh_wireframe, ui.mode), (ViewLayout::Only3D, 3.0, true, mode));
            assert_eq!(ui.recent, vec!["a.tcraft".to_string()]);
        }
    }

    #[test]
    fn a_saved_state_with_the_old_3d_wireframe_switch_keeps_it_as_the_one_mesh_wireframe_switch() {
        let mut saved = serde_json::to_value(UiState::default()).unwrap();
        let fields = saved.as_object_mut().unwrap();
        assert_eq!((fields.remove("mesh_wireframe"), fields.remove("wireframe_opacity")), (Some(false.into()), Some(40.0.into())));
        fields.insert("wireframe_3d".into(), true.into());
        fields.insert("uv_wireframe".into(), true.into());

        let loaded: UiState = serde_json::from_value(saved).unwrap();
        assert!(loaded.mesh_wireframe);
        assert_eq!(loaded.wireframe_opacity, 40.0);
        assert!(loaded.uv_tile_info);

        let fresh = UiState::default();
        assert!(!fresh.mesh_wireframe, "off by default, the UV wireframe too");
        assert_eq!(fresh.wireframe_opacity, 40.0);
    }

    #[test]
    fn adopting_a_layout_keeps_the_shelf_query_and_saved_searches() {
        let saved = crate::saved_search::SavedSearch { name: "rust".into(), filter: "rust".into(), tab: 2, path: vec!["starter".into()] };
        let mut ui = UiState { shelf_filter: "rust".into(), shelf_tab: 2, shelf_path: vec!["starter".into()], ..UiState::default() };
        ui.shelf_saved_searches.push(saved.clone());

        ui.adopt_layout(UiState::default());

        assert_eq!((ui.shelf_filter.as_str(), ui.shelf_tab, ui.shelf_path.as_slice()), ("rust", 2, ["starter".to_string()].as_slice()));
        assert_eq!(ui.shelf_saved_searches, [saved]);
    }

    #[test]
    fn the_data_folder_is_an_absolute_choice_or_documents_texturecraft() {
        let default = crate::data_folder("");
        assert!(default.ends_with(std::path::Path::new("Documents").join("TextureCraft")));
        assert_eq!(crate::data_folder("relative/dir"), default, "a relative path is not a choice");
        let chosen = std::env::temp_dir().join("tc-data");
        assert_eq!(crate::data_folder(&format!(" {} ", chosen.display())), chosen);
        assert_eq!(Prefs::default().data_folder, "", "the default is no choice");
    }

    #[test]
    fn a_bake_dock_saved_before_the_bake_layout_version_loads_the_current_bake_layout() {
        let old_bake_dock = serde_json::json!({"version": 2, "layout": {"root": {"Split": {"axis": "Horizontal", "size": {"FixedFirst": 320.0},
            "first": {"Tabs": {"panels": ["TextureSetList", "BakeSettings", "BakeMaps"], "active": 0}},
            "second": {"Tabs": {"panels": ["Viewport"], "active": 0}}}}, "floating": []}});
        for dock_is_baking in [false, true] {
            let mut painting_dock = Dock::default();
            painting_dock.close(crate::docking::Panel::Assets);
            let (bake_field, painting_field) = if dock_is_baking { ("dock", "inactive_mode_dock") } else { ("inactive_mode_dock", "dock") };
            let mut saved = serde_json::to_value(UiState::default()).unwrap();
            let fields = saved.as_object_mut().unwrap();
            fields.remove("bake_layout_version");
            fields.insert("dock_is_baking".into(), serde_json::json!(dock_is_baking));
            fields.insert("mode".into(), serde_json::to_value(if dock_is_baking { Mode::Baking } else { Mode::Painting }).unwrap());
            fields.insert(bake_field.into(), old_bake_dock.clone());
            fields.insert(painting_field.into(), serde_json::to_value(&painting_dock).unwrap());

            let mut ui: UiState = serde_json::from_value(saved).unwrap();
            let loaded_bake_dock = if dock_is_baking { &ui.dock } else { &ui.inactive_mode_dock };
            assert!(!loaded_bake_dock.is_open(crate::docking::Panel::MeshMapBakers), "the old layout loads without the bakers list");

            ui.sync_mode_dock();
            let (bake_dock, painting) = if dock_is_baking { (&ui.dock, &ui.inactive_mode_dock) } else { (&ui.inactive_mode_dock, &ui.dock) };
            assert_eq!(*bake_dock, Dock::for_baking());
            assert_eq!(*painting, painting_dock, "the painting layout is kept");

            let mut reloaded: UiState = serde_json::from_value(serde_json::to_value(&ui).unwrap()).unwrap();
            reloaded.sync_mode_dock();
            assert_eq!(reloaded, ui, "a current layout is not reset again");
        }
    }

    #[test]
    fn the_bake_layout_survives_a_restart_whichever_mode_was_open() {
        for mode in [Mode::Painting, Mode::Baking] {
            let mut ui = UiState { mode, ..UiState::default() };
            ui.sync_mode_dock();

            let mut restarted = crate::ui_state_store::tests::restart(&ui);
            restarted.sync_mode_dock();
            let bake_dock = if mode == Mode::Baking { &restarted.dock } else { &restarted.inactive_mode_dock };
            assert_eq!(*bake_dock, Dock::for_baking(), "{mode:?}");
            assert!(bake_dock.is_open(crate::docking::Panel::MeshMapBakers), "{mode:?}");
        }
    }

    #[test]
    fn older_saved_state_without_a_dock_loads_the_default_layout() {
        let mut v = serde_json::to_value(UiState::default()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("dock");
        obj.remove("keymap");
        obj.insert("show_shelf".into(), serde_json::json!(false));
        let ui: UiState = serde_json::from_value(v).unwrap();
        assert_eq!(ui.dock, Dock::default());
    }
}
