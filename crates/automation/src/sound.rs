//! Purpose-led sound-design tools and the client-side creative workflow.
use serde_json::{Value, json};

pub const WORKFLOW: &str = "For a sound brief, interpret the user's language, purpose, mood, timing and context in the client model. \
Use plan_sound for a starting recipe, then design_sound to create original synthetic audio or a NEW variant of source_clip. \
The server's brief matcher is a small bilingual keyword fallback, not a language model. Translate nuanced briefs to explicit purpose/vibe \
and customize the result using the entire command/plugin catalogue. Inspect before editing existing work. list_plugins returns exact ids, \
parameter ranges, units, defaults and choices; never invent plugin ids, parameter names or installed external plugins. \
apply_effect_chain validates a complete chain and applies it with one undo; append follows existing inserts, replace clears inserts. \
Only reassigned insert slots lose their old automation; volume/trim automation stays. Use set_plugin_params, bypass_plugin, remove_plugin and set_automation_point for revisions. \
process_audio renders offline processing on explicit clip ids (reverse, normalize, pitch/time, gain, fades and supported AudioSuite processors); \
discover exact options through list_commands before use. All other editing, routing, MIDI, presets, automation, trim, fades, crossfades, \
stretch and export commands remain available through execute/batch. Preserve originals and make named variants for comparisons. \
preview_audio returns actual WAV audio and measurements for up to 30 seconds; it works headlessly, never starts speakers or writes a file. \
Choose track and explicit start/end for an individual sound or use the whole mix to hear context. Check silence, clipping and media warnings; \
short SFX may have no meaningful integrated LUFS. Refine, preview again, then bounce_mix to WAV/AIFF/FLAC and save_session. \
Do not claim to have listened unless the client actually evaluated the returned audio. Do not claim an exact LUFS target from peak normalization. \
No external generative service or transcription is bundled; describe synthetic results honestly. Respect file-access roots and report unsupported \
formats/plugins rather than silently substituting. In headless mode transport commands do not produce audible playback; desktop transport requires --connect.";

fn tool(name: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    json!({"name": name, "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
        "annotations": {"readOnlyHint": read_only, "destructiveHint": !read_only, "openWorldHint": false}})
}

fn track_schema() -> Value {
    json!({"type": ["string", "integer"], "description": "Exact track name or numeric track id from inspect_session."})
}

fn position_schema() -> Value {
    json!({"description": "Sample integer, {seconds: number}, or session time string (e.g. bars_beats:5|1|000)."})
}

fn design_properties() -> Value {
    json!({
        "brief": {"type": "string", "maxLength": 4096, "description": "Sound purpose, context and vibe in English or Russian. Prefer explicit purpose/vibe for nuanced requests; keyword fallback reports its assumptions."},
        "purpose": {"type": "string", "enum": ["transition", "reveal", "disappear", "typography", "impact", "notification", "ambience"]},
        "vibe": {"type": "string", "enum": ["clean", "warm", "dark", "cinematic", "retro", "futuristic", "playful", "organic"]},
        "duration": {"type": "number", "minimum": 0.03, "maximum": 30, "description": "Duration in seconds, including the designed clip envelope. Preview/bounce can include extra effect tail."},
        "intensity": {"type": "number", "minimum": 0, "maximum": 1},
        "source_clip": {"type": "integer", "minimum": 1, "description": "Optional existing audio clip id: create a new variant from its audible clip material; original and track inserts stay untouched."},
        "name": {"type": "string", "maxLength": 256}, "at": position_schema()
    })
}

fn render_properties() -> Value {
    json!({"track": track_schema(), "start": position_schema(), "end": position_schema(),
        "duration": {"type": "number", "exclusiveMinimum": 0, "maximum": 30, "description": "Seconds; omit end when using duration. Default preview is at most 10 seconds."}})
}

