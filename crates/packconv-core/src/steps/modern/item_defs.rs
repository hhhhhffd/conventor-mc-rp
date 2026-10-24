//! Conversion between the pre-1.21.4 model `overrides` and 1.21.4 `items/<id>.json` definitions.
//!
//! Old semantics: overrides are tried in order and the LAST one whose predicates all match wins;
//! a predicate `name: v` matches when the item's value for `name` is `>= v`; with no match the
//! model itself is used.
//!
//! Up lowering builds a decision tree: discrete predicates become `condition` / `select` nodes,
//! numeric ones `range_dispatch` nodes, split one predicate at a time. Down flattening walks the
//! tree and emits one override per leaf, in an order that reproduces "last match wins".

use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

/// One override with trivially-true predicates removed.
#[derive(Clone)]
struct Entry {
    preds: Vec<(String, f64)>,
    model: String,
}

impl Entry {
    fn value(&self, name: &str) -> Option<f64> {
        self.preds.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }

    fn without(&self, names: &[&str]) -> Entry {
        Entry {
            preds: self.preds.iter().filter(|(n, _)| !names.contains(&n.as_str())).cloned().collect(),
            model: self.model.clone(),
        }
    }
}

/// Predicates that map to `condition` nodes: (old name, new property).
const CONDITIONS: &[(&str, &str)] = &[
    ("pulling", "minecraft:using_item"),
    ("blocking", "minecraft:using_item"),
    ("throwing", "minecraft:using_item"),
    ("tooting", "minecraft:using_item"),
    ("cast", "minecraft:fishing_rod/cast"),
    ("broken", "minecraft:broken"),
    ("damaged", "minecraft:damaged"),
];

/// Predicates that map to `range_dispatch` nodes. `pull` and `angle` depend on the item.
const NUMERIC: &[&str] = &["pull", "damage", "cooldown", "custom_model_data", "angle", "time", "filled"];

fn normalize_id(id: &str) -> String {
    if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") }
}

fn strip_minecraft(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

pub fn same_model(a: &str, b: &str) -> bool {
    normalize_id(a) == normalize_id(b)
}

/// Whole numbers are written as JSON integers, the rest as floats.
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 { Value::from(v as i64) } else { json!(v) }
}

fn model_node(model: &str) -> Value {
    json!({"type": "minecraft:model", "model": normalize_id(model)})
}

// ---------------------------------------------------------------- up

/// Builds the `items/<id>.json` content for a model with `overrides`. `base_model` is the id of
/// the model file itself. Unsupported predicates are reported through `notes` (once per kind by
/// the caller) and cause their override to be dropped.
pub fn overrides_to_definition(item: &str, base_model: &str, overrides: &[Value], notes: &mut BTreeSet<String>) -> Value {
    let mut entries = Vec::new();
    'overrides: for o in overrides {
        let Some(model) = o.get("model").and_then(Value::as_str) else { continue };
        let mut preds = Vec::new();
        for (name, value) in o.get("predicate").and_then(Value::as_object).into_iter().flatten() {
            let name = strip_minecraft(name);
            let known = CONDITIONS.iter().any(|(n, _)| *n == name) || NUMERIC.contains(&name) || name == "charged" || name == "firework";
            let numeric = match value {
                Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                other => other.as_f64(),
            };
            let (true, Some(v)) = (known, numeric) else {
                notes.insert(format!("override predicate \"{name}\" has no 1.21.4 item definition equivalent; those overrides were dropped"));
                continue 'overrides;
            };
            // `value >= v` always holds for non-positive v (all these values are >= 0).
            if v > 0.0 {
                preds.push((name.to_string(), v));
            }
        }
        entries.push(Entry { preds, model: model.to_string() });
    }
    json!({"model": build(item, &entries, &model_node(base_model))})
}

