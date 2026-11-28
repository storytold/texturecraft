//! The dock host: every side panel (assets, texture set list, layers, texture set settings,
//! properties, display/shader settings, history, log) is a tab that can be docked, tabbed,
//! resized, floated and closed around the viewport. craft-ui's `DockArea` draws the frames, tab
//! strips, floating windows and drop previews and owns the gestures; this module names the
//! panels, builds the default arrangement, applies the dock's changes and hosts the panel bodies.
//! The bodies live in `dock.rs`, `shelf.rs` and `windows.rs`.
//!
//! The viewport is the central canvas: it stays docked, never takes tabs and is the only panel
//! that cannot close or float. The layout persists with `UiState` as a craft-behavior
//! `Workspace`; a layout that does not load or validate becomes the default.

use std::sync::Arc;

use craft_behavior::dock::{Action, DockPlacement, Layout, Location, Node, PanelState, Permissions, SplitSize};
use craft_behavior::flow::Axis;
use craft_ui::contract::{Bind, BodyCx, PlainData, SizeLimits, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, GroupAt, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use egui::{Margin, Rect, Ui, Vec2, ViewportId, vec2};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    Viewport,
    Assets,
    // Saved layouts name the bake panels by their earlier names
    #[serde(rename = "BakeMaps")]
    MeshMapSettings,
    #[serde(rename = "BakeSettings")]
    CommonSettings,
    MeshMapBakers,
    TextureSetList,
    Layers,
    TextureSetSettings,
    Properties,
    Display,
    Shader,
    History,
    Log,
    BakingLog,
    Export,
    /// The path tracer's settings (Window ▸ Views ▸ Renderer Settings, Rendering mode only)
    Renderer,
}

/// Window ▸ Views items and the panels they show or hide.
pub const VIEWS: [(&str, Panel); 15] = [
    ("ui.view.assets", Panel::Assets),
    ("ui.view.bakingLog", Panel::BakingLog),
    ("ui.view.bakeMaps", Panel::MeshMapSettings),
    ("ui.view.bakeSettings", Panel::CommonSettings),
    ("ui.view.display", Panel::Display),
    ("ui.view.export", Panel::Export),
    ("ui.view.history", Panel::History),
    ("ui.view.layers", Panel::Layers),
    ("ui.view.log", Panel::Log),
    ("ui.view.meshMapBakers", Panel::MeshMapBakers),
    ("ui.view.properties", Panel::Properties),
    ("ui.view.renderer", Panel::Renderer),
    ("ui.view.shader", Panel::Shader),
    ("ui.view.textureSetList", Panel::TextureSetList),
    ("ui.view.textureSetSettings", Panel::TextureSetSettings),
];

/// The panel a Window ▸ Views item toggles.
pub fn view_panel(id: &str) -> Option<Panel> {
    VIEWS.iter().find(|(v, _)| *v == id).map(|(_, p)| *p)
}

impl Panel {
    /// Whether Window ▸ Views offers the panel in `mode`, as measured on the reference app: Baking
    /// has its own panels and lacks Assets, Export, Layers, Properties, Renderer Settings, Shader
    /// Settings and Texture Set Settings; Rendering keeps only Display Settings, Log, Renderer
    /// Settings, Shader Settings and the texture set list.
    pub(crate) fn available_in(self, mode: crate::state::Mode) -> bool {
        use crate::state::Mode;
        match self {
            Self::MeshMapSettings | Self::CommonSettings | Self::MeshMapBakers | Self::BakingLog => mode == Mode::Baking,
            Self::Assets | Self::Layers | Self::Properties | Self::TextureSetSettings | Self::Export => mode == Mode::Painting,
            Self::History => mode != Mode::Rendering,
            Self::Shader => mode != Mode::Baking,
            Self::Renderer => mode == Mode::Rendering,
            Self::Viewport | Self::TextureSetList | Self::Display | Self::Log => true,
        }
    }

