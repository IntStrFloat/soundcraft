//! Purpose-oriented sound design. The client LLM interprets free language; this module supplies
//! a deterministic, inspectable starting point and keeps every generated variation undoable.
use super::{CommandSpec, always, bad, position_param};
use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_audio_io::{AudioBuffer, FileFormat};
use soundcraft_model::{ChannelFormat, Clip, ClipId, Fade, FadeShape, Insert, TrackKind};
use soundcraft_time::{MAX_POSITION, Range};

const PLAN: &str = "sound.design_plan";
const DESIGN: &str = "sound.design";
const MAX_SECONDS: f64 = 30.0;
// A high-rate session must not multiply a short sound into an unbounded allocation.
const MAX_TOTAL_SAMPLES: usize = 16_000_000;
const PURPOSES: &[&str] = &["transition", "reveal", "disappear", "typography", "impact", "notification", "ambience"];
const VIBES: &[&str] = &["clean", "warm", "dark", "cinematic", "retro", "futuristic", "playful", "organic"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "sound.design_plan", "Plan Sound Design", [], None,
            "{purpose?: transition|reveal|disappear|typography|impact|notification|ambience, vibe?: clean|warm|dark|cinematic|retro|futuristic|playful|organic, brief?: RU/EN text, duration?: seconds 0.03..30, intensity?: 0..1, source_clip?: id, name?, at?} — inspect interpretation, editable inserts and fades without changing the session", always, plan),
        cmd!(
            "sound.design",
            "Design Sound",
            [],
            None,
            "{purpose?, vibe?, brief?, duration?: seconds 0.03..30, intensity?: 0..1, source_clip?: id, name?, at?} — create one original synthetic sound or a processed copy of an existing mono/stereo clip on a new track; one undo step",
            always,
            design
        ),
    ]
}

struct Design {
    purpose: &'static str,
    vibe: &'static str,
    brief: String,
    assumptions: Vec<String>,
    seconds: f64,
    frames: usize,
    intensity: f32,
    source: Option<ClipId>,
    channels: usize,
    name: String,
    at: i64,
    fade_in: i64,
    fade_out: i64,
    inserts: Vec<Insert>,
}

fn string(p: &Value, key: &str, cmd: &str) -> Result<Option<String>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::String(v)) if v.chars().count() <= 4096 => Ok(Some(v.trim().to_string())),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a string of at most 4096 characters"))),
    }
}

fn number(p: &Value, key: &str, default: f64, min: f64, max: f64, cmd: &str) -> Result<f64> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && (min..=max).contains(n))
            .ok_or_else(|| bad(cmd, format!("`{key}` must be a finite number in {min}..{max}"))),
    }
}