/// Splits on the first predicate present (discrete before numeric) and recurses into each
/// region of its value, until only override-free entries remain: then the last one wins.
fn build(item: &str, entries: &[Entry], fallback: &Value) -> Value {
    let has = |name: &str| entries.iter().any(|e| e.value(name).is_some());
    if let Some((name, property)) = CONDITIONS.iter().find(|(n, _)| has(n)) {
        // A loaded crossbow is never being pulled, so charge overrides only apply while not "pulling".
        let loaded = |e: &Entry| e.value("charged").is_some() || e.value("firework").is_some();
        let on_true: Vec<Entry> =
            entries.iter().filter(|e| *name != "pulling" || !loaded(e)).map(|e| e.without(&[*name])).collect();
        let on_false: Vec<Entry> = entries.iter().filter(|e| e.value(name).is_none()).cloned().collect();
        let (on_true, on_false) = (build(item, &on_true, fallback), build(item, &on_false, fallback));
        if on_true == on_false {
            return on_true;
        }
        return json!({"type": "minecraft:condition", "property": property, "on_true": on_true, "on_false": on_false});
    }
    if has("charged") || has("firework") {
        return build_charge(item, entries, fallback);
    }
    if let Some(name) = NUMERIC.iter().find(|n| has(n)) {
        return build_numeric(item, name, entries, fallback);
    }
    entries.last().map_or_else(|| fallback.clone(), |e| model_node(&e.model))
}

/// `charged` (loaded) and `firework` (loaded with a rocket) are the states of `charge_type`.
fn build_charge(item: &str, entries: &[Entry], fallback: &Value) -> Value {
    let arrow: Vec<Entry> = entries.iter().filter(|e| e.value("firework").is_none()).map(|e| e.without(&["charged"])).collect();
    let rocket: Vec<Entry> = entries.iter().map(|e| e.without(&["charged", "firework"])).collect();
    let none: Vec<Entry> = entries.iter().filter(|e| e.value("charged").is_none() && e.value("firework").is_none()).cloned().collect();
    json!({
        "type": "minecraft:select",
        "property": "minecraft:charge_type",
        "cases": [
            {"when": "arrow", "model": build(item, &arrow, fallback)},
            {"when": "rocket", "model": build(item, &rocket, fallback)},
        ],
        "fallback": build(item, &none, fallback),
    })
}

fn build_numeric(item: &str, name: &str, entries: &[Entry], fallback: &Value) -> Value {
    let mut thresholds: Vec<f64> = entries.iter().filter_map(|e| e.value(name)).collect();
    thresholds.sort_by(f64::total_cmp);
    thresholds.dedup();

    // A region is the value range starting at `threshold`; the entries active there are those
    // without the predicate or with a threshold at or below the region start.
    let region = |threshold: Option<f64>| {
        let active: Vec<Entry> = entries
            .iter()
            .filter(|e| match (e.value(name), threshold) {
                (None, _) => true,
                (Some(v), Some(t)) => v <= t,
                (Some(_), None) => false,
            })
            .map(|e| e.without(&[name]))
            .collect();
        build(item, &active, fallback)
    };
    let below_all = region(None);
    let mut dispatch_entries: Vec<Value> = Vec::new();
    let mut previous = below_all.clone();
    for t in thresholds {
        let node = region(Some(t));
        if node != previous {
            dispatch_entries.push(json!({"threshold": number(t), "model": node.clone()}));
            previous = node;
        }
    }
    if dispatch_entries.is_empty() {
        return below_all;
    }
    let dispatch = |extra: Map<String, Value>| {
        let mut node = Map::new();
        node.insert("type".into(), "minecraft:range_dispatch".into());
        node.extend(extra);
        node.insert("entries".into(), Value::Array(dispatch_entries.clone()));
        node.insert("fallback".into(), below_all.clone());
        Value::Object(node)
    };
    let props = |pairs: &[(&str, Value)]| pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Map<_, _>>();
    match name {
        // Bow pull is ticks used / 20, i.e. `use_duration` scaled by 1/20.
        "pull" if item == "crossbow" => dispatch(props(&[("property", "minecraft:crossbow/pull".into())])),
        "pull" => dispatch(props(&[("property", "minecraft:use_duration".into()), ("scale", json!(0.05))])),
        "damage" => dispatch(props(&[("property", "minecraft:damage".into())])),
        "cooldown" => dispatch(props(&[("property", "minecraft:cooldown".into())])),
        "custom_model_data" => dispatch(props(&[("property", "minecraft:custom_model_data".into())])),
        "time" => dispatch(props(&[("property", "minecraft:time".into()), ("source", "daytime".into())])),
        "filled" => dispatch(props(&[("property", "minecraft:bundle/fullness".into())])),
        "angle" => {
            let compass = |target: &str| dispatch(props(&[("property", "minecraft:compass".into()), ("target", target.into())]));
            match item {
                "recovery_compass" => compass("recovery"),
                // The vanilla compass points at its lodestone when it has one, else at the world spawn.
                "compass" => json!({
                    "type": "minecraft:condition",
                    "property": "minecraft:has_component",
                    "component": "minecraft:lodestone_tracker",
                    "on_true": compass("lodestone"),
                    "on_false": compass("spawn"),
                }),
                _ => compass("spawn"),
            }
        }
        _ => unreachable!("numeric predicate table and match are in sync"),
    }
}