    pub const ALL: [Panel; 16] = [
        Panel::Viewport,
        Panel::Assets,
        Panel::MeshMapSettings,
        Panel::CommonSettings,
        Panel::MeshMapBakers,
        Panel::TextureSetList,
        Panel::Layers,
        Panel::TextureSetSettings,
        Panel::Properties,
        Panel::Display,
        Panel::Shader,
        Panel::History,
        Panel::Log,
        Panel::BakingLog,
        Panel::Export,
        Panel::Renderer,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Panel::Viewport => "Viewport",
            Panel::Assets => "Assets",
            Panel::MeshMapSettings => "Mesh Map Settings",
            Panel::CommonSettings => "Common Settings",
            Panel::MeshMapBakers => "Mesh Map Bakers",
            Panel::TextureSetList => "Texture Sets",
            Panel::Layers => "Layers",
            Panel::TextureSetSettings => "Texture Set Settings",
            Panel::Properties => "Properties",
            Panel::Display => "Display Settings",
            Panel::Shader => "Shader Settings",
            Panel::History => "History",
            Panel::Log => "Log",
            Panel::BakingLog => "Baking Log",
            Panel::Export => "Export",
            Panel::Renderer => "Renderer Settings",
        }
    }

    /// Where the panel floats when it has never been laid out, cascaded.
    fn float_rect(self) -> Option<Rect> {
        let slot = match self {
            Panel::Display => 0.0,
            Panel::Shader => 1.0,
            Panel::History => 2.0,
            Panel::Log => 3.0,
            Panel::Export => 4.0,
            Panel::Renderer => 5.0,
            Panel::BakingLog => 6.0,
            _ => return None,
        };
        Some(Rect::from_min_size(egui::pos2(960.0, 90.0 + 50.0 * slot), vec2(300.0, 380.0)))
    }
}

/// The saved panel arrangement: craft-behavior's workspace over TextureCraft's panels.
pub type Workspace = craft_behavior::dock::Workspace<Panel>;

/// The viewport never moves, floats, closes or takes tabs; panels dock beside it.
pub fn permissions(panel: &Panel) -> Permissions {
    if *panel == Panel::Viewport { Permissions::PROTECTED.accepts_splits(true) } else { Permissions::default() }
}

fn tabs(panels: &[Panel]) -> Node<Panel> {
    Node::tabs(panels.to_vec(), 0)
}

fn split(axis: Axis, size: SplitSize, first: Node<Panel>, second: Node<Panel>) -> Node<Panel> {
    Node::split(axis, size, first, second)
}

/// The default: assets (4 tile columns at 1920 px) left of the viewport, then texture sets, layers (tabbed with texture set settings) and properties
pub fn default_workspace() -> Workspace {
    use Panel::*;

    // The Assets split keeps four tiles visible at the reference window width
    const ASSETS_SPLIT_RATIO: f32 = 0.16;
    const VIEWPORT_SPLIT_RATIO: f32 = 0.80;
    const TEXTURE_SET_SPLIT_RATIO: f32 = 0.125;

    // Leave room for three layer rows and the fill's channel controls
    const LAYERS_SPLIT_RATIO: f32 = 0.31;

    let column = split(
        Axis::Y,
        SplitSize::Ratio(TEXTURE_SET_SPLIT_RATIO),
        tabs(&[TextureSetList]),
        split(Axis::Y, SplitSize::Ratio(LAYERS_SPLIT_RATIO), tabs(&[Layers, TextureSetSettings]), tabs(&[Properties])),
    );
    let root = split(
        Axis::X,
        SplitSize::Ratio(ASSETS_SPLIT_RATIO),
        tabs(&[Assets]),
        split(Axis::X, SplitSize::Ratio(VIEWPORT_SPLIT_RATIO), tabs(&[Viewport]), column),
    );
    let mut hidden: Vec<(Panel, Location<Panel>)> = Panel::ALL.into_iter().filter_map(|panel| Some((panel, floating_location(panel.float_rect()?)))).collect();
    for panel in [MeshMapSettings, CommonSettings, MeshMapBakers] {
        hidden.push((panel, Location::new(Some(TextureSetList), DockPlacement::tab(None))));
    }

    Workspace::from_parts(Layout::new(Some(root), Vec::new()), hidden, Vec::new())
}

/// Where a closed panel reopens as its own floating window at `rect`
fn floating_location(rect: Rect) -> Location<Panel> {
    let mut location = Location::new(None, DockPlacement::tab(None));
    location.floating = Some(rect);
    location
}

/// craft-ui's checks plus the viewport's: a saved or hand-edited file cannot hide, float, group
/// or collapse it.
pub fn validate(workspace: &Workspace) -> Result<(), String> {
    workspace.validate().map_err(|error| error.to_string())?;
    let layout = workspace.layout();
    if layout.floating.iter().any(|window| window.contains(&Panel::Viewport)) {
        return Err("The viewport cannot float".into());
    }
    if workspace.hidden().iter().any(|(panel, _)| *panel == Panel::Viewport) {
        return Err("The viewport cannot be hidden".into());
    }
    let mut found = false;
    let mut nodes: Vec<&Node<Panel>> = layout.root.iter().collect();
    while let Some(node) = nodes.pop() {
        match node {
            Node::Split { first, second, .. } => {
                nodes.push(first);
                nodes.push(second);
            }
            Node::Tabs { panels, .. } if panels.contains(&Panel::Viewport) => {
                if panels.len() != 1 {
                    return Err("The viewport cannot share a tab group".into());
                }
                found = true;
            }
            Node::Stack { entries, .. } if entries.iter().any(|entry| entry.panel == Panel::Viewport) => {
                return Err("The viewport cannot be collapsed".into());
            }
            Node::Dock { columns, .. } if columns.iter().flat_map(|column| &column.groups).any(|group| group.panels.contains(&Panel::Viewport)) => {
                return Err("The viewport cannot be in a dock".into());
            }
            _ => {}
        }
    }
    if found { Ok(()) } else { Err("The panel layout must contain the viewport".into()) }
}