pub fn tools() -> Vec<Value> {
    vec![
        tool(
            "list_plugins",
            "Discover built-in effects/instruments and exact parameters. Opt in to scanning installed CLAP/VST3/Audio Units; request plugin by id for details. External plugins require compatible locally installed binaries.",
            json!({"filter": {"type": "string"}, "plugin": {"type": "string"}, "include_external": {"type": "boolean", "default": false}}),
            &[],
            true,
        ),
        tool(
            "plan_sound",
            "Read-only starting recipe from sound purpose and vibe. Returns interpretation and assumptions before creation. The client model handles natural-language nuance; explicit fields override simple RU/EN keyword fallback.",
            design_properties(),
            &[],
            true,
        ),
        tool(
            "design_sound",
            "Create an original synthetic sound or a new variant of source_clip on a NEW track with editable effects/fades. Seven purposes, eight vibes. One undo restores the document; returns track and clip ids for refinement.",
            design_properties(),
            &[],
            false,
        ),
        tool(
            "analyze_audio",
            "Measure current mix or isolated source track with effects/routing: peak, true peak, RMS, LUFS, clipping and warnings. No file writes or playback, at most 30 seconds. Non-measurable silence/loudness is null.",
            render_properties(),
            &[],
            true,
        ),
        tool(
            "preview_audio",
            "Return a real PCM16 WAV as MCP audio plus analysis for the current mix or one source track, max 30 seconds. No file or speaker playback. Include start/end to hear a sound placed later on the timeline. Peaks are measured before PCM encoding.",
            render_properties(),
            &[],
            true,
        ),
        tool(
            "apply_effect_chain",
            "Apply a validated ordered effect chain atomically with one undo. Append follows the last used slot (max 10); replace clears all inserts. Automation on reassigned insert slots is cleared; track trim stays. Discover exact ids/parameter ranges with list_plugins.",
            json!({"track": track_schema(), "mode": {"type": "string", "enum": ["append", "replace"], "default": "append"},
                "effects": {"type": "array", "maxItems": 10, "items": {"type": "object", "properties": {
                    "plugin": {"type": "string"}, "params": {"type": "object", "additionalProperties": {"type": "number"}}, "bypass": {"type": "boolean"}}, "required": ["plugin"], "additionalProperties": false}}}),
            &["track", "effects"],
            false,
        ),
        tool(
            "set_plugin_params",
            "Refine parameters of an existing insert. Unknown ids/non-numeric values fail. Values clamp to plugin ranges; use list_plugins to discover them. Undoable.",
            json!({"track": track_schema(), "slot": {"type": ["string", "integer"], "description": "0..9 or a..j"},
                "params": {"type": "object", "additionalProperties": {"type": "number"}}, "preset": {"type": "string"}}),
            &["track", "slot", "params"],
            false,
        ),
        tool(
            "bypass_plugin",
            "Bypass or enable an insert for A/B comparison; value=true bypasses, false enables. Undoable.",
            json!({"track": track_schema(), "slot": {"type": ["string", "integer"]}, "value": {"type": "boolean"}}),
            &["track", "slot", "value"],
            false,
        ),
        tool(
            "remove_plugin",
            "Remove one insert. Undoable.",
            json!({"track": track_schema(), "slot": {"type": ["string", "integer"]}}),
            &["track", "slot"],
            false,
        ),
        tool(
            "process_audio",
            "Offline AudioSuite processing on explicit audio clip ids: reverse, normalize, pitch/time and built-in effects. Updates selected clip material with undo; use design_sound(source_clip) first to preserve a separate original. Discover process options via list_commands(audiosuite).",
            json!({"process": {"type": "string"}, "clips": {"type": "array", "minItems": 1, "items": {"type": "integer"}},
                "params": {"type": "object"}}),
            &["process", "clips"],
            false,
        ),
        tool(
            "set_automation_point",
            "Animate volume, pan, sends or plugin parameters. e.g. param=plugin:a:cutoff. Values use each lane's native units; discover automation.set_point for exact behavior. Undoable.",
            json!({"track": track_schema(), "param": {"type": "string"}, "at": position_schema(), "value": {"type": "number"}, "from": position_schema()}),
            &["track", "param", "at", "value"],
            false,
        ),
        tool("undo", "Undo the previous edit; design_sound and apply_effect_chain each undo as one operation.", json!({}), &[], false),
        tool("redo", "Redo the previous undone edit.", json!({}), &[], false),
    ]
}

pub fn command(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "list_plugins" => "sound.catalog",
        "plan_sound" => "sound.design_plan",
        "design_sound" => "sound.design",
        "analyze_audio" => "sound.analyze",
        "preview_audio" => "sound.preview",
        "apply_effect_chain" => "sound.effect_chain",
        "set_plugin_params" => "mix.insert_params",
        "bypass_plugin" => "mix.insert_bypass",
        "remove_plugin" => "mix.insert_remove",
        "process_audio" => "audiosuite.process",
        "set_automation_point" => "automation.set_point",
        "undo" => "edit.undo",
        "redo" => "edit.redo",
        _ => return None,
    })
}

/// Validate the deliberately small schema vocabulary used above before forwarding a typed
/// alias. Legacy engine commands may use selection defaults or toggle on a missing boolean;
/// those are inappropriate when this MCP tool promises an explicit target/value.
pub fn validate_arguments(name: &str, args: &Value) -> Result<(), String> {
    let definition = tools().into_iter().find(|t| t["name"] == name).ok_or("unknown sound tool")?;
    validate(args, &definition["inputSchema"], "arguments")
}

