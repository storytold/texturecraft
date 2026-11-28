//! The tool shelf's variant menus: opening them, picking a variant, and the button following the settings

use egui::accesskit::{Role, Toggled};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable, by};
use serde_json::json;
use texturecraft_engine::Tool;
use texturecraft_engine::tool_variant::ToolVariant;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::Mode;

fn app_harness(tool: Tool) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.tool.tool = tool;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(4);
    harness
}

/// The shelf button named `name`, with or without its shortcut after it
fn shelf(name: &str) -> By<'_> {
    let named = move |label: &str| label == name || label.strip_prefix(name).is_some_and(|rest| rest.starts_with(" ("));
    by().role(Role::Button).predicate(move |node| node.label().is_some_and(|label| named(&label)))
}

fn shelf_button(harness: &Harness<'_, TextureCraftApp>, name: &str) -> Rect {
    harness.get(shelf(name)).rect()
}

/// The variant the shelf button of the tool named `tool` shows: its description for screen readers
fn shown_variant(harness: &Harness<'_, TextureCraftApp>, tool: &str) -> Option<String> {
    harness.get(shelf(tool)).accesskit_node().description()
}

fn key(harness: &mut Harness<'_, TextureCraftApp>, key: Key, modifiers: Modifiers) {
    harness.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    harness.run_steps(2);
}

/// The open menu's row named `name` has the keyboard focus
fn row_focused(harness: &Harness<'_, TextureCraftApp>, name: &str) -> bool {
    harness.get(by().role(Role::RadioButton).label(name)).is_focused()
}

/// A press and release at `position`, each in its own frame, as a hand clicks
fn click_at(harness: &mut Harness<'_, TextureCraftApp>, position: Pos2) {
    harness.hover_at(position);
    harness.run_steps(1);
    for pressed in [true, false] {
        harness.event(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        harness.run_steps(1);
    }
    harness.run_steps(2);
}

/// One frame at `seconds` past a fixed start, so a test controls how long a press lasts
fn step_at(harness: &mut Harness<'_, TextureCraftApp>, seconds: f64) {
    const START_SECONDS: f64 = 100.0;
    harness.input_mut().time = Some(START_SECONDS + seconds);
    harness.step();
}

/// The pointer moved to `position` and pressed there, at `seconds`
fn press_at(harness: &mut Harness<'_, TextureCraftApp>, position: Pos2, seconds: f64) {
    harness.hover_at(position);
    step_at(harness, seconds);
    harness.event(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    step_at(harness, seconds);
}

/// How far inside the button's lower-right corner a click lands on the arrow
const ARROW_INSET_POINTS: f32 = 3.0;

/// A click on the arrow in the button's lower-right corner
fn click_arrow(harness: &mut Harness<'_, TextureCraftApp>, button: Rect) {
    click_at(harness, button.right_bottom() - vec2(ARROW_INSET_POINTS, ARROW_INSET_POINTS));
}

/// The open menu's rows, by name, with whether each is the current variant
fn menu_rows(harness: &Harness<'_, TextureCraftApp>) -> Vec<(String, bool)> {
    let is_variant = |label: &str| ToolVariant::ALL.iter().any(|variant| variant.label() == label);
    harness
        .query_all(by().role(Role::RadioButton).predicate(move |node| node.label().is_some_and(|label| is_variant(&label))))
        .filter_map(|node| Some((node.accesskit_node().label()?, node.accesskit_node().toggled() == Some(Toggled::True))))
        .collect()
}

/// Whether a down caret is drawn inside `rect`
fn shows_caret_in(harness: &Harness<'_, TextureCraftApp>, rect: Rect) -> bool {
    harness.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == egui_phosphor::regular::CARET_DOWN && rect.contains(text.visual_bounding_rect().center()))
    })
}

#[test]
fn picking_a_row_sets_the_tool_and_its_setting() {
    let mut harness = app_harness(Tool::Eraser);
    assert!(!harness.state().session.tool.brush.particles.enabled);

    let paint = shelf_button(&harness, "Paint");
    click_arrow(&mut harness, paint);
    assert_eq!(menu_rows(&harness), [("Paint".to_owned(), true), ("Physical paint".to_owned(), false)], "the arrow opens Paint's variants");
    assert_eq!(harness.state().session.tool.tool, Tool::Eraser, "opening the menu selects nothing");

    harness.get(by().role(Role::RadioButton).label("Physical paint")).click();
    harness.run_steps(3);
    let tool = &harness.state().session.tool;
    assert_eq!(tool.tool, Tool::Paint);
    assert!(tool.brush.particles.enabled, "the physical variant sheds particles");
    assert!(menu_rows(&harness).is_empty(), "picking closes the menu");

    let path = shelf_button(&harness, "Path");
    click_arrow(&mut harness, path);
    harness.get(by().role(Role::RadioButton).label("Filled path")).click();
    harness.run_steps(3);
    let tool = &harness.state().session.tool;
    assert_eq!(tool.tool, Tool::Path);
    assert!(tool.path.closed && tool.path.fill, "a filled path is closed and filled");

    let clone = shelf_button(&harness, "Clone");
    click_arrow(&mut harness, clone);
    harness.get(by().role(Role::RadioButton).label("Clone (absolute source)")).click();
    harness.run_steps(3);
    let tool = &harness.state().session.tool;
    assert_eq!(tool.tool, Tool::Clone);
    assert!(!tool.clone_aligned, "an absolute source stays where it was set");
}