// ---------------------------------------------------------------------------------------------
// The live dock

/// Height of a panel group's tab strip, in points
pub(crate) const TAB_HEIGHT: f32 = 30.0;

/// Width of the bake layout's texture set and mesh map bakers column in points: the reference's scaled to our larger font
const BAKERS_COLUMN_WIDTH: f32 = 232.0;
/// Width of the bake layout's common and mesh map settings column, in points: both tab names fit beside the ×
const SETTINGS_COLUMN_WIDTH: f32 = 380.0;
/// Height of the log under the bake layout's columns, in points
const BAKING_LOG_HEIGHT: f32 = 160.0;
/// Share of the first bake column the texture set list takes above the mesh map bakers
const TEXTURE_SET_LIST_SHARE: f32 = 0.4;

/// The dock layout (part of `UiState`; serialized as a craft-behavior `Workspace`).
#[derive(Clone, Debug)]
pub struct Dock {
    workspace: Workspace,
    /// A panel to bring in front of the other floating windows on the next frame.
    raise: Option<Panel>,
    /// A saved layout could not be read, so this is the painting default in its place
    replaced_unreadable: bool,
}

impl Default for Dock {
    fn default() -> Self {
        Dock { workspace: default_workspace(), raise: None, replaced_unreadable: false }
    }
}

impl PartialEq for Dock {
    fn eq(&self, other: &Self) -> bool {
        self.workspace == other.workspace
    }
}

impl Serialize for Dock {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.workspace.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Dock {
    /// Never fails: a layout that cannot be read or fails the viewport's rules is the default.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Dock::from_value(Value::deserialize(d)?))
    }
}

impl Dock {
    /// The dock for a saved layout; the default, marked as a replacement, when it is not a valid workspace
    pub fn from_value(value: Value) -> Dock {
        let unreadable = Dock { replaced_unreadable: true, ..Dock::default() };
        let default = default_workspace();
        let loaded = serde_json::from_value::<Workspace>(value).ok();
        let Some(mut workspace) = loaded.filter(|w| validate(w).is_ok()) else { return unreadable };

        workspace.reconcile(|_| true, permissions, &default);
        if validate(&workspace).is_err() {
            return unreadable;
        }

        Dock { workspace, raise: None, replaced_unreadable: false }
    }

    /// Whether a saved layout could not be read and this dock is the painting default instead
    pub(crate) fn replaced_unreadable(&self) -> bool {
        self.replaced_unreadable
    }

