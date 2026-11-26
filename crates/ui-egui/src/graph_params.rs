use egui::{DragValue, Key, Slider, TextEdit, Ui, emath::Numeric};
use serde_json::{Value, json};
use texturecraft_engine::{
    compose::graphs::GraphParameter,
    core::{GraphRef, GraphValue},
};

use crate::{
    TextureCraftApp,
    dock::with,
    i18n::{tr, trf},
    theme::caption,
};

pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui, target: &Value, graph: &GraphRef) {
    let parameters = match app.session.doc.graphs.parameters(&graph.key) {
        Ok(parameters) => parameters,
        Err(_) => {
            ui.label(tr("Graph parameters are unavailable."));
            return;
        }
    };
    ui.label(caption(&trf("Graph: {name}", &[("name", &graph.name)])));
    for parameter in parameters.iter() {
        ui.push_id(&parameter.name, |ui| {
            if let Some((value, merge)) = edit(ui, parameter, parameter.value(graph)) {
                app.run("graph.parameter.set", with(target, json!({"name": parameter.name, "value": value, "merge": merge})));
            }
        });
    }
}

fn edit(ui: &mut Ui, p: &GraphParameter, current: GraphValue) -> Option<(Value, bool)> {
    let label = p.name.replace('_', " ");
    match current {
        GraphValue::Bool(mut value) => ui.checkbox(&mut value, &label).changed().then_some((json!(value), false)),
        GraphValue::Float(mut value) => number(ui, &label, &mut value, p.range).map(|merge| (json!(value), merge)),
        GraphValue::Int(mut value) => number(ui, &label, &mut value, p.range).map(|merge| (json!(value), merge)),
        GraphValue::Int2(mut value) => lanes(ui, &label, &mut value, p.range).map(|merge| (json!(value), merge)),
        GraphValue::Vector(values) if p.integer => {
            let mut value: Vec<_> = values.into_iter().map(|v| v as i32).collect();
            lanes(ui, &label, &mut value, p.range).map(|merge| (json!(value), merge))
        }
        GraphValue::Vector(mut value) => lanes(ui, &label, &mut value, p.range).map(|merge| (json!(value), merge)),
        GraphValue::Text(value) => text(ui, &label, &value).map(|value| (json!(value), false)),
    }
}

fn lanes<T: Numeric>(ui: &mut Ui, label: &str, values: &mut [T], range: Option<[f32; 2]>) -> Option<bool> {
    let mut edit = None;
    for (axis, value) in ["X", "Y", "Z", "W"].into_iter().zip(values) {
        let label = trf("{parameter} {axis}", &[("parameter", &label), ("axis", &axis)]);
        edit = edit.or(number(ui, &label, value, range));
    }
    edit
}

fn number<T: Numeric>(ui: &mut Ui, label: &str, value: &mut T, range: Option<[f32; 2]>) -> Option<bool> {
    let response = if let Some([min, max]) = range {
        let low = T::from_f64(f64::from(min).min(value.to_f64()));
        let high = T::from_f64(f64::from(max).max(value.to_f64()));
        ui.add(Slider::new(value, low..=high).text(label))
    } else {
        ui.horizontal(|ui| {
            let caption = ui.label(label);
            let speed = if T::INTEGRAL { 1.0 } else { 0.01 };
            ui.add(DragValue::new(value).speed(speed)).labelled_by(caption.id)
        })
        .inner
    };
    let id = response.id.with("changed during drag");
    let merge = response.changed().then(|| response.dragged() && ui.data_mut(|data| std::mem::replace(data.get_temp_mut_or_default::<bool>(id), true)));
    if response.drag_stopped() {
        ui.data_mut(|data| data.remove_temp::<bool>(id));
    }
    merge
}

fn text(ui: &mut Ui, label: &str, value: &str) -> Option<String> {
    let id = ui.next_auto_id().with("graph parameter draft");
    let mut draft = ui.data(|data| data.get_temp::<String>(id)).unwrap_or_else(|| value.into());
    let caption = ui.label(label);
    let response = ui.add(TextEdit::singleline(&mut draft).id(id)).labelled_by(caption.id);
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, draft));
        return None;
    }
    ui.data_mut(|data| data.remove_temp::<String>(id));
    (response.lost_focus() && draft != value && !ui.input(|input| input.key_pressed(Key::Escape))).then_some(draft)
}
