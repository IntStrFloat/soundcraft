//! Bounded, in-memory audition and signal measurements for sound-design agents.

use super::{CommandSpec, MAX_POSITION, always, bad, position_param, track_param};
use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions};
use soundcraft_model::{ClipContent, Session, TrackId, TrackKind, b64};
use soundcraft_time::Range;

const MAX_SECONDS: f64 = 30.0;
const DEFAULT_SECONDS: f64 = 10.0;
// Bound both the planar render and the encoded/base64 response, including surround sessions.
const MAX_CHANNEL_SAMPLES: usize = 8_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "sound.catalog", "Sound Design Plugin Catalog", [], None,
            "{filter?: string, plugin?: id, include_external?: false}; built-in metadata and optionally installed CLAP/VST3/AU metadata", always, catalog),
        cmd!(query "sound.analyze", "Analyze Sound", [], None,
            "{track?: id|name, start?, end?, duration?: seconds}; current mix or isolated source track through its routing; maximum 30 seconds/8M channel samples", always,
            |e, p| audition(e, p, false)),
        cmd!(query "sound.preview", "Preview Sound", [], None,
            "{track?: id|name, start?, end?, duration?: seconds}; read-only PCM16 WAV audio_base64 plus analysis, maximum 30 seconds/8M channel samples", always,
            |e, p| audition(e, p, true)),
    ]
}

fn object_params(p: &Value, id: &str) -> Result<()> {
    if !p.is_object() {
        return Err(bad(id, "parameters must be an object"));
    }
    Ok(())
}

fn optional_string<'a>(p: &'a Value, id: &str, key: &str) -> Result<Option<&'a str>> {
    p.get(key).map(|v| v.as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(id, format!("`{key}` must be a nonempty string")))).transpose()
}

fn plugin_json(info: &soundcraft_dsp::PluginInfo, format: &str, descriptor: Value) -> Value {
    json!({"id": info.id, "name": info.name, "category": info.category,
        "instrument": info.is_instrument, "audiosuite": info.audiosuite,
        "params": info.params, "format": format, "available": true, "descriptor": descriptor})
}

fn catalog(_: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "sound.catalog";
    object_params(p, ID)?;
    let filter = optional_string(p, ID, "filter")?.map(str::to_lowercase);
    let requested = optional_string(p, ID, "plugin")?;
    let external =
        p.get("include_external").map(|v| v.as_bool().ok_or_else(|| bad(ID, "`include_external` must be boolean"))).transpose()?.unwrap_or(false);
    let mut plugins: Vec<Value> = soundcraft_dsp::plugins()
        .iter()
        .filter(|info| requested.is_none_or(|id| info.id == id))
        .map(|info| plugin_json(info, "builtin", Value::Null))
        .collect();
    // A specific external id is also an explicit request to load its parameter metadata.
    let scan_external = external || requested.is_some_and(soundcraft_mix::is_third_party);
    if scan_external {
        let mut descriptors = Vec::new();
        for d in soundcraft_clap_host::scan() {
            descriptors.push((d.id.clone(), "clap", json!(d)));
        }
        for d in soundcraft_vst3_host::scan() {
            descriptors.push((d.id.clone(), "vst3", json!(d)));
        }
        for d in soundcraft_au_host::scan() {
            descriptors.push((d.id.clone(), "au", json!(d)));
        }
        for (id, format, descriptor) in descriptors {
            if requested.is_some_and(|wanted| wanted != id) {
                continue;
            }
            let info = super::clap::plugin_info(&id);
            plugins.push(match info {
                Some(info) => plugin_json(info, format, descriptor),
                None => json!({"id": id, "name": descriptor.get("name"), "format": format,
                    "available": false, "params": null, "descriptor": descriptor,
                    "error": "Plugin was discovered but could not be instantiated for parameter inspection."}),
            });
        }
    }
    if requested.is_some() && plugins.is_empty() {
        return Err(bad(ID, format!("no installed plugin `{}`", requested.unwrap_or_default())));
    }
    if let Some(filter) = filter {
        plugins.retain(|v| {
            ["id", "name", "category", "format"]
                .iter()
                .any(|key| v.get(key).and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(&filter)))
        });
    }
    Ok(json!({"count": plugins.len(), "plugins": plugins, "external_scanned": scan_external,
        "external_scope": "Installed plugins supported by this platform only. Discovery and parameter inspection load native plugins in-process."}))
}