    /// Texture sets over the bakers, the two settings tabs, the Baking Log under both, then the viewport
    pub(crate) fn for_baking() -> Self {
        let painting = default_workspace();
        let mut hidden = painting.hidden().to_vec();
        for panel in [Panel::Assets, Panel::Layers, Panel::TextureSetSettings, Panel::Properties] {
            if let Ok(location) = painting.layout().location(&panel) {
                hidden.push((panel, location));
            }
        }

        let bake_panels = [Panel::MeshMapSettings, Panel::CommonSettings, Panel::MeshMapBakers, Panel::BakingLog];
        hidden.retain(|(panel, _)| !bake_panels.contains(panel));

        let gutter_width = f32::from(crate::theme::GUTTER);
        let list_column = split(Axis::Y, SplitSize::Ratio(TEXTURE_SET_LIST_SHARE), tabs(&[Panel::TextureSetList]), tabs(&[Panel::MeshMapBakers]));
        let settings_column = tabs(&[Panel::CommonSettings, Panel::MeshMapSettings]);
        let columns = split(Axis::X, SplitSize::FixedFirst(BAKERS_COLUMN_WIDTH), list_column, settings_column);
        let columns_over_log = split(Axis::Y, SplitSize::FixedSecond(BAKING_LOG_HEIGHT), columns, tabs(&[Panel::BakingLog]));
        let columns_width = BAKERS_COLUMN_WIDTH + gutter_width + SETTINGS_COLUMN_WIDTH;
        let root = split(Axis::X, SplitSize::FixedFirst(columns_width), columns_over_log, tabs(&[Panel::Viewport]));
        let workspace = Workspace::from_parts(Layout::new(Some(root), Vec::new()), hidden, painting.floated().to_vec());

        Dock { workspace, raise: None, replaced_unreadable: false }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn is_open(&self, p: Panel) -> bool {
        self.workspace.layout().contains(&p)
    }

    /// Open and the front tab of its group.
    pub fn is_visible(&self, p: Panel) -> bool {
        self.workspace.layout().panel_state(&p) == PanelState::Front
    }

    /// Hide a visible panel, otherwise bring it up.
    pub fn toggle(&mut self, p: Panel) {
        if self.is_visible(p) { self.close(p) } else { self.focus(p) }
    }

    pub fn close(&mut self, p: Panel) {
        if let Err(error) = self.workspace.set_visible(p, false, None, false, permissions) {
            log::warn!("panel {p:?} was not closed: {error}");
        }
    }

    /// Open `p` (where it was when closed, else where it lives by default) and bring it to the
    /// front.
    pub fn focus(&mut self, p: Panel) {
        let mut next = self.workspace.clone();
        match next.set_visible(p, true, None, false, permissions) {
            Ok(()) if validate(&next).is_ok() => {
                self.workspace = next;
                self.raise = Some(p);
            }
            Ok(()) => log::warn!("panel {p:?} was not opened: it would break the viewport's rules"),
            Err(error) => log::warn!("panel {p:?} was not opened: {error}"),
        }
    }

    /// Open the closed `panel` as a floating window at `rect`
    pub(crate) fn float_at(&mut self, panel: Panel, rect: Rect) {
        let mut hidden = self.workspace.hidden().to_vec();
        hidden.retain(|(hidden_panel, _)| *hidden_panel != panel);
        hidden.push((panel, floating_location(rect)));
        self.workspace = Workspace::from_parts(self.workspace.layout().clone(), hidden, self.workspace.floated().to_vec());
        self.focus(panel);
    }

    /// One change from the dock area's gestures. A change that is rejected leaves the layout as
    /// it was.
    fn apply(&mut self, action: Action<Panel>) {
        let mut next = self.workspace.clone();
        match next.apply(action, false, permissions) {
            Ok(_) if validate(&next).is_ok() => self.workspace = next,
            Ok(_) => log::warn!("a panel move would break the viewport's rules"),
            Err(error) => log::warn!("a panel move was not applied: {error}"),
        }
    }
}

/// What the dock area keeps between passes: the workspace it shows and the panels' specs
#[derive(Default)]
pub(crate) struct LiveDock {
    shown: Option<Arc<Workspace>>,
    specs: Tracked<DockSpecs>,
    /// The panel whose floating window takes over the held press in the next pass, with the grab point inside the window
    continue_move: Option<(Panel, egui::Vec2)>,
    /// The OS windows of the floating panels, as the last pass's dock drew them
    floating_windows: Vec<ViewportId>,
}

impl LiveDock {
    /// The OS windows the last pass's dock drew floating panels in
    pub(crate) fn floating_windows(&self) -> &[ViewportId] {
        &self.floating_windows
    }

    /// Asks the next pass's dock to let the window holding `panel` follow the held pointer from `grab`
    pub(crate) fn hand_over(&mut self, panel: Panel, grab: egui::Vec2) {
        self.continue_move = Some((panel, grab));
    }

    /// The `Arc` the dock shows for `workspace`: the last one while the arrangement is the same, so an idle pass reads no spec
    fn shown(&mut self, workspace: &Workspace) -> Arc<Workspace> {
        if let Some(shown) = self.shown.as_ref().filter(|shown| ***shown == *workspace) {
            return Arc::clone(shown);
        }

        let shown = Arc::new(workspace.clone());
        self.shown = Some(Arc::clone(&shown));
        shown
    }
}

/// What the dock reads of the panels: their tab names, in [`Panel::ALL`] order.
#[derive(Clone, Debug, Default, PartialEq, PlainData)]
struct DockSpecs {
    labels: Vec<String>,
}

impl PanelSpecs<Panel> for DockSpecs {
    fn spec(&self, panel: &Panel) -> PanelSpec {
        let label = Panel::ALL.iter().position(|p| p == panel).and_then(|index| self.labels.get(index)).cloned().unwrap_or_default();
        // Every panel body is TextureCraft's own egui code: the dock runs it with the window's style.
        let spec = PanelSpec::new(label).limits(SizeLimits::at_least(min_size(*panel))).float_size(FLOATING_SIZE.max(min_size(*panel))).host_body();
        if *panel == Panel::Viewport { spec.canvas() } else { spec }
    }