#[test]
fn plain_click_selects_without_opening() {
    let mut harness = app_harness(Tool::Eraser);

    let paint = shelf_button(&harness, "Paint");
    click_at(&mut harness, paint.center());

    assert_eq!(harness.state().session.tool.tool, Tool::Paint);
    assert!(menu_rows(&harness).is_empty(), "a click on the icon selects the tool and opens nothing");

    // The arrow toggles: a second click on it closes the menu it opened
    click_arrow(&mut harness, paint);
    assert!(!menu_rows(&harness).is_empty());
    click_arrow(&mut harness, paint);
    assert!(menu_rows(&harness).is_empty());
}

#[test]
fn plain_click_keeps_the_variant() {
    let mut harness = app_harness(Tool::Eraser);
    harness.state_mut().session.tool.brush.particles.enabled = true;
    harness.run_steps(2);

    let paint = shelf_button(&harness, "Paint");
    click_at(&mut harness, paint.center());
    assert_eq!(harness.state().session.tool.tool, Tool::Paint);
    assert!(harness.state().session.tool.brush.particles.enabled, "selecting the tool keeps its variant");

    key(&mut harness, Key::Num2, Modifiers::NONE);
    assert_eq!(harness.state().session.tool.tool, Tool::Eraser);
    assert!(harness.state().session.tool.brush.particles.enabled, "the tool's key keeps its variant too");
    assert_eq!(shown_variant(&harness, "Eraser").as_deref(), Some("Physical eraser"));
}

#[test]
fn shelf_tooltips_name_the_tool_without_its_key() {
    // Frames past the tooltip delay
    const TOOLTIP_FRAMES: usize = 60;
    let mut harness = app_harness(Tool::Paint);
    harness.state_mut().session.tool.brush.particles.enabled = true;
    harness.run_steps(2);

    // A tool with variants tips its current variant, a plain tool its name; the key stays in the button's name for screen readers
    for (tool, tooltip) in [("Paint", "Physical paint"), ("Smudge", "Smudge")] {
        let button = shelf_button(&harness, tool);
        let before = harness.query_all_by_label(tooltip).count();
        harness.hover_at(button.center());
        harness.run_steps(TOOLTIP_FRAMES);
        assert_eq!(harness.query_all_by_label(tooltip).count(), before + 1, "hovering {tool} tips {tooltip:?} with no key after it");
        assert!(harness.get(shelf(tool)).accesskit_node().label().is_some_and(|name| name.ends_with(')')), "the {tool} button's name keeps its key");
    }
}

#[test]
fn menu_follows_the_old_controls() {
    let mut harness = app_harness(Tool::Paint);

    // What Properties' particle checkbox, Clone's Aligned tick and the path's Closed and Fill buttons do
    harness.state_mut().session.tool.brush.particles.enabled = true;
    harness.state_mut().run("tool.brush", json!({"cloneAligned": false}));
    let session = &mut harness.state_mut().session;
    session.set_path_closed(true);
    session.set_path_fill(true);
    session.tool.tool = Tool::Paint;
    harness.run_steps(2);

    let shown = [
        ("Paint", "Physical paint"),
        ("Eraser", "Physical eraser"),
        ("Projection", "Physical projection"),
        ("Clone", "Clone (absolute source)"),
        ("Path", "Filled path"),
    ];
    for (tool, variant) in shown {
        assert_eq!(shown_variant(&harness, tool).as_deref(), Some(variant), "the {tool} button shows its variant");
    }
    let paint = shelf_button(&harness, "Paint");
    click_arrow(&mut harness, paint);
    assert_eq!(menu_rows(&harness), [("Paint".to_owned(), false), ("Physical paint".to_owned(), true)], "the menu ticks the physical variant");
}

