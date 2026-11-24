//! The images a project reads beyond fill sources: a colour selection's ID map, a filter's image input, the mask brush and its paths

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

const RESOLUTION: u32 = 128;

fn session() -> Session {
    Session::new_project("Resources", primitives::sphere(1.0, 32, 16), "builtin:sphere", None, RESOLUTION).unwrap()
}

/// A 4 × 4 white picture
fn white_png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

/// A red fill layer whose mask selects white on `id_map`
fn selecting_layer(session: &mut Session, id_map: &str) -> LayerId {
    let layer = LayerId(execute(session, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap()["layer"].as_u64().unwrap());
    let selection = execute(session, "mask.add.colorSelection", &Value::Null).unwrap()["effect"].as_u64().unwrap();
    execute(session, "effect.set", &json!({"effect": selection, "idMap": id_map, "colors": [[1.0, 1.0, 1.0]], "tolerance": 0.05})).unwrap();
    layer
}

/// [`selecting_layer`] with a Bevel Smooth that reads `input` as its Distance Map
fn layer_reading(session: &mut Session, id_map: &str, input: &str) -> LayerId {
    let layer = selecting_layer(session, id_map);
    let filter = execute(session, "effect.add.filter", &json!({"filter": "bevelSmooth", "target": "content"})).unwrap()["effect"].as_u64().unwrap();
    execute(session, "layer.fill.set", &json!({"effect": filter, "input": "distanceMap", "image": input})).unwrap();
    layer
}

#[test]
fn removing_unused_resources_keeps_the_images_a_selection_a_filter_input_the_mask_brush_and_a_path_read() {
    let mut session = session();
    let [id_map, input, path, brush, unused] =
        ["id.png", "input.png", "path.png", "brush.png", "unused.png"].map(|name| session.import_resource_bytes(name, white_png()).unwrap());
    let layer = layer_reading(&mut session, &id_map, &input);

    // A path on the mask is painted with one image, then the brush moves on to another
    execute(&mut session, "layer.select", &json!({"layer": layer.0, "target": "mask"})).unwrap();
    execute(&mut session, "tool.material", &json!({"maskImage": path})).unwrap();
    execute(&mut session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.8, 0.5]], "commit": true})).unwrap();
    execute(&mut session, "tool.material", &json!({"maskImage": brush})).unwrap();

    let reply = execute(&mut session, "file.removeUnused", &Value::Null).unwrap();

    assert_eq!(reply["removed"], json!([unused]), "only the image nothing reads goes");
}

#[test]
fn a_smart_material_carries_the_images_its_selection_and_filter_input_read() {
    let mut session = session();
    let id_map = session.import_resource_bytes("id.png", white_png()).unwrap();
    let input = session.import_resource_bytes("input.png", white_png()).unwrap();
    let layer = layer_reading(&mut session, &id_map, &input);
    let bytes = session.smart_material_bytes(layer, "Parts").unwrap();

    let mut other = self::session();
    other.add_smart_material_bytes(&bytes).unwrap();

    assert!(other.doc.resources.contains_key(&id_map), "the ID map comes along");
    assert!(other.doc.resources.contains_key(&input), "the filter input comes along");
}

#[test]
fn importing_the_id_map_a_selection_reads_again_redraws_its_mask() {
    let mut session = session();
    let id_map = session.import_resource_bytes("id.png", white_png()).unwrap();
    selecting_layer(&mut session, &id_map);

    assert!(red_in_the_view(&mut session), "white is selected");

    execute(&mut session, "resource.remove", &json!({"name": id_map})).unwrap();

    assert!(!red_in_the_view(&mut session), "no ID map, nothing selected");

    let imported = session.import_resource_bytes("id.png", white_png()).unwrap();

    assert_eq!(imported, id_map);
    assert!(red_in_the_view(&mut session), "the image is read again");
}

/// Whether the composite the views show is the masked layer's red in the middle of the texture
fn red_in_the_view(session: &mut Session) -> bool {
    session.update();
    let [red, green, _] = session.caches[0].composite.value(Channel::BaseColor, RESOLUTION / 2, RESOLUTION / 2);
    red > 0.85 && green < 0.15
}