// ---------------------------------------------------------------- down

pub struct Flattened {
    /// Model used when no override matches.
    pub base: String,
    pub overrides: Vec<Value>,
}

type Preds = Vec<(String, f64)>;

/// Turns an item definition back into a base model plus `overrides`. Returns `None` (with a
/// note) when the definition uses constructs that have no override equivalent.
pub fn definition_to_overrides(item: &str, definition: &Value, notes: &mut BTreeSet<String>) -> Option<Flattened> {
    let mut leaves: Vec<(Preds, String)> = Vec::new();
    let root = definition.get("model")?;
    if !flatten(item, root, &mut Vec::new(), &mut leaves, notes) {
        return None;
    }
    let Some((_, base)) = leaves.iter().rev().find(|(preds, _)| preds.is_empty()) else {
        notes.insert("item definition without an unconditional model cannot be expressed as overrides".into());
        return None;
    };
    let overrides = leaves
        .iter()
        .filter(|(preds, _)| !preds.is_empty())
        .map(|(preds, model)| {
            let predicate: Map<String, Value> = preds.iter().map(|(n, v)| (n.clone(), number(*v))).collect();
            json!({"predicate": predicate, "model": model})
        })
        .collect();
    Some(Flattened { base: base.clone(), overrides })
}

fn unsupported(notes: &mut BTreeSet<String>, what: &str) -> bool {
    notes.insert(format!("item definition construct \"{what}\" has no override equivalent; those items were not converted"));
    false
}