/// The top bar shows a control labelled `name`: one level with the topmost `anchor`, as Properties may repeat both lower down
fn in_top_bar(harness: &Harness<'_, TextureCraftApp>, anchor: &str, name: &str) -> bool {
    let top = harness.query_all(by().label(anchor)).map(|node| node.rect().center().y).fold(f32::INFINITY, f32::min);
    assert!(top.is_finite(), "the top bar shows {anchor}");
    harness.query_all(by().label(name)).any(|node| (node.rect().center().y - top).abs() < 1.0)
}

#[test]
fn top_bar_leaves_the_variants_to_the_shelf() {
    const PARTICLES: &str = "Particle brush: particles slide down the surface";
    let mut harness = app_harness(Tool::Paint);

    // Wide enough that the top bar's strip holds every control without scrolling
    harness.set_size(vec2(2600.0, 960.0));
    for (tool, toggle) in
        [(Tool::Paint, false), (Tool::Eraser, false), (Tool::Projection, false), (Tool::Smudge, true), (Tool::Clone, true), (Tool::Path, true)]
    {
        harness.state_mut().session.tool.tool = tool;
        harness.run_steps(2);
        assert_eq!(in_top_bar(&harness, "Lazy mouse", PARTICLES), toggle, "{tool:?}: the particle toggle shows only where no variant covers it");
    }

    for closed in [false, true] {
        harness.state_mut().session.set_path_closed(closed);
        harness.run_steps(2);
        assert!(in_top_bar(&harness, "Lazy mouse", "Closed path"));
        for fill in ["Fill the path (close it first)", "Fill the closed path (lasso)"] {
            assert!(!in_top_bar(&harness, "Lazy mouse", fill), "Filled path stands for the Fill button");
        }
    }
}

#[test]
fn baking_shelf_has_no_variant_arrows() {
    let mut harness = app_harness(Tool::Paint);
    harness.state_mut().session.tool.brush.particles.enabled = true;
    harness.run_steps(2);
    let paint = shelf_button(&harness, "Paint");
    assert!(shows_caret_in(&harness, paint), "painting shows the arrow");

    click_arrow(&mut harness, paint);
    assert!(!menu_rows(&harness).is_empty());
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(3);

    let paint = shelf_button(&harness, "Paint");
    assert!(!shows_caret_in(&harness, paint), "the bake brushes have no variants");
    assert_ne!(shown_variant(&harness, "Paint").as_deref(), Some("Physical paint"), "nor show one");
    assert!(menu_rows(&harness).is_empty(), "leaving painting closes the menu");
    click_arrow(&mut harness, paint);
    assert!(menu_rows(&harness).is_empty());
}