fn checked_position(e: &Engine, p: &Value, id: &str, key: &str) -> Result<Option<i64>> {
    if let Some(v) = p.get(key) {
        let raw_samples = v.as_f64().or_else(|| v.get("seconds").and_then(Value::as_f64).map(|secs| secs * f64::from(e.session().sample_rate.hz())));
        if raw_samples.is_some_and(|n| !n.is_finite() || n.abs() > MAX_POSITION as f64) {
            return Err(bad(id, format!("`{key}` is outside the supported timeline")));
        }
    }
    position_param(e, id, p, key)
}

fn render_range(e: &Engine, p: &Value, id: &str, track: Option<TrackId>) -> Result<Range> {
    let s = e.session();
    let sr = s.sample_rate.hz();
    if sr == 0 {
        return Err(bad(id, "session sample rate must be positive"));
    }
    let content_start =
        s.tracks.iter().filter(|t| track.is_none_or(|target| target == t.id)).flat_map(|t| t.clips()).map(|c| c.start).min().unwrap_or(0).max(0);
    let content_end =
        s.tracks.iter().filter(|t| track.is_none_or(|target| target == t.id)).flat_map(|t| t.clips()).map(|c| c.end()).max().unwrap_or(0).max(0);
    let selection = s.edit.selection;
    let use_selection = !selection.is_empty() && track.is_none_or(|target| s.edit.selected_tracks.contains(&target));
    let default_start = if use_selection { selection.start } else { content_start };
    let start = checked_position(e, p, id, "start")?.unwrap_or(default_start);
    if start < 0 {
        return Err(bad(id, "`start` must be nonnegative"));
    }
    let duration = p
        .get("duration")
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite() && *n > 0.0 && *n <= MAX_SECONDS)
                .ok_or_else(|| bad(id, "`duration` must be greater than zero and at most 30 seconds"))
        })
        .transpose()?;
    let end = checked_position(e, p, id, "end")?;
    if duration.is_some() && end.is_some() {
        return Err(bad(id, "pass `duration` or `end`, not both"));
    }
    let max_default_end = start.saturating_add(s.sample_rate.samples(DEFAULT_SECONDS));
    let default_end = if use_selection && selection.end > start {
        selection.end.min(max_default_end)
    } else if content_end > start {
        content_end.min(max_default_end)
    } else {
        max_default_end
    };
    let end = match (end, duration) {
        (Some(end), _) => end,
        (_, Some(seconds)) => start.checked_add(s.sample_rate.samples(seconds)).ok_or_else(|| bad(id, "range exceeds the timeline"))?,
        _ => default_end,
    };
    if end <= start || end > MAX_POSITION {
        return Err(bad(id, "`end` must be after `start` and inside the supported timeline"));
    }
    let frames = end - start;
    if frames > s.sample_rate.samples(MAX_SECONDS) {
        return Err(bad(id, "auditions are limited to 30 seconds; choose a shorter range"));
    }
    let samples = usize::try_from(frames).ok().and_then(|n| n.checked_mul(soundcraft_mix::main_channels(s)));
    if samples.is_none_or(|n| n > MAX_CHANNEL_SAMPLES) {
        return Err(bad(id, "audition exceeds 8M channel samples; choose a shorter range"));
    }
    Ok(Range::new(start, end))
}

fn isolated_session(s: &Session, track: Option<TrackId>) -> Session {
    let mut isolated = s.clone();
    if let Some(target) = track {
        for t in &mut isolated.tracks {
            // Remove other sources rather than mute them: pre-fader sends and solo-safe tracks
            // must not leak into this audition. Keep auxes, routing folders, VCAs and masters.
            if matches!(t.kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Instrument) && t.id != target {
                t.inactive = true;
            }
            t.mixer.solo = false;
        }
    }
    isolated
}

fn finite_db(v: f32) -> Value {
    if v.is_finite() && v > soundcraft_dsp::MIN_DB { json!(v) } else { Value::Null }
}