fn validate(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let matches_type = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.as_f64().is_some_and(f64::is_finite),
        _ => false,
    };
    if let Some(kind) = schema.get("type") {
        let valid = match kind {
            Value::String(kind) => matches_type(kind),
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).any(matches_type),
            _ => false,
        };
        if !valid {
            return Err(format!("`{path}` must have type {kind}"));
        }
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array)
        && !choices.contains(value)
    {
        return Err(format!("`{path}` must be one of {}", Value::Array(choices.clone())));
    }
    if let Some(number) = value.as_f64() {
        for (key, valid) in [("minimum", true), ("maximum", false), ("exclusiveMinimum", true)] {
            if let Some(bound) = schema.get(key).and_then(Value::as_f64) {
                let outside = if key == "exclusiveMinimum" {
                    number <= bound
                } else if valid {
                    number < bound
                } else {
                    number > bound
                };
                if outside {
                    return Err(format!("`{path}` violates {key} {bound}"));
                }
            }
        }
    }
    if let Some(string) = value.as_str()
        && schema.get("maxLength").and_then(Value::as_u64).is_some_and(|limit| string.chars().count() as u64 > limit)
    {
        return Err(format!("`{path}` is too long"));
    }
    if let Some(array) = value.as_array() {
        if schema.get("minItems").and_then(Value::as_u64).is_some_and(|limit| (array.len() as u64) < limit)
            || schema.get("maxItems").and_then(Value::as_u64).is_some_and(|limit| array.len() as u64 > limit)
        {
            return Err(format!("`{path}` has an invalid number of items"));
        }
        if let Some(items) = schema.get("items") {
            for (index, item) in array.iter().enumerate() {
                validate(item, items, &format!("{path}[{index}]"))?;
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("`{path}.{key}` required"));
                }
            }
        }
        for (key, item) in object {
            if let Some(property) = schema.get("properties").and_then(|p| p.get(key)) {
                validate(item, property, &format!("{path}.{key}"))?;
            } else {
                match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => return Err(format!("unknown argument `{path}.{key}`")),
                    Some(property @ Value::Object(_)) => validate(item, property, &format!("{path}.{key}"))?,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

pub fn guide() -> Value {
    json!({"workflow": WORKFLOW,
        "purposes": ["transition", "reveal", "disappear", "typography", "impact", "notification", "ambience"],
        "vibes": ["clean", "warm", "dark", "cinematic", "retro", "futuristic", "playful", "organic"],
        "examples": [
            {"brief": "Мягкое появление заголовка, тепло и без резкого щелчка", "purpose": "typography", "vibe": "warm", "duration": 0.45, "intensity": 0.3},
            {"brief": "Dark cinematic transition into the next scene", "purpose": "transition", "vibe": "cinematic", "duration": 1.5, "intensity": 0.7}],
        "refinement": {"effects": "list_plugins → apply_effect_chain → set_plugin_params → bypass_plugin",
            "timeline": "list_commands → execute/batch (edit, clip, automation, routing, MIDI, presets)",
            "delivery": "preview_audio → analyze_audio → bounce_mix → save_session"}})
}

pub fn prompts() -> Value {
    json!({"prompts": [{"name": "sound_design", "title": "Design or refine a sound from a brief",
        "description": "Complete sound workflow: purpose/vibe, source or synthesis, effects, A/B, measured preview, export and save.",
        "arguments": [{"name": "brief", "description": "Describe what the sound accompanies, vibe, length and desired changes, in any language.", "required": true},
            {"name": "source_clip", "description": "Existing clip id to create a new variant from.", "required": false},
            {"name": "output_path", "description": "Optional WAV/AIFF/FLAC delivery path allowed by write roots.", "required": false}]}]})
}

pub fn prompt(params: &Value) -> Result<Value, String> {
    if params.get("name").and_then(Value::as_str) != Some("sound_design") {
        return Err("unknown prompt; use sound_design".into());
    }
    let args = params.get("arguments").and_then(Value::as_object).ok_or("prompt `arguments` object required")?;
    if args.keys().any(|k| !matches!(k.as_str(), "brief" | "source_clip" | "output_path"))
        || args.values().any(|v| v.as_str().is_none_or(|s| s.len() > 16_384))
    {
        return Err("prompt arguments must be strings (at most 16384 bytes): brief, source_clip, output_path".into());
    }
    if args.get("brief").and_then(Value::as_str).is_none_or(|s| s.trim().is_empty()) {
        return Err("non-empty `brief` required".into());
    }
    Ok(json!({"description": "SoundCraft purpose-led sound design",
        "messages": [{"role": "user", "content": {"type": "text",
            "text": format!("{WORKFLOW}\n\nUser sound brief and delivery context (JSON):\n{}", Value::Object(args.clone()))}}]}))
}