fn flatten(item: &str, node: &Value, preds: &mut Preds, leaves: &mut Vec<(Preds, String)>, notes: &mut BTreeSet<String>) -> bool {
    let kind = node.get("type").and_then(Value::as_str).map_or("", strip_minecraft);
    let property = node.get("property").and_then(Value::as_str).map_or("", strip_minecraft);
    let child = |key: &str| node.get(key);
    match kind {
        "model" => {
            let Some(model) = node.get("model").and_then(Value::as_str) else { return unsupported(notes, "model without id") };
            if node.get("tints").is_some() {
                notes.insert("item model tints were dropped".into());
            }
            leaves.push((preds.clone(), model.to_string()));
            true
        }
        "condition" => {
            let name = match property {
                "using_item" => match item {
                    "shield" => "blocking",
                    "trident" => "throwing",
                    "goat_horn" => "tooting",
                    _ => "pulling",
                },
                "fishing_rod/cast" => "cast",
                "broken" => "broken",
                "damaged" => "damaged",
                // The old compass `angle` covers lodestone tracking itself.
                "has_component" | "component" if node.get("component").and_then(Value::as_str).map(strip_minecraft) == Some("lodestone_tracker") => {
                    return child("on_false").is_some_and(|n| flatten(item, n, preds, leaves, notes));
                }
                other => return unsupported(notes, &format!("condition {other}")),
            };
            let (Some(on_true), Some(on_false)) = (child("on_true"), child("on_false")) else { return unsupported(notes, "condition without branches") };
            // The false branch comes first: its overrides also match while true, and must lose.
            if !flatten(item, on_false, preds, leaves, notes) {
                return false;
            }
            preds.push((name.to_string(), 1.0));
            let ok = flatten(item, on_true, preds, leaves, notes);
            preds.pop();
            ok
        }
        "range_dispatch" => {
            let scale = node.get("scale").and_then(Value::as_f64).filter(|s| *s != 0.0).unwrap_or(1.0);
            let name = match property {
                // Ticks / 20 is the old pull value; vanilla's scale of 0.05 makes thresholds equal.
                "use_duration" => "pull",
                "crossbow/pull" => "pull",
                "damage" if node.get("normalize").and_then(Value::as_bool).unwrap_or(true) => "damage",
                "cooldown" => "cooldown",
                "custom_model_data" if node.get("index").and_then(Value::as_u64).unwrap_or(0) == 0 => "custom_model_data",
                "compass" => "angle",
                "time" => "time",
                "bundle/fullness" => "filled",
                other => return unsupported(notes, &format!("range_dispatch {other}")),
            };
            let per_unit = if property == "use_duration" { 20.0 } else { 1.0 };
            if let Some(fallback) = child("fallback") {
                if !flatten(item, fallback, preds, leaves, notes) {
                    return false;
                }
            }
            let mut entries: Vec<(f64, &Value)> = node
                .get("entries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| Some((e.get("threshold")?.as_f64()?, e.get("model")?)))
                .collect();
            entries.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (threshold, model) in entries {
                preds.push((name.to_string(), threshold / scale / per_unit));
                let ok = flatten(item, model, preds, leaves, notes);
                preds.pop();
                if !ok {
                    return false;
                }
            }
            true
        }
        "select" if property == "charge_type" => {
            if let Some(fallback) = child("fallback") {
                if !flatten(item, fallback, preds, leaves, notes) {
                    return false;
                }
            }
            for case in node.get("cases").and_then(Value::as_array).into_iter().flatten() {
                let (Some(when), Some(model)) = (case.get("when").and_then(Value::as_str), case.get("model")) else { continue };
                let added: &[&str] = match strip_minecraft(when) {
                    "arrow" => &["charged"],
                    "rocket" => &["charged", "firework"],
                    _ => continue,
                };
                preds.extend(added.iter().map(|n| (n.to_string(), 1.0)));
                let ok = flatten(item, model, preds, leaves, notes);
                preds.truncate(preds.len() - added.len());
                if !ok {
                    return false;
                }
            }
            true
        }
        other => {
            let what = if property.is_empty() { other.to_string() } else { format!("{other} {property}") };
            unsupported(notes, &what)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(item: &str, overrides: Value) -> Value {
        let mut notes = BTreeSet::new();
        overrides_to_definition(item, &format!("minecraft:item/{item}"), overrides.as_array().unwrap(), &mut notes)
    }

    fn down(item: &str, def: &Value) -> Flattened {
        definition_to_overrides(item, def, &mut BTreeSet::new()).expect("expressible")
    }

    #[test]
    fn bow_matches_vanilla_definition() {
        let def = up(
            "bow",
            json!([
                {"predicate": {"pulling": 1}, "model": "item/bow_pulling_0"},
                {"predicate": {"pulling": 1, "pull": 0.65}, "model": "item/bow_pulling_1"},
                {"predicate": {"pulling": 1, "pull": 0.9}, "model": "item/bow_pulling_2"},
            ]),
        );
        let m = |id: &str| json!({"type": "minecraft:model", "model": format!("minecraft:{id}")});
        let expected = json!({"model": {
            "type": "minecraft:condition", "property": "minecraft:using_item",
            "on_true": {
                "type": "minecraft:range_dispatch", "property": "minecraft:use_duration", "scale": 0.05,
                "entries": [{"threshold": 0.65, "model": m("item/bow_pulling_1")}, {"threshold": 0.9, "model": m("item/bow_pulling_2")}],
                "fallback": m("item/bow_pulling_0"),
            },
            "on_false": m("item/bow"),
        }});
        assert_eq!(def, expected);
        let flat = down("bow", &def);
        assert_eq!(flat.base, "minecraft:item/bow");
        assert_eq!(
            Value::Array(flat.overrides),
            json!([
                {"predicate": {"pulling": 1}, "model": "minecraft:item/bow_pulling_0"},
                {"predicate": {"pulling": 1, "pull": 0.65}, "model": "minecraft:item/bow_pulling_1"},
                {"predicate": {"pulling": 1, "pull": 0.9}, "model": "minecraft:item/bow_pulling_2"},
            ])
        );
    }

    #[test]
    fn crossbow_uses_charge_type_and_crossbow_pull() {
        let def = up(
            "crossbow",
            json!([
                {"predicate": {"pulling": 1}, "model": "item/crossbow_pulling_0"},
                {"predicate": {"pulling": 1, "pull": 0.58}, "model": "item/crossbow_pulling_1"},
                {"predicate": {"pulling": 1, "pull": 1.0}, "model": "item/crossbow_pulling_2"},
                {"predicate": {"charged": 1}, "model": "item/crossbow_arrow"},
                {"predicate": {"charged": 1, "firework": 1}, "model": "item/crossbow_firework"},
            ]),
        );
        let on_false = &def["model"]["on_false"];
        assert_eq!(on_false["type"], "minecraft:select");
        assert_eq!(on_false["property"], "minecraft:charge_type");
        assert_eq!(on_false["cases"][0]["model"]["model"], "minecraft:item/crossbow_arrow");
        assert_eq!(on_false["cases"][1]["model"]["model"], "minecraft:item/crossbow_firework");
        assert_eq!(on_false["fallback"]["model"], "minecraft:item/crossbow");
        assert_eq!(def["model"]["on_true"]["property"], "minecraft:crossbow/pull");
        let flat = down("crossbow", &def);
        assert_eq!(flat.base, "minecraft:item/crossbow");
        assert_eq!(flat.overrides.len(), 5);
        assert!(flat.overrides.iter().any(|o| o["predicate"] == json!({"charged": 1, "firework": 1})));
    }

    #[test]
    fn custom_model_data_becomes_range_dispatch_and_back() {
        let def = up(
            "stick",
            json!([
                {"predicate": {"custom_model_data": 2}, "model": "item/b"},
                {"predicate": {"custom_model_data": 1}, "model": "item/a"},
            ]),
        );
        let node = &def["model"];
        assert_eq!(node["property"], "minecraft:custom_model_data");
        assert_eq!(node["fallback"]["model"], "minecraft:item/stick");
        // Old semantics: at value 2 the LAST matching override (threshold 1) wins, so 2 is not split off.
        assert_eq!(node["entries"].as_array().unwrap().len(), 1);
        assert_eq!(node["entries"][0]["threshold"], 1);
        assert_eq!(node["entries"][0]["model"]["model"], "minecraft:item/a");
        let flat = down("stick", &def);
        assert_eq!(flat.overrides.len(), 1);
        assert_eq!(flat.overrides[0]["predicate"], json!({"custom_model_data": 1}));
    }

    #[test]
    fn damage_and_unsupported_predicates() {
        let mut notes = BTreeSet::new();
        let def = overrides_to_definition(
            "sword",
            "minecraft:item/sword",
            json!([
                {"predicate": {"damage": 0.5, "damaged": 1}, "model": "item/worn"},
                {"predicate": {"mymod:glow": 1}, "model": "item/glow"},
            ])
            .as_array()
            .unwrap(),
            &mut notes,
        );
        assert_eq!(notes.len(), 1);
        assert_eq!(def["model"]["property"], "minecraft:damaged");
        assert_eq!(def["model"]["on_true"]["property"], "minecraft:damage");
    }

    #[test]
    fn inexpressible_definitions_are_reported() {
        let mut notes = BTreeSet::new();
        let def = json!({"model": {"type": "minecraft:composite", "models": []}});
        assert!(definition_to_overrides("x", &def, &mut notes).is_none());
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn plain_model_definition_has_no_overrides() {
        let def = json!({"model": {"type": "minecraft:model", "model": "mymod:item/x"}});
        let flat = down("x", &def);
        assert_eq!(flat.base, "mymod:item/x");
        assert!(flat.overrides.is_empty());
    }
}