fn audition(e: &mut Engine, p: &Value, audio: bool) -> Result<Value> {
    let id = if audio { "sound.preview" } else { "sound.analyze" };
    object_params(p, id)?;
    let track = track_param(e, id, p, "track")?;
    if let Some(target) = track {
        let t = e.session().track(target).ok_or_else(|| bad(id, "no such track"))?;
        if !matches!(t.kind, TrackKind::Audio | TrackKind::Midi | TrackKind::Instrument) {
            return Err(bad(id, "`track` must be an audio, MIDI or instrument source track; omit it to audition the full mix"));
        }
    }
    let range = render_range(e, p, id, track)?;
    let isolated = isolated_session(e.session(), track);
    let mut warnings = Vec::new();
    for t in &isolated.tracks {
        if t.inactive {
            continue;
        }
        for c in t.clips().iter().filter(|c| c.range().overlaps(&range)) {
            if let ClipContent::Audio { source, .. } = c.content
                && isolated.pool.get(source).is_none()
            {
                warnings.push(json!({"code": "unavailable_audio", "track": t.id.0, "clip": c.id.0,
                    "message": "Audio is not loaded for this clip; it renders as silence. Restore or reimport the source."}));
            }
        }
    }
    let channels = soundcraft_mix::render_range(&isolated, range, 1024);
    let buf = AudioBuffer { sample_rate: isolated.sample_rate.hz(), channels };
    let mut meter = soundcraft_dsp::meter::LoudnessMeter::new(buf.sample_rate as f32, buf.num_channels());
    meter.process(&buf.channels, buf.frames());
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    let mut clipped = 0usize;
    let mut nonfinite = 0usize;
    for value in buf.channels.iter().flat_map(|c| c.iter()) {
        if !value.is_finite() {
            nonfinite += 1;
            continue;
        }
        peak = peak.max(value.abs());
        sum += f64::from(*value).powi(2);
        clipped += usize::from(value.abs() >= 1.0);
    }
    let count = buf.frames().saturating_mul(buf.num_channels());
    let rms = if count > 0 { (sum / count as f64).sqrt() } else { 0.0 };
    if peak == 0.0 {
        warnings.push(
            json!({"code": "silence", "message": "This range is silent. Check the range, mute/inactive state, source availability and routing."}),
        );
    }
    if clipped > 0 {
        warnings.push(json!({"code": "clipping", "message": "Samples reach or exceed full scale. Lower the gain before integer export; PCM16 audition clips values outside full scale."}));
    }
    if nonfinite > 0 {
        warnings.push(
            json!({"code": "nonfinite_samples", "message": "The render contains invalid sample values; inspect its processors before delivery."}),
        );
    }
    let lufs = finite_db(meter.integrated_lufs());
    if lufs.is_null() {
        warnings.push(json!({"code": "loudness_unavailable", "message": "Integrated LUFS needs at least 400 ms of measurable signal above the loudness gate. Short one-shots should be judged with peak, RMS and listening."}));
    }
    let mut result = json!({"start": range.start, "end": range.end, "duration": buf.duration_secs(),
        "frames": buf.frames(), "sample_rate": buf.sample_rate, "channels": buf.num_channels(),
        "track": track.map(|t| t.0), "peak_db": if peak > 0.0 { json!(20.0 * peak.log10()) } else { Value::Null },
        "true_peak_db": finite_db(meter.true_peak_db()), "integrated_lufs": lufs,
        "rms_db": if rms > 0.0 { json!(20.0 * rms.log10()) } else { Value::Null },
        "clipping_samples": clipped, "nonfinite_samples": nonfinite, "warnings": warnings,
        "scope": "Offline render from this range's start with fresh plugin state; includes source inserts, sends, routing, automation and master processing. Track isolation disables other source tracks on a session copy. No pre-roll or automatic effect-tail extension."});
    if audio {
        let opts = EncodeOptions { bit_depth: BitDepth::Int16, ..EncodeOptions::default() };
        let bytes = soundcraft_audio_io::encode(&buf, &opts).map_err(|err| bad(id, err.to_string()))?;
        result["audio_base64"] = json!(b64::encode(&bytes));
        result["mime_type"] = json!("audio/wav");
        result["preview_bit_depth"] = json!(16);
    }
    Ok(result)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use soundcraft_model::{ChannelFormat, Clip, ClipId};

    fn source(e: &mut Engine, name: &str, level: f32) -> TrackId {
        let s = e.session_mut();
        let rate = s.sample_rate.hz();
        let mut buffer = AudioBuffer::new(rate, 1, rate as usize);
        for (n, x) in buffer.channels[0].iter_mut().enumerate() {
            *x = level * (std::f32::consts::TAU * 440.0 * n as f32 / rate as f32).sin();
        }
        let src = crate::io::add_source(s, name, buffer, None, soundcraft_audio_io::FileFormat::Wav);
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some(name));
        let clip = Clip::audio(ClipId(s.alloc()), name, src, 0, 0, i64::from(rate));
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(clip);
        t
    }

    #[test]
    fn preview_is_read_only_and_returns_decodable_audio() {
        let mut e = Engine::default();
        let t = source(&mut e, "Tone", 0.3);
        let before = e.session_arc();
        let revision = e.revision;
        let history = e.undo_history();
        let journal = e.journal.clone();
        let transport = e.transport;
        let v = e.execute("sound.preview", &json!({"track": t.0, "duration": 0.5})).unwrap();
        assert!(v["peak_db"].as_f64().unwrap() < 0.0);
        assert!(v["rms_db"].as_f64().unwrap() < v["peak_db"].as_f64().unwrap());
        assert_eq!(v["mime_type"], "audio/wav");
        let bytes = b64::decode(v["audio_base64"].as_str().unwrap()).unwrap();
        let (_, audio) = soundcraft_audio_io::decode(&bytes, Some("wav")).unwrap();
        assert_eq!(audio.frames(), e.session().sample_rate.hz() as usize / 2);
        assert!(audio.peak() > 0.1);
        assert_eq!(*before, *e.session());
        assert_eq!(revision, e.revision);
        assert_eq!(history, e.undo_history());
        assert_eq!(journal, e.journal);
        assert_eq!(transport, e.transport);
    }

    #[test]
    fn isolation_removes_other_sources_even_if_solo_safe() {
        let mut e = Engine::default();
        let quiet = source(&mut e, "Quiet", 0.1);
        let loud = source(&mut e, "Loud", 0.7);
        e.session_mut().track_mut(loud).unwrap().mixer.solo_safe = true;
        let single = e.execute("sound.analyze", &json!({"track": quiet.0})).unwrap();
        let mix = e.execute("sound.analyze", &json!({})).unwrap();
        assert!(mix["peak_db"].as_f64().unwrap() > single["peak_db"].as_f64().unwrap() + 10.0);
        assert!(!e.session().track(loud).unwrap().inactive);
    }

    #[test]
    fn an_explicit_track_ignores_another_tracks_selection() {
        let mut e = Engine::default();
        let first = source(&mut e, "First", 0.3);
        let second = source(&mut e, "Second", 0.2);
        e.execute("edit.select", &json!({"tracks": [second.0], "start": {"seconds": 10}, "end": {"seconds": 11}})).unwrap();
        let analysis = e.execute("sound.analyze", &json!({"track": first.0})).unwrap();
        assert_eq!(analysis["start"], 0);
        assert!(analysis["peak_db"].as_f64().unwrap() > -20.0);
        assert_eq!(e.session().edit.selection.start, 480_000);
    }

    #[test]
    fn analysis_hears_insert_changes_and_flags_clipping() {
        let mut e = Engine::default();
        let t = source(&mut e, "Tone", 0.5);
        let dry = e.execute("sound.analyze", &json!({"track": t.0})).unwrap();
        e.execute("mix.insert", &json!({"track": t.0, "plugin": "gain", "params": {"gain": 18.0}})).unwrap();
        let wet = e.execute("sound.analyze", &json!({"track": t.0})).unwrap();
        assert!(wet["peak_db"].as_f64().unwrap() > dry["peak_db"].as_f64().unwrap() + 10.0);
        assert!(wet["clipping_samples"].as_u64().unwrap() > 0);
    }

    #[test]
    fn silence_and_short_sounds_have_honest_null_levels() {
        let mut e = Engine::default();
        let v = e.execute("sound.analyze", &json!({"duration": 0.1})).unwrap();
        assert!(v["peak_db"].is_null());
        assert!(v["rms_db"].is_null());
        assert!(v["integrated_lufs"].is_null());
        assert!(v["true_peak_db"].is_null());
        assert!(v.to_string().contains("silence"));
        assert!(!v.to_string().contains("NaN"));
    }

    #[test]
    fn rejects_hostile_ranges_before_rendering() {
        let mut e = Engine::default();
        for p in [
            json!({"duration": 0}),
            json!({"duration": 31}),
            json!({"duration": "1"}),
            json!({"start": -1}),
            json!({"start": 10, "end": 1}),
            json!({"end": {"seconds": 31}}),
            json!({"start": {"seconds": 1e100}}),
            json!({"end": "nope"}),
            json!({"track": 9999}),
            json!({"duration": 1, "end": 48000}),
            Value::Null,
        ] {
            assert!(e.execute("sound.preview", &p).is_err(), "{p}");
        }
    }

    #[test]
    fn catalog_exposes_parameter_ranges_and_rejects_invalid_filters() {
        let mut e = Engine::default();
        let v = e.execute("sound.catalog", &json!({"plugin": "gain"})).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["plugins"][0]["id"], "gain");
        assert!(v["plugins"][0]["params"].is_array());
        assert_eq!(v["external_scanned"], false);
        assert!(e.execute("sound.catalog", &json!({"plugin": "does.not.exist"})).is_err());
        assert!(e.execute("sound.catalog", &json!({"filter": 1})).is_err());
        assert!(e.execute("sound.catalog", &json!({"include_external": "yes"})).is_err());
    }
}