#[test]
fn hold_and_right_click_open_the_menu() {
    let mut harness = app_harness(Tool::Eraser);
    let paint = shelf_button(&harness, "Paint").center();

    harness.event(Event::PointerButton { pos: paint, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    harness.event(Event::PointerButton { pos: paint, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    assert!(!menu_rows(&harness).is_empty(), "a right click opens the menu");
    key(&mut harness, Key::Escape, Modifiers::NONE);
    assert!(menu_rows(&harness).is_empty());

    // A press of a quarter second is still a click; the menu opens once it has lasted 0.4 s, shown from the frame after (egui sizes a new popup first)
    press_at(&mut harness, paint, 0.0);
    step_at(&mut harness, 0.25);
    assert!(menu_rows(&harness).is_empty(), "a short press is still a click");
    step_at(&mut harness, 0.5);
    step_at(&mut harness, 0.55);
    assert!(!menu_rows(&harness).is_empty(), "a held press opens the menu");
    assert_eq!(harness.state().session.tool.tool, Tool::Eraser);
}

#[test]
fn releasing_a_hold_neither_selects_nor_closes() {
    let mut harness = app_harness(Tool::Eraser);
    let paint = shelf_button(&harness, "Paint").center();
    press_at(&mut harness, paint, 0.0);
    step_at(&mut harness, 0.5);
    step_at(&mut harness, 0.55);
    assert!(!menu_rows(&harness).is_empty());

    // Released within egui's click time, so egui reports it as a click
    harness.event(Event::PointerButton { pos: paint, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    step_at(&mut harness, 0.6);
    step_at(&mut harness, 0.7);

    assert!(!menu_rows(&harness).is_empty(), "the menu stays for a row to be picked");
    assert_eq!(harness.state().session.tool.tool, Tool::Eraser, "the hold's release is not a click on the tool");
}

#[test]
fn keyboard_opens_picks_and_returns_focus() {
    let mut harness = app_harness(Tool::Paint);
    harness.get(shelf("Paint")).focus();
    harness.run_steps(2);
    assert_eq!(harness.get(shelf("Paint")).accesskit_node().data().is_expanded(), Some(false), "the button says it has a closed menu");

    key(&mut harness, Key::ArrowDown, Modifiers::ALT);
    assert!(row_focused(&harness, "Paint"), "the menu opens on the current variant");
    assert_eq!(harness.get(shelf("Paint")).accesskit_node().data().is_expanded(), Some(true));
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    assert!(row_focused(&harness, "Physical paint"));
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    assert!(row_focused(&harness, "Physical paint"), "the last row keeps the focus in the menu");
    for sideways in [Key::ArrowLeft, Key::ArrowRight] {
        key(&mut harness, sideways, Modifiers::NONE);
        assert!(row_focused(&harness, "Physical paint"), "{sideways:?} does not take the focus out of the menu");
    }

    key(&mut harness, Key::Enter, Modifiers::NONE);
    assert!(harness.state().session.tool.brush.particles.enabled, "Enter picks the focused row");
    assert!(menu_rows(&harness).is_empty());
    assert!(harness.get(shelf("Paint")).is_focused(), "the focus goes back to the button");

    key(&mut harness, Key::F10, Modifiers::SHIFT);
    assert!(row_focused(&harness, "Physical paint"));
    key(&mut harness, Key::Escape, Modifiers::NONE);
    assert!(menu_rows(&harness).is_empty());
    assert!(harness.get(shelf("Paint")).is_focused(), "Escape closes the menu back to the button");

    key(&mut harness, Key::Menu, Modifiers::NONE);
    assert!(row_focused(&harness, "Physical paint"));
    key(&mut harness, Key::Tab, Modifiers::NONE);
    assert!(menu_rows(&harness).is_empty(), "Tab leaves the menu");
    assert!(harness.get(shelf("Paint")).is_focused(), "and lands on the button, so the next Tab goes on from there");
    assert_eq!(harness.state().session.tool.tool, Tool::Paint);
}

#[test]
fn a_menu_left_behind_takes_no_focus() {
    let mut harness = app_harness(Tool::Paint);
    harness.get(shelf("Paint")).focus();
    harness.run_steps(2);
    key(&mut harness, Key::ArrowDown, Modifiers::ALT);

    let elsewhere = harness.ctx.content_rect().center();
    click_at(&mut harness, elsewhere);

    assert!(menu_rows(&harness).is_empty());
    assert!(!harness.get(shelf("Paint")).is_focused(), "a click elsewhere is not pulled back to the button");

    harness.get(shelf("Paint")).focus();
    harness.run_steps(2);
    key(&mut harness, Key::ArrowDown, Modifiers::ALT);
    for mode in [Mode::Baking, Mode::Painting] {
        harness.state_mut().ui.mode = mode;
        harness.run_steps(3);
    }

    assert!(!harness.get(shelf("Paint")).is_focused(), "coming back to painting grabs no focus");
}

#[test]
fn variant_of_an_unusable_tool_is_disabled() {
    let mut harness = app_harness(Tool::Eraser);
    assert!(texturecraft_ui_egui::menus::enabled(harness.state(), "tool.variant.physicalPaint"));
    let button = shelf_button(&harness, "Paint");
    click_arrow(&mut harness, button);
    assert!(!menu_rows(&harness).is_empty());

    // A fill layer's content takes no paint
    let session = &mut harness.state_mut().session;
    let fill = texturecraft_engine::execute(session, "layer.add.fill", &json!({})).unwrap()["layer"].as_u64().unwrap();
    texturecraft_engine::execute(session, "layer.select", &json!({"layer": fill, "target": "content"})).unwrap();
    harness.run_steps(3);

    assert!(menu_rows(&harness).is_empty(), "the menu of a tool that can no longer act closes");
    assert!(!texturecraft_ui_egui::menus::enabled(harness.state(), "tool.variant.physicalPaint"), "its variants are greyed like the tool");
    let context = harness.ctx.clone();
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "tool.variant.physicalPaint");
    assert!(!harness.state().session.tool.brush.particles.enabled, "so a key bound to one does nothing");
}

#[test]
fn filled_path_survives_apply() {
    let mut harness = app_harness(Tool::Path);
    let button = shelf_button(&harness, "Path");
    click_arrow(&mut harness, button);
    harness.get(by().role(Role::RadioButton).label("Filled path")).click();
    harness.run_steps(3);

    harness.state_mut().run("tool.path", json!({"uvPoints": [[0.3, 0.3], [0.7, 0.3], [0.7, 0.7], [0.3, 0.7]], "commit": true}));
    harness.run_steps(2);

    let button = shelf_button(&harness, "Path");
    click_arrow(&mut harness, button);
    assert_eq!(menu_rows(&harness), [("Paint along path".to_owned(), false), ("Filled path".to_owned(), true)], "Apply keeps the filled path");
}
