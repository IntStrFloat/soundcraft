//! Atomic, validated effect chains for sound-design agents.
use super::{CommandSpec, always, bad, track_param};
use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{INSERT_SLOTS, Insert};

const CMD: &str = "sound.effect_chain";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "sound.effect_chain",
        "Apply Sound Effect Chain",
        [],
        None,
        "{track, mode?: append|replace, effects: [{plugin, params?: {id: number}, bypass?: bool}]} — one undo; append follows the last used slot; automation on reassigned insert slots is cleared",
        always,
        apply
    )]
}

fn apply(e: &mut Engine, p: &Value) -> Result<Value> {
    let target = track_param(e, CMD, p, "track")?.ok_or_else(|| bad(CMD, "`track` required"))?;
    let mode = match p.get("mode") {
        None => "append",
        Some(Value::String(s)) if matches!(s.as_str(), "append" | "replace") => s,
        _ => return Err(bad(CMD, "`mode` must be append or replace")),
    };
    let effects = p.get("effects").and_then(Value::as_array).ok_or_else(|| bad(CMD, "`effects` must be an array"))?;
    if effects.len() > INSERT_SLOTS {
        return Err(bad(CMD, "at most 10 effects per track"));
    }
    let track = e.session().track(target).ok_or_else(|| bad(CMD, "no such track"))?;
    if !track.kind.is_audio_path() {
        return Err(bad(CMD, "choose a track with an audio path"));
    }
    let start = if mode == "replace" { 0 } else { track.mixer.inserts.iter().rposition(Option::is_some).map_or(0, |slot| slot + 1) };
    let slots: Vec<usize> = (start..INSERT_SLOTS).collect();
    if slots.len() < effects.len() {
        return Err(bad(CMD, "not enough insert slots after the existing chain; use replace or move/remove existing inserts"));
    }
    // Resolve every plugin and parameter before changing any part of the session.
    let mut inserts = Vec::with_capacity(effects.len());
    for effect in effects {
        let id = effect.get("plugin").and_then(Value::as_str).ok_or_else(|| bad(CMD, "each effect needs a `plugin` id"))?;
        let info = super::clap::plugin_info(id).ok_or_else(|| bad(CMD, format!("unknown plugin `{id}`; call list_plugins first")))?;
        if info.is_instrument {
            return Err(bad(CMD, format!("`{id}` is an instrument; use instrument commands instead")));
        }
        let mut insert = Insert::new(info.id);
        for param in info.params {
            insert.params.insert(param.id.to_owned(), param.default);
        }
        if let Some(params) = effect.get("params") {
            let params = params.as_object().ok_or_else(|| bad(CMD, "effect `params` must be an object"))?;
            for (key, value) in params {
                let param = info.param(key).ok_or_else(|| bad(CMD, format!("`{id}` has no parameter `{key}`")))?;
                let value = value
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= f64::from(param.min) && *v <= f64::from(param.max))
                    .ok_or_else(|| bad(CMD, format!("`{id}.{key}` needs a number in {}..{}", param.min, param.max)))?;
                insert.params.insert(key.clone(), param.clamp(value as f32));
            }
        }
        if let Some(bypass) = effect.get("bypass") {
            insert.bypass = bypass.as_bool().ok_or_else(|| bad(CMD, "`bypass` must be boolean"))?;
        }
        inserts.push(insert);
    }
    if inserts.is_empty() && mode == "append" {
        return Ok(json!({"track": target, "mode": mode, "effects": []}));
    }
    let track = e.session_mut().track_mut(target).ok_or_else(|| bad(CMD, "no such track"))?;
    if mode == "replace" {
        track.mixer.inserts.fill(None);
    }
    // Empty slots can still have old automation. Keep existing processors' lanes when appending,
    // but don't drive new effects with stale lanes. The u8::MAX sentinel is track trim, not an insert.
    let assigned_end = start + inserts.len();
    track.automation.retain(|lane| match lane.param {
        soundcraft_model::AutoParam::Plugin { slot, .. } if usize::from(slot) < INSERT_SLOTS => {
            mode != "replace" && !(start..assigned_end).contains(&usize::from(slot))
        }
        _ => true,
    });
    let mut applied = Vec::new();
    for (slot, insert) in slots.into_iter().zip(inserts) {
        applied.push(json!({"slot": slot, "plugin": insert.plugin, "params": insert.params, "bypass": insert.bypass}));
        if let Some(dest) = track.mixer.inserts.get_mut(slot) {
            *dest = Some(insert);
        }
    }
    Ok(json!({"track": target, "mode": mode, "effects": applied}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_is_atomic_and_one_undo_restores_the_previous_inserts() {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"name": "Sound"})).unwrap();
        let before = crate::inspect::session(&e, true);
        let r = e.execute(CMD, &json!({"track": "Sound", "effects": [{"plugin": "gain"}, {"plugin": "no-such-plugin"}]}));
        assert!(r.is_err());
        assert_eq!(crate::inspect::session(&e, true), before);
        e.execute(CMD, &json!({"track": "Sound", "effects": [{"plugin": "gain"}, {"plugin": "eq_7band"}]})).unwrap();
        assert_eq!(e.undo_label(), Some("Apply Sound Effect Chain"));
        e.execute("edit.undo", &json!({})).unwrap();
        assert!(e.session().tracks[0].mixer.inserts.iter().all(Option::is_none));
        e.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(e.session().tracks[0].mixer.inserts.iter().filter(|s| s.is_some()).count(), 2);
    }

    #[test]
    fn rejects_bad_modes_parameters_instruments_and_oversize_chains() {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"name": "Sound"})).unwrap();
        let before = crate::inspect::session(&e, true);
        for p in [
            json!({"track": "Sound", "mode": "typo", "effects": []}),
            json!({"track": "Sound", "effects": [{"plugin": "gain", "params": {"oops": 3}}]}),
            json!({"track": "Sound", "effects": [{"plugin": "gain", "params": {"gain": "bad"}}]}),
            json!({"track": "Sound", "effects": [{"plugin": "gain", "bypass": "false"}]}),
            json!({"track": "Sound", "effects": vec![json!({"plugin": "gain"}); 11]}),
        ] {
            assert!(e.execute(CMD, &p).is_err(), "{p}");
            assert_eq!(crate::inspect::session(&e, true), before);
        }
    }

    #[test]
    fn append_preserves_order_and_replace_preserves_trim_and_volume_automation() {
        let mut e = Engine::default();
        e.execute("track.new", &json!({"name": "Sound"})).unwrap();
        e.execute("mix.insert", &json!({"track": "Sound", "slot": 2, "plugin": "gain"})).unwrap();
        for param in ["volume", "plugin:c:gain", "plugin:d:gain"] {
            e.execute("automation.set_point", &json!({"track": "Sound", "param": param, "at": 0, "value": -6})).unwrap();
        }
        e.session_mut().tracks[0].lane_mut(&super::super::track_more::trim_param()).set_point(0, -3.0);
        let result = e.execute(CMD, &json!({"track": "Sound", "effects": [{"plugin": "gain"}, {"plugin": "eq_7band"}]})).unwrap();
        assert_eq!(result["effects"][0]["slot"], 3);
        assert_eq!(result["effects"][1]["slot"], 4);
        assert_eq!(e.session().tracks[0].automation.len(), 3);
        e.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(e.session().tracks[0].automation.len(), 4);
        e.execute(CMD, &json!({"track": "Sound", "mode": "replace", "effects": []})).unwrap();
        assert!(e.session().tracks[0].mixer.inserts.iter().all(Option::is_none));
        assert_eq!(e.session().tracks[0].automation.len(), 2);
        assert!(e.session().tracks[0].automation.iter().any(|lane| lane.param == super::super::track_more::trim_param()));
        assert!(e.session().tracks[0].automation.iter().any(|lane| lane.param == soundcraft_model::AutoParam::Volume));
    }
}