fn interpret(
    explicit: Option<String>,
    text: &str,
    choices: &[&'static str],
    matches: &[(&'static str, &[&str])],
    default: &'static str,
    kind: &str,
    assumptions: &mut Vec<String>,
    cmd: &str,
) -> Result<&'static str> {
    if let Some(value) = explicit {
        return choices
            .iter()
            .copied()
            .find(|choice| choice.eq_ignore_ascii_case(&value))
            .ok_or_else(|| bad(cmd, format!("unknown {kind} `{value}`; choose {}", choices.join(", "))));
    }
    let found: Vec<_> = matches.iter().filter(|(_, words)| words.iter().any(|word| text.contains(word))).map(|(id, _)| *id).collect();
    if let Some(first) = found.first() {
        assumptions.push(format!("{kind} `{first}` matched brief keywords; this is a preset rule, not a language model"));
        if found.len() > 1 {
            assumptions.push(format!("multiple {kind} matches ({}); used `{first}`; pass an explicit {kind} to override", found.join(", ")));
        }
        return Ok(first);
    }
    assumptions.push(format!("no {kind} keyword matched; defaulted to `{default}`; the client may choose another explicit {kind}"));
    Ok(default)
}

fn insert(id: &str, values: &[(&str, f32)], cmd: &str) -> Result<Insert> {
    let info = soundcraft_dsp::plugin_info(id).ok_or_else(|| bad(cmd, format!("design plugin `{id}` is unavailable")))?;
    let mut insert = Insert::new(id);
    for param in info.params {
        insert.params.insert(param.id.to_string(), param.default);
    }
    for &(id, value) in values {
        let param = info.param(id).ok_or_else(|| bad(cmd, format!("design parameter `{id}` is unavailable")))?;
        insert.params.insert(id.to_string(), param.clamp(value));
    }
    Ok(insert)
}

fn parse(e: &Engine, p: &Value, cmd: &str) -> Result<Design> {
    if !p.is_object() {
        return Err(bad(cmd, "parameters must be an object"));
    }
    let brief = string(p, "brief", cmd)?.unwrap_or_default();
    let lowered = brief.to_lowercase();
    let mut assumptions = Vec::new();
    let purpose = interpret(
        string(p, "purpose", cmd)?,
        &lowered,
        PURPOSES,
        &[
            ("typography", &["typograph", "text", "title", "typewriter", "шрифт", "текст", "букв", "титр", "печат"]),
            ("disappear", &["disappear", "fade out", "exit", "исчез", "убав", "уход", "затух"]),
            ("transition", &["transition", "whoosh", "swoosh", "переход", "свуш", "вжух"]),
            ("reveal", &["reveal", "appear", "logo", "появ", "логотип", "открыт"]),
            ("impact", &["impact", "hit", "boom", "удар", "взрыв"]),
            ("notification", &["notification", "notify", "success", "уведом", "успех", "сигнал"]),
            ("ambience", &["ambien", "ambient", "background", "атмосфер", "эмбиент", "фон"]),
        ],
        "transition",
        "purpose",
        &mut assumptions,
        cmd,
    )?;
    let vibe = interpret(
        string(p, "vibe", cmd)?,
        &lowered,
        VIBES,
        &[
            ("dark", &["dark", "ominous", "мрач", "темн", "тёмн", "страш"]),
            ("cinematic", &["cinema", "epic", "кин", "эпич"]),
            ("retro", &["retro", "vintage", "8-bit", "8bit", "ретро", "винтаж"]),
            ("futuristic", &["futur", "sci-fi", "scifi", "cyber", "футур", "кибер", "космич"]),
            ("playful", &["playful", "cartoon", "fun", "игрив", "мульт", "весёл", "весел"]),
            ("organic", &["organic", "natural", "wood", "органич", "натураль", "дерев"]),
            ("warm", &["warm", "soft", "тепл", "тёпл", "мягк"]),
            ("clean", &["clean", "minimal", "чист", "минимал"]),
        ],
        "clean",
        "vibe",
        &mut assumptions,
        cmd,
    )?;
    let default_seconds = match purpose {
        "typography" => 0.12,
        "notification" => 0.55,
        "impact" => 0.8,
        "reveal" => 1.1,
        "disappear" => 0.7,
        "ambience" => 5.0,
        _ => 0.8,
    };
    let seconds = number(p, "duration", default_seconds, 0.03, MAX_SECONDS, cmd)?;
    let intensity = number(p, "intensity", 0.5, 0.0, 1.0, cmd)? as f32;
    let frames = (seconds * e.session().sample_rate.as_f64()).round() as usize;
    let source = match p.get("source_clip") {
        None => None,
        Some(v) => Some(ClipId(v.as_u64().filter(|&n| n != 0).ok_or_else(|| bad(cmd, "`source_clip` must be a positive clip id"))?)),
    };
    let channels = if let Some(id) = source {
        let (track, clip) = e.session().find_clip(id).ok_or_else(|| bad(cmd, "source clip does not exist"))?;
        let source = clip.source().ok_or_else(|| bad(cmd, "source clip must contain audio"))?;
        if !e.session().pool.contains(source) {
            return Err(bad(cmd, "source clip audio is missing; load its media first"));
        }
        if clip.length <= 0 || clip.muted {
            return Err(bad(cmd, "source clip must have positive length and be unmuted"));
        }
        let channels = e.session().track(track).map_or(0, |t| t.channels());
        if !(1..=2).contains(&channels) {
            return Err(bad(cmd, "sound design copies mono/stereo clips; use the regular render commands for surround"));
        }
        assumptions.push("source copy bakes clip gain, fades and stretch; track inserts/fader are excluded; shorter sources are zero-padded, longer sources trimmed without changing pitch".into());
        channels
    } else {
        2
    };
    if frames.checked_mul(channels).is_none_or(|n| n > MAX_TOTAL_SAMPLES) {
        return Err(bad(cmd, "sound exceeds the 16 million sample design budget; reduce duration or session sample rate"));
    }
    let at = position_param(e, cmd, p, "at")?.unwrap_or(e.session().edit.selection.start);
    if at < 0 || at.checked_add(frames as i64).is_none_or(|end| end > MAX_POSITION) {
        return Err(bad(cmd, "sound position and end must lie within the session timeline"));
    }
    let name = string(p, "name", cmd)?.filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{vibe} {purpose}"));
    let fade_in_seconds: f64 = match purpose {
        "transition" => seconds * 0.35,
        "disappear" => 0.003,
        "ambience" => seconds * 0.15,
        _ => 0.003,
    };
    let fade_out_seconds: f64 = match purpose {
        "disappear" => seconds * 0.8,
        "ambience" => seconds * 0.2,
        "transition" => seconds * 0.25,
        _ => seconds * 0.4,
    };
    let fade_in = (fade_in_seconds.min(seconds * 0.45) * e.session().sample_rate.as_f64()).round() as i64;
    let fade_out = (fade_out_seconds * e.session().sample_rate.as_f64()).round() as i64;
    let mut inserts = Vec::new();
    match vibe {
        "warm" => inserts.push(insert("saturator", &[("drive", 2.0 + intensity * 5.0), ("tone", 6500.0), ("mix", 40.0), ("output", -2.0)], cmd)?),
        "dark" => {
            inserts.push(insert("eq_1band", &[("type", 4.0), ("freq", 2200.0), ("q", 0.707)], cmd)?);
            inserts.push(insert("room_reverb", &[("mix", 12.0 + intensity * 15.0), ("decay", 1.8), ("damping", 2500.0)], cmd)?);
        }
        "cinematic" => {
            inserts.push(insert("saturator", &[("drive", 3.0 + intensity * 6.0), ("mix", 30.0), ("output", -3.0)], cmd)?);
            inserts.push(insert("plate_reverb", &[("mix", 16.0 + intensity * 15.0), ("decay", 2.2), ("pre_delay", 15.0)], cmd)?);
        }
        "retro" => inserts.push(insert("lofi", &[("bits", 8.0), ("sample_rate", 12000.0), ("noise", 0.0), ("mix", 45.0 + intensity * 40.0)], cmd)?),
        "futuristic" => {
            inserts.push(insert("flanger", &[("rate", 0.5), ("depth", 70.0), ("feedback", 35.0), ("mix", 15.0 + intensity * 25.0)], cmd)?);
            inserts.push(insert("mod_delay", &[("time", 90.0), ("feedback", 22.0), ("mix", 12.0)], cmd)?);
        }
        "playful" => inserts.push(insert("mod_delay", &[("time", 105.0), ("feedback", 18.0), ("mix", 10.0 + intensity * 12.0)], cmd)?),
        "organic" => {
            inserts.push(insert("room_reverb", &[("mix", 10.0 + intensity * 10.0), ("decay", 0.45), ("pre_delay", 3.0), ("damping", 4500.0)], cmd)?)
        }
        _ => {}
    }
    Ok(Design { purpose, vibe, brief, assumptions, seconds, frames, intensity, source, channels, name, at, fade_in, fade_out, inserts })
}

impl Design {
    fn report(&self, sample_rate: u32) -> Value {
        json!({
            "purpose": self.purpose, "vibe": self.vibe, "brief": self.brief,
            "duration": self.seconds, "frames": self.frames, "sample_rate": sample_rate,
            "intensity": self.intensity, "source_clip": self.source.map(|id| id.0), "channels": self.channels,
            "name": self.name, "at": self.at, "end": self.at.saturating_add(self.frames as i64),
            "interpretation": {"method": "explicit fields or bilingual preset keyword rules", "assumptions": self.assumptions},
            "stages": [if self.source.is_some() {"isolated clip copy; trim or zero-pad"} else {"original deterministic procedural synthesis"}, "purpose envelope", "editable clip fades", "editable track plugin chain"],
            "inserts": self.inserts.iter().enumerate().map(|(slot, insert)| json!({"slot": slot, "plugin": insert.plugin, "params": insert.params})).collect::<Vec<_>>(),
            "fade_in": {"samples": self.fade_in, "shape": "s_curve"},
            "fade_out": {"samples": self.fade_out, "shape": "s_curve"},
            "tail_note": "duration is the dry clip length; reverb/delay can ring beyond it, so include their tail when previewing or exporting",
            "available_purposes": PURPOSES, "available_vibes": VIBES,
        })
    }
}

fn plan(e: &mut Engine, p: &Value) -> Result<Value> {
    Ok(parse(e, p, PLAN)?.report(e.session().sample_rate.hz()))
}

/// Original noise + oscillator textures. Separate stereo noise streams avoid a mono whoosh;
/// phase is integrated so sweeps remain continuous at every supported sample rate.
fn synth(d: &Design, sample_rate: f64) -> Vec<Vec<f32>> {
    let mut channels = Vec::with_capacity(d.channels);
    for channel in 0..d.channels {
        let mut noise_state = 0x9e37_79b9_7f4a_7c15u64.wrapping_add(channel as u64 * 157);
        let mut phase = 0.0f64;
        let mut low_noise = 0.0f64;
        let mut out = Vec::with_capacity(d.frames);
        let base = match d.vibe {
            "dark" => 90.0,
            "cinematic" => 140.0,
            "warm" | "organic" => 290.0,
            "playful" => 720.0,
            "futuristic" => 1100.0,
            _ => 480.0,
        };
        for frame in 0..d.frames {
            let progress = frame as f64 / d.frames.saturating_sub(1).max(1) as f64;
            let seconds = frame as f64 / sample_rate;
            noise_state ^= noise_state << 13;
            noise_state ^= noise_state >> 7;
            noise_state ^= noise_state << 17;
            let noise = (noise_state >> 40) as f64 / (1u64 << 24) as f64 * 2.0 - 1.0;
            low_noise += (0.025 + 0.3 * progress) * (noise - low_noise);
            let freq = match d.purpose {
                "transition" | "reveal" => base * (0.4 + progress * progress * 3.6),
                "disappear" => base * (2.0 - progress * 1.7),
                "impact" => base * (0.35 + 1.7 * (-seconds * 35.0).exp()),
                "notification" => base * if progress < 0.42 { 1.0 } else { 1.5 },
                "typography" => base * (1.0 + 1.8 * (-seconds * 95.0).exp()),
                _ => base * (1.0 + 0.015 * (seconds * 1.7).sin()),
            }
            .min(sample_rate * 0.2);
            phase = (phase + std::f64::consts::TAU * freq / sample_rate) % std::f64::consts::TAU;
            let tone = phase.sin();
            let value = match d.purpose {
                "transition" => low_noise * 0.8 + tone * 0.12,
                "reveal" => low_noise * (1.0 - progress) * 0.55 + tone * 0.4 * (-progress * 1.6).exp(),
                "disappear" => low_noise * 0.45 + tone * 0.3,
                "typography" => (noise * 0.65 + tone * 0.35) * (-seconds * 65.0).exp(),
                "impact" => tone * 0.6 * (-seconds * 9.0).exp() + low_noise * 0.55 * (-seconds * 22.0).exp(),
                "notification" => tone * 0.55 * (-progress * 2.8).exp(),
                _ => low_noise * 0.3 + tone * 0.15 + (phase * 1.501).sin() * 0.05,
            };
            out.push((value * (0.2 + f64::from(d.intensity) * 0.55)) as f32);
        }
        channels.push(out);
    }
    channels
}

fn copy_source(e: &Engine, d: &Design, id: ClipId) -> Result<Vec<Vec<f32>>> {
    let (track, clip) = e.session().find_clip(id).ok_or_else(|| bad(DESIGN, "source clip disappeared"))?;
    let mut isolated = e.session().clone();
    if let Some(playlist) = isolated.track_mut(track).and_then(|track| track.playlist_mut()) {
        playlist.clips.retain(|clip| clip.id == id);
    }
    // Only render the requested prefix; a many-hour source never creates a many-hour buffer.
    let end = clip.start.saturating_add(clip.length.min(d.frames as i64));
    let mut audio = soundcraft_mix::render_clips(&isolated, track, Range::new(clip.start, end));
    if audio.len() != d.channels {
        return Err(bad(DESIGN, "source render returned an unexpected channel count"));
    }
    for channel in &mut audio {
        channel.resize(d.frames, 0.0);
        for sample in channel {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
    }
    // Preserve the source's dynamics; intensity is an intentional gain offset around 0.5.
    soundcraft_dsp::offline::gain(&mut audio, (d.intensity - 0.5) * 12.0);
    Ok(audio)
}

fn design(e: &mut Engine, p: &Value) -> Result<Value> {
    let d = parse(e, p, DESIGN)?;
    let sample_rate = e.session().sample_rate.hz();
    let mut audio = match d.source {
        Some(id) => copy_source(e, &d, id)?,
        None => synth(&d, f64::from(sample_rate)),
    };
    soundcraft_dsp::offline::remove_dc(&mut audio);
    let mut report = d.report(sample_rate);
    // Everything fallible runs before the first session mutation. The command registry journals
    // the complete document change as one action, so undo removes both source and variation.
    let session = e.session_mut();
    let track = session.add_track(TrackKind::Audio, if d.channels == 1 { ChannelFormat::Mono } else { ChannelFormat::Stereo }, Some(&d.name));
    let source = crate::io::add_source(session, &d.name, AudioBuffer { sample_rate, channels: audio }, None, FileFormat::Wav);
    let id = session.new_clip_id();
    let mut clip = Clip::audio(id, &d.name, source, 0, d.at, d.frames as i64);
    clip.fade_in = Fade { len: d.fade_in, shape: FadeShape::SCurve, curvature: 0.0 };
    clip.fade_out = Fade { len: d.fade_out, shape: FadeShape::SCurve, curvature: 0.0 };
    crate::edit::place_clip(session, track, clip);
    if let Some(created) = session.track_mut(track) {
        report["requested_name"] = json!(d.name);
        report["name"] = json!(created.name);
        for (slot, insert) in created.mixer.inserts.iter_mut().zip(d.inserts) {
            *slot = Some(insert);
        }
    }
    session.edit.selected_tracks = vec![track];
    session.edit.selected_clips = vec![id];
    session.edit.selection = Range::new(d.at, d.at.saturating_add(d.frames as i64));
    report["track"] = json!(track.0);
    report["clip"] = json!(id.0);
    report["source"] = json!(source.0);
    report["undo"] = json!("edit.undo removes this complete variation in one step");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_bilingual_briefs_without_mutation() {
        let mut e = Engine::default();
        let before = serde_json::to_value(e.session()).unwrap();
        let result = plan(&mut e, &json!({"brief": "Мягкое появление логотипа", "duration": 0.1})).unwrap();
        assert_eq!(result["purpose"], "reveal");
        assert_eq!(result["vibe"], "warm");
        assert_eq!(result["inserts"][0]["plugin"], "saturator");
        assert_eq!(before, serde_json::to_value(e.session()).unwrap());
        assert!(!e.can_undo());
        let fallback = plan(&mut e, &json!({"brief": "unrecognized request"})).unwrap();
        assert_eq!(fallback["interpretation"]["assumptions"].as_array().unwrap().len(), 2);
        let explicit = plan(&mut e, &json!({"purpose": "typography", "vibe": "retro", "brief": "cinematic transition"})).unwrap();
        assert_eq!(explicit["purpose"], "typography");
        assert_eq!(explicit["vibe"], "retro");
    }

    #[test]
    fn variations_report_actual_unique_track_names() {
        let mut e = Engine::default();
        for name in ["Title".to_owned(), "Title".to_owned(), "Sound".repeat(200)] {
            let result = e.execute(DESIGN, &json!({"name": name, "duration": 0.03})).unwrap();
            let created = e.session().track(soundcraft_model::TrackId(result["track"].as_u64().unwrap())).unwrap();
            assert_eq!(result["name"], created.name);
            assert_eq!(result["requested_name"], name);
        }
        assert_ne!(e.session().tracks[0].name, e.session().tracks[1].name);
    }

    #[test]
    fn generated_variation_is_nonzero_deterministic_and_one_undo_step() {
        let mut e = Engine::default();
        let p = json!({"purpose": "transition", "vibe": "futuristic", "duration": 0.06});
        let d = parse(&e, &p, DESIGN).unwrap();
        assert_eq!(synth(&d, 48_000.0), synth(&d, 48_000.0));
        let result = e.execute(DESIGN, &p).unwrap();
        let track = e.session().tracks.first().unwrap();
        assert_eq!(track.id.0, result["track"].as_u64().unwrap());
        assert_eq!(track.mixer.inserts.iter().filter(|s| s.is_some()).count(), 2);
        let source = e.session().pool.get(track.clips()[0].source().unwrap()).unwrap();
        assert!(source.buffer.channels.iter().flatten().any(|&s| s.abs() > 0.001));
        assert!(source.buffer.channels.iter().flatten().all(|s| s.is_finite()));
        assert_eq!(e.undo_history().len(), 1);
        assert!(e.undo());
        assert!(e.session().tracks.is_empty());
        assert!(e.session().sources.is_empty());
        assert!(e.redo());
        assert_eq!(e.session().tracks.len(), 1);
    }

    #[test]
    fn source_copy_keeps_original_and_bakes_clip_gain_before_adding_editable_fades() {
        let mut e = Engine::default();
        let session = e.session_mut();
        let track = session.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("original"));
        let source = crate::io::add_source(
            session,
            "original",
            AudioBuffer { sample_rate: 48_000, channels: vec![(0..4800).map(|i| (i as f32 * 0.1).sin() * 0.5).collect()] },
            None,
            FileFormat::Wav,
        );
        let id = session.new_clip_id();
        let mut clip = Clip::audio(id, "original", source, 0, 0, 4800);
        clip.gain_db = -6.0;
        let original = clip.clone();
        crate::edit::place_clip(session, track, clip);
        let result = e.execute(DESIGN, &json!({"source_clip": id.0, "purpose": "reveal", "vibe": "clean", "duration": 0.2})).unwrap();
        assert_eq!(e.session().find_clip(id).unwrap().1, &original);
        let new_source = e.session().pool.get(soundcraft_model::SourceId(result["source"].as_u64().unwrap())).unwrap();
        assert_eq!(new_source.buffer.frames(), 9600);
        assert!(soundcraft_dsp::offline::peak(&new_source.buffer.channels) < 0.26);
        assert_eq!(e.session().tracks.len(), 2);
        assert!(e.undo());
        assert_eq!(e.session().tracks.len(), 1);
        assert_eq!(e.session().find_clip(id).unwrap().1, &original);
    }

    #[test]
    fn every_purpose_and_vibe_has_valid_plugin_parameters_and_nonzero_audio() {
        let e = Engine::default();
        for purpose in PURPOSES {
            for vibe in VIBES {
                let d = parse(&e, &json!({"purpose": purpose, "vibe": vibe, "duration": 0.04}), PLAN).unwrap();
                let audio = synth(&d, 48_000.0);
                assert!(soundcraft_dsp::offline::peak(&audio) > 0.001, "{purpose}/{vibe}");
                for insert in d.inserts {
                    let info = soundcraft_dsp::plugin_info(&insert.plugin).unwrap();
                    for (id, value) in insert.params {
                        assert_eq!(info.param(&id).unwrap().clamp(value), value);
                    }
                }
            }
        }
    }

    #[test]
    fn hostile_parameters_are_rejected_before_mutation() {
        let mut e = Engine::default();
        let before = serde_json::to_value(e.session()).unwrap();
        for p in [
            json!(null),
            json!({"purpose": "unknown"}),
            json!({"vibe": "unknown"}),
            json!({"duration": 0}),
            json!({"duration": 31}),
            json!({"duration": "one"}),
            json!({"intensity": -1}),
            json!({"brief": 3}),
            json!({"at": -1}),
            json!({"at": MAX_POSITION}),
            json!({"source_clip": 1234}),
            json!({"source_clip": null}),
        ] {
            assert!(e.execute(DESIGN, &p).is_err(), "{p}");
            assert_eq!(before, serde_json::to_value(e.session()).unwrap());
            assert!(!e.can_undo());
        }
    }
}