    fn permissions(&self, panel: &Panel) -> Permissions {
        permissions(panel)
    }
}

/// The tab name of `panel` in the interface language.
fn label(app: &TextureCraftApp, panel: Panel) -> String {
    match panel {
        Panel::Properties => crate::dock::properties_title(app),
        p => tr(p.name()).to_string(),
    }
}

/// Reads the tab names, moving the specs' revision only when one changed.
fn refresh_specs(app: &mut TextureCraftApp) {
    let labels: Vec<String> = Panel::ALL.iter().map(|panel| label(app, *panel)).collect();
    if app.docks.specs.labels != labels {
        app.docks.specs.labels = labels;
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing

/// From a panel's left edge to its contents' text (an empty panel's note)
pub(crate) const HEADER_TITLE_LEFT: f32 = 12.0;

/// Whether one of TextureCraft's own modal layers or the update dialogs hold the input, so the
/// panels floated into windows of their own do not take it (`FrameInput::app_modal`).
pub(crate) fn input_blocked(app: &TextureCraftApp, context: &egui::Context) -> bool {
    let modal = context.memory(|memory| memory.top_modal_layer()).is_some();
    #[cfg(not(target_arch = "wasm32"))]
    let modal = modal || crate::updates_ui::blocks_input(app);
    #[cfg(target_arch = "wasm32")]
    let _ = app;
    modal
}

/// Draw the dock (viewport and panels) into `ui`
pub fn show(app: &mut TextureCraftApp, ui: &mut Ui) {
    // Rendering keeps disabled panels visible because it shares the painting dock
    let dock_mode = if app.ui.mode == crate::state::Mode::Rendering { crate::state::Mode::Painting } else { app.ui.mode };
    for panel in Panel::ALL {
        let kept = panel.available_in(dock_mode) || (panel == Panel::Renderer && app.ui.mode == crate::state::Mode::Rendering);
        if !kept && app.ui.dock.is_open(panel) {
            app.ui.dock.close(panel);
        }
    }

    let context = ui.ctx().clone();
    refresh_specs(app);
    let shown = app.docks.shown(&app.ui.dock.workspace);

    // Move the specs out so the dock can borrow the app to draw panels
    let specs = std::mem::take(&mut app.docks.specs);
    let hand_over = app.docks.continue_move.take();
    let raise = app.ui.dock.raise.take();

    // Keep drag targets on the dock so a floating title bar only moves its window
    let config = DockConfig::default().new_dock_zones(false).title_bar_docks(false).tab_close_buttons(true);
    let (output, focused_windows) = {
        let tokens = Tokens::get(&context);
        let mut panels = Panels { app: &mut *app, tokens, layout: shown.layout().clone(), focused_windows: Vec::new() };
        let mut area = DockArea::new("Panels", Bind::given(Arc::clone(&shown)), &specs, &mut panels, config);
        if let Some(hand_over) = hand_over {
            area = area.continue_move(hand_over);
        }
        if let Some(panel) = raise {
            area = area.raise(panel);
        }
        let output = area.show(ui);
        (output, panels.focused_windows)
    };

    app.docks.specs = specs;
    app.docks.floating_windows.clear();
    app.docks.floating_windows.extend(output.windows.iter().map(|(_, viewport)| *viewport));

    // The popup needs the viewport the dock recorded this frame
    crate::rail_popup::show(app, &context);

    // Keys pressed in a panel floated into a window of its own run the app's shortcuts there
    #[cfg(not(target_arch = "wasm32"))]
    for (_, viewport) in &output.windows {
        let (events, held) = forwarded_events(&context, *viewport, &focused_windows);
        crate::menus::forwarded_shortcuts(app, &context, events, held);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = focused_windows;

    if let Some(error) = &output.error {
        log::warn!("docking: {:?}", error.error);
    }

    // A layout changed by a panel takes priority over gestures from the old layout
    if app.ui.dock.workspace != *shown {
        craft_ui::runtime::drag::cancel(&context, craft_behavior::CancelReason::HostChanged);
        return;
    }

    for action in output.actions.into_iter().chain(output.requested) {
        app.ui.dock.apply(action);
    }
}

/// Records in `focused_windows` whether a widget of the running pass's window has the keyboard, after one of the window's
/// panels drew
fn note_focus(context: &egui::Context, focused_windows: &mut Vec<ViewportId>) {
    let window = context.viewport_id();
    focused_windows.retain(|focused| *focused != window);
    if context.memory(|memory| memory.focused().is_some()) {
        focused_windows.push(window);
    }
}

/// The keys, text and clipboard events `window` hands to the app's shortcuts, with the modifiers held there: nothing
/// while one of its widgets has the keyboard
#[cfg(not(target_arch = "wasm32"))]
fn forwarded_events(context: &egui::Context, window: ViewportId, focused_windows: &[ViewportId]) -> (Vec<egui::Event>, egui::Modifiers) {
    use egui::Event;
    let focused = focused_windows.contains(&window);
    let forwarded = |event: &&Event| !focused && matches!(event, Event::Key { .. } | Event::Text(_) | Event::Copy | Event::Cut | Event::Paste(_));
    context.input_for(window, |input| (input.events.iter().filter(forwarded).cloned().collect(), input.modifiers))
}

/// The panels' contents for the dock area
struct Panels<'a> {
    app: &'a mut TextureCraftApp,
    tokens: Tokens,
    /// The arrangement the dock shows in this pass
    layout: Layout<Panel>,
    /// The windows in which a widget had the keyboard once their panels drew
    focused_windows: Vec<ViewportId>,
}

/// Room between a panel's edges and its contents
const PANEL_BODY_MARGIN: Margin = Margin::symmetric(8, 4);

/// Narrowest Properties panel and so its dock column: the 300 point design width plus the solid scroll bar; narrower, the 3D projection settings' X, Y, Z fields (the widest row) overlap
pub const PROPERTIES_MIN_WIDTH: f32 = 310.0;

/// Least size of a docked or floating panel other than the 3D view and Properties
const PANEL_MIN_SIZE: Vec2 = vec2(120.0, 60.0);

/// Least size of the 3D view
const VIEWPORT_MIN_SIZE: Vec2 = vec2(200.0, 150.0);

/// Size a panel opens at when it floats
const FLOATING_SIZE: Vec2 = vec2(300.0, 380.0);

/// Least size of `panel`, docked or floating; a dock column or split stops shrinking at its panels' largest
fn min_size(panel: Panel) -> Vec2 {
    match panel {
        Panel::Viewport => VIEWPORT_MIN_SIZE,
        Panel::Properties => vec2(PROPERTIES_MIN_WIDTH, PANEL_MIN_SIZE.y),
        _ => PANEL_MIN_SIZE,
    }
}

/// The body of every panel except the viewport, in the dock or on the rail
pub(crate) fn panel_body(app: &mut TextureCraftApp, ui: &mut Ui, panel: Panel) {
    // No project open: the library and the log work, the other panels about a project stay empty
    let works_empty = matches!(panel, Panel::Assets | Panel::Log | Panel::Layers | Panel::TextureSetList | Panel::Properties | Panel::Export);
    if !app.session.project_open && !works_empty {
        return;
    }

    // Properties draws full-width bands and tabs, so it keeps its own margins
    let margin = if panel == Panel::Properties { Margin::ZERO } else { PANEL_BODY_MARGIN };
    egui::Frame::new().inner_margin(margin).show(ui, |ui| match panel {
        Panel::MeshMapSettings => crate::bake_mode::mesh_map_settings_panel(app, ui),
        Panel::Assets => crate::shelf::body(app, ui),
        Panel::CommonSettings => crate::bake_mode::common_settings_panel(app, ui),
        Panel::MeshMapBakers => crate::bake_mode::mesh_map_bakers_panel(app, ui),
        Panel::TextureSetList if app.ui.mode == crate::state::Mode::Baking => crate::bake_mode::texture_sets(app, ui),
        Panel::TextureSetList => crate::dock::texture_set_list(app, ui),
        Panel::Layers => crate::dock::layers(app, ui),
        Panel::TextureSetSettings => crate::dock::texture_set_settings(app, ui),
        Panel::Properties => crate::dock::properties(app, ui),

        // Display and Shader Settings scroll one page under their section tabs; History and Log scroll their own lists
        Panel::Display => crate::display_settings::body(app, ui),
        Panel::Shader => crate::shader_settings::body(app, ui),
        Panel::History => crate::windows::history(app, ui),
        Panel::Log => crate::log_panel::body(app, ui),
        Panel::BakingLog => crate::baking_log::body(app, ui),
        Panel::Export => crate::export_panel::body(app, ui),
        Panel::Renderer => crate::render_mode::settings_body(app, ui),
        Panel::Viewport => {}
    });
}

impl DockPanels<Panel> for Panels<'_> {
    fn body(&mut self, panel: &Panel, ui: &mut Ui, _: &mut BodyCx<'_>) {
        // The viewport is its group's `group_view`
        if *panel != Panel::Viewport {
            panel_body(self.app, ui, *panel);
        }
        note_focus(ui.ctx(), &mut self.focused_windows);
    }

    /// The viewport, edge to edge in its group.
    fn group_view(&mut self, panels: &[Panel], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        let GroupAt::Show(rect) = at else { return None };
        if !panels.contains(&Panel::Viewport) {
            return None;
        }
        crate::rail_popup::remember_viewport(ui.ctx(), ui.max_rect());
        ui.painter().rect_filled(ui.max_rect(), 0.0, self.tokens.viewport);
        crate::viewport::central(self.app, ui);
        Some(rect.height())
    }

    /// A rail panel floating on its own carries the Dock button that sends it back to the rail
    fn group_header(&mut self, panels: &[Panel], active: usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let Some(active) = panels.get(active).copied() else { return };
        let floating = self.layout.floating.iter().any(|window| window.contains(&active));
        if redocks(panels, &active, floating)
            && crate::widgets::icon_button(ui, egui_phosphor::regular::ARROW_SQUARE_IN, tr("Dock"), false, DOCK_BUTTON_SIZE).clicked()
        {
            self.app.ui.dock.close(active);
        }
    }
}

/// Size of the Dock button in a floating rail panel's header
const DOCK_BUTTON_SIZE: f32 = 22.0;

/// Whether a floating window holds one rail panel alone: its Dock button closes it back onto the rail
fn redocks(panels: &[Panel], active: &Panel, floating: bool) -> bool {
    floating && panels.len() == 1 && crate::rail_popup::is_rail_panel(*active)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panels_of(w: &Workspace) -> Vec<Panel> {
        w.layout().panels().into_iter().copied().collect()
    }

    /// What a floating panel's own window with a text field, focused when `typing`, hands on of `events`
    #[cfg(not(target_arch = "wasm32"))]
    fn handed_on(typing: bool, events: &[egui::Event]) -> Vec<egui::Event> {
        use std::{cell::RefCell, rc::Rc};

        let window = ViewportId::from_hash_of("floating panel");
        let context = egui::Context::default();
        context.set_embed_viewports(false);

        // Runs the window's pass inside its parent's, as eframe does
        let arriving = Rc::new(RefCell::new(Vec::new()));
        let queued = Rc::clone(&arriving);
        egui::Context::set_immediate_viewport_renderer(move |context, viewport| {
            let mut input = egui::RawInput { viewport_id: viewport.ids.this, events: queued.take(), ..Default::default() };
            input.viewports.insert(viewport.ids.this, egui::ViewportInfo { parent: Some(viewport.ids.parent), ..Default::default() });
            let mut output = context.run_ui(input, viewport.viewport_ui_cb);
            output.textures_delta.clear();
        });

        let mut name = String::new();
        let mut handed_on = Vec::new();
        // The field takes the keyboard a pass before the keys arrive
        for arrived in [Vec::new(), events.to_vec()] {
            *arriving.borrow_mut() = arrived;
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                let mut focused_windows = Vec::new();
                ui.ctx().show_viewport_immediate(window, egui::ViewportBuilder::default(), |ui, _| {
                    let field = ui.text_edit_singleline(&mut name);
                    if typing {
                        field.request_focus();
                    }
                    note_focus(ui.ctx(), &mut focused_windows);
                });
                handed_on.extend(forwarded_events(ui.ctx(), window, &focused_windows).0);
            });
            output.textures_delta.clear();
        }

        assert_eq!(name.is_empty(), !typing, "a focused field types the text");
        handed_on
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_floating_panel_hands_on_its_keys_unless_a_field_there_has_the_keyboard() {
        use egui::{Event, Key, Modifiers};

        let key = Event::Key { key: Key::B, physical_key: Some(Key::B), pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let keys = [key, Event::Text("b".into()), Event::Paste("pasted".into())];
        assert!(handed_on(true, &keys).is_empty(), "the field's keys are its own");
        assert_eq!(handed_on(false, &keys), keys, "with nothing focused they are the app's");
    }

    #[test]
    fn default_workspace_has_every_panel_and_is_valid() {
        let workspace = default_workspace();

        assert!(validate(&workspace).is_ok());

        for panel in Panel::ALL {
            assert!(
                workspace.layout().contains(&panel) || workspace.hidden().iter().any(|(hidden, _)| *hidden == panel),
                "{panel:?} is neither shown nor hidden"
            );
        }

        assert_eq!(panels_of(&workspace), [Panel::Assets, Panel::Viewport, Panel::TextureSetList, Panel::Layers, Panel::TextureSetSettings, Panel::Properties]);
    }

    #[test]
    fn a_saved_split_keeps_its_own_share() {
        let default = default_workspace();
        let mut layout = default.layout().clone();
        let Some(Node::Split { second: right, .. }) = layout.root.as_mut() else { panic!("assets | rest") };
        let Node::Split { second: column, .. } = right.as_mut() else { panic!("viewport | right column") };
        let Node::Split { second: below_sets, .. } = column.as_mut() else { panic!("texture sets | rest") };
        let Node::Split { size, .. } = below_sets.as_mut() else { panic!("layers | properties") };
        *size = SplitSize::Ratio(0.57);
        let workspace = Workspace::from_parts(layout, default.hidden().to_vec(), default.floated().to_vec());
        let back = Dock::from_value(serde_json::to_value(&workspace).unwrap());

        assert_eq!(back.workspace(), &workspace, "a layout saved with the old split opens unchanged");
    }

    #[test]
    fn the_bake_dock_round_trips_with_the_baking_log_under_both_settings_columns() {
        let dock = Dock::for_baking();
        assert!(validate(dock.workspace()).is_ok());
        assert_eq!(
            panels_of(dock.workspace()),
            [Panel::TextureSetList, Panel::MeshMapBakers, Panel::CommonSettings, Panel::MeshMapSettings, Panel::BakingLog, Panel::Viewport]
        );
        assert!(dock.is_visible(Panel::CommonSettings) && !dock.is_visible(Panel::MeshMapSettings));

        let back: Dock = serde_json::from_str(&serde_json::to_string(&dock).unwrap()).unwrap();

        assert_eq!(back, dock);
    }

    #[test]
    fn unreadable_layouts_become_the_default() {
        for text in [
            // A layout in another shape.
            r#"{"main":{"tabs":{"tabs":["Viewport"]}},"windows":[]}"#,
            // Not a layout.
            r#"[1,2]"#,
            r#""dock""#,
            // A shared workspace with a panel this build does not know.
            r#"{"version":2,"layout":{"root":{"Tabs":{"panels":["Bogus"],"active":0}},"floating":[]}}"#,
            // A shared workspace from a newer format.
            r#"{"version":99,"layout":{"root":{"Tabs":{"panels":["Viewport"],"active":0}},"floating":[]}}"#,
            // A viewport that floats.
            r#"{"version":2,"layout":{"root":{"Tabs":{"panels":["Assets"],"active":0}},"floating":[{"panels":["Viewport"],"active":0,"rect":[0,0,200,200]}]}}"#,
        ] {
            let dock: Dock = serde_json::from_str(text).unwrap();
            assert_eq!(dock, Dock::default(), "{text}");
        }
    }

    #[test]
    fn a_layout_saved_before_the_export_panel_still_loads() {
        let mut saved = serde_json::to_value(default_workspace()).unwrap();
        let hidden = saved["hidden"].as_array_mut().unwrap();
        hidden.retain(|entry| entry[0] != "Export");
        assert!(!saved.to_string().contains("\"Export\""));

        let dock = Dock::from_value(saved);
        assert!(!dock.replaced_unreadable(), "the saved layout is kept");
        assert!(dock.workspace().hidden().iter().any(|(panel, _)| *panel == Panel::Export), "Export joins it, closed");
    }

    #[test]
    fn a_focused_panel_is_raised_for_one_frame() {
        let session = texturecraft_engine::sample_session(128, 32).unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 1200.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|cc| TextureCraftApp::new(cc, session));
        let dock = &mut harness.state_mut().ui.dock;
        dock.focus(Panel::Display);
        assert_eq!(dock.raise, Some(Panel::Display));

        harness.run_steps(4);

        assert_eq!(harness.state().ui.dock.raise, None);
    }

    #[test]
    fn the_viewport_stays_put() {
        let mut dock = Dock::default();
        dock.close(Panel::Viewport);
        assert!(dock.is_open(Panel::Viewport));
        let before = dock.clone();
        dock.apply(Action::float(Panel::Viewport, Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 300.0))));
        dock.apply(Action::move_beside(Panel::Layers, Panel::Viewport, DockPlacement::tab(None)));
        assert_eq!(dock, before);
    }

    #[test]
    fn closed_panels_come_back_where_they_were() {
        let mut dock = Dock::default();
        let before = dock.clone();
        for p in [Panel::Properties, Panel::Layers, Panel::Assets] {
            dock.close(p);
            assert!(!dock.is_open(p));
            // The places persist with the layout.
            dock = serde_json::from_str(&serde_json::to_string(&dock).unwrap()).unwrap();
            dock.focus(p);
            assert!(dock.is_visible(p), "{p:?}");
            assert_eq!(dock, before, "{p:?} came back elsewhere");
        }
    }

    #[test]
    fn closed_panels_without_a_place_open_floating() {
        let mut dock = Dock::default();
        dock.focus(Panel::Display);
        assert!(dock.is_visible(Panel::Display));
        assert_eq!(dock.workspace().layout().floating.len(), 1);
        assert!(dock.workspace().layout().floating[0].contains(&Panel::Display));
        // Hiding and showing the views one by one.
        dock.toggle(Panel::Display);
        assert!(!dock.is_open(Panel::Display));
        dock.toggle(Panel::TextureSetSettings);
        assert!(dock.is_visible(Panel::TextureSetSettings) && !dock.is_visible(Panel::Layers));
    }
}
