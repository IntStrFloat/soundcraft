//! Purpose-led creation, audition, refinement and delivery through the real MCP surface.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use soundcraft_automation::{Headless, mcp::Server};
use soundcraft_engine::{Engine, model::b64};

fn server() -> Server<Headless> {
    Server::new(Headless::new(Engine::default()))
}

fn rpc(server: &mut Server<Headless>, method: &str, params: Value) -> Value {
    server.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).unwrap()
}

fn raw_tool(server: &mut Server<Headless>, name: &str, arguments: Value) -> Value {
    rpc(server, "tools/call", json!({"name": name, "arguments": arguments}))["result"].clone()
}

fn tool(server: &mut Server<Headless>, name: &str, arguments: Value) -> Value {
    let result = raw_tool(server, name, arguments);
    assert_eq!(result["isError"], false, "{name}: {result}");
    if result.get("structuredContent").is_some() {
        result["structuredContent"].clone()
    } else {
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }
}

fn session(server: &mut Server<Headless>) -> Value {
    tool(server, "inspect_session", json!({"detail": "full"}))
}

fn create(server: &mut Server<Headless>, vibe: &str) -> Value {
    tool(
        server,
        "design_sound",
        json!({"brief": "Мягкий звук появления заголовка", "purpose": "typography", "vibe": vibe,
            "name": "Title sound", "duration": 0.1, "intensity": 0.4, "at": 0}),
    )
}

#[test]
fn sound_tools_prompts_and_resources_are_discoverable() {
    let mut s = server();
    let initialized = rpc(&mut s, "initialize", json!({"protocolVersion": "2025-06-18"}));
    assert!(initialized["result"]["capabilities"]["prompts"].is_object());
    assert!(initialized["result"]["instructions"].as_str().unwrap().contains("preview_audio"));
    let listed = rpc(&mut s, "tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    for name in
        ["list_plugins", "plan_sound", "design_sound", "preview_audio", "analyze_audio", "apply_effect_chain", "process_audio", "undo", "redo"]
    {
        assert!(tools.iter().any(|t| t["name"] == name), "missing tool {name}");
    }
    let preview = tools.iter().find(|t| t["name"] == "preview_audio").unwrap();
    assert_eq!(preview["annotations"]["readOnlyHint"], true);
    let prompts = rpc(&mut s, "prompts/list", json!({}));
    assert!(prompts["result"]["prompts"].as_array().unwrap().iter().any(|p| p["name"] == "sound_design"));
    let prompt =
        rpc(&mut s, "prompts/get", json!({"name": "sound_design", "arguments": {"brief": "Тёмный переход", "output_path": "transition.wav"}}));
    let prompt_text = prompt["result"]["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(prompt_text.contains("Тёмный переход"));
    assert!(prompt_text.contains("transition.wav"));
    assert!(prompt_text.contains("apply_effect_chain"));
    for args in [json!({}), json!({"brief": ""}), json!({"brief": 8}), json!({"brief": "sound", "unknown": "field"})] {
        let invalid = rpc(&mut s, "prompts/get", json!({"name": "sound_design", "arguments": args}));
        assert_eq!(invalid["error"]["code"], -32602, "{invalid}");
    }
    let before = session(&mut s);
    for uri in ["soundcraft://plugins", "soundcraft://sound-design"] {
        let result = rpc(&mut s, "resources/read", json!({"uri": uri}));
        let content = &result["result"]["contents"][0];
        assert_eq!(content["uri"], uri);
        assert_eq!(content["mimeType"], "application/json");
        assert!(serde_json::from_str::<Value>(content["text"].as_str().unwrap()).unwrap().is_object());
    }
    assert_eq!(session(&mut s), before);
}

#[test]
fn brief_to_preview_export_save_and_reopen_is_a_real_audio_workflow() {
    let mut s = server();
    let blank = session(&mut s);
    let plan = tool(&mut s, "plan_sound", json!({"brief": "Мягкое появление текста", "purpose": "typography", "vibe": "warm", "duration": 0.1}));
    assert_eq!(plan["purpose"], "typography");
    assert_eq!(plan["vibe"], "warm");
    assert_eq!(session(&mut s), blank);
    let designed = create(&mut s, "warm");
    let target = designed["track"].clone();
    let before_queries = session(&mut s);
    assert_eq!(before_queries["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(before_queries["tracks"][0]["inserts"][0]["plugin"], "saturator");
    assert!(before_queries["tracks"][0]["clips"][0]["fade_in"].as_i64().unwrap() > 0);
    let analysis = tool(&mut s, "analyze_audio", json!({"track": target, "start": 0, "duration": 0.1}));
    assert!(analysis["peak_db"].as_f64().unwrap() > -70.0, "{analysis}");
    assert_eq!(analysis["clipping_samples"], 0);
    let preview = raw_tool(&mut s, "preview_audio", json!({"track": target, "start": 0, "duration": 0.1}));
    assert_eq!(preview["isError"], false, "{preview}");
    assert!(preview["structuredContent"].get("audio_base64").is_none());
    assert_eq!(preview["structuredContent"]["mime_type"], "audio/wav");
    let audio = preview["content"].as_array().unwrap().iter().find(|c| c["type"] == "audio").unwrap();
    assert_eq!(audio["mimeType"], "audio/wav");
    let bytes = b64::decode(audio["data"].as_str().unwrap()).unwrap();
    assert!(bytes.len() > 44);
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert!(bytes[44..].iter().any(|b| *b != 0));
    assert_eq!(session(&mut s), before_queries, "audition or measurement changed the document/history");

    let directory = std::env::temp_dir().join(format!("soundcraft-mcp-sound-lifecycle-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let wave_path = directory.join("title.wav");
    let session_path = directory.join("title.scraft");
    let bounce = tool(&mut s, "bounce_mix", json!({"path": wave_path.to_string_lossy(), "start": 0, "end": {"seconds": 0.1}, "bit_depth": "24"}));
    assert!(bounce["peak_db"].as_f64().unwrap() > -70.0);
    let delivered = std::fs::read(&wave_path).unwrap();
    assert_eq!(&delivered[..4], b"RIFF");
    assert!(delivered.len() > 44);
    tool(&mut s, "save_session", json!({"path": session_path.to_string_lossy(), "replace": true}));
    tool(&mut s, "new_session", json!({}));
    let opened = tool(&mut s, "open_session", json!({"path": session_path.to_string_lossy()}));
    assert!(opened["missing"].as_array().unwrap().is_empty(), "{opened}");
    let restored = session(&mut s);
    assert_eq!(restored["tracks"][0]["inserts"], before_queries["tracks"][0]["inserts"]);
    assert_eq!(restored["tracks"][0]["clips"], before_queries["tracks"][0]["clips"]);
    assert!(restored["sources"].as_array().unwrap().iter().all(|source| source["loaded"] == true));
    let replay = tool(&mut s, "analyze_audio", json!({"track": target, "start": 0, "duration": 0.1}));
    assert!((replay["peak_db"].as_f64().unwrap() - analysis["peak_db"].as_f64().unwrap()).abs() < 0.01);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn effect_chain_is_atomic_undoable_and_audibly_changes_the_render() {
    let mut s = server();
    // A sustained sound with a 300 ms fade-in lets the gain smoother settle before the peak.
    // A 100 ms typography click peaks during plugin startup and cannot prove settled dB gain.
    let designed = tool(&mut s, "design_sound", json!({"purpose": "ambience", "vibe": "clean", "duration": 2.0, "at": 0}));
    let target = designed["track"].clone();
    let catalog = tool(&mut s, "list_plugins", json!({"plugin": "gain"}));
    let gain = &catalog["plugins"][0];
    assert_eq!(gain["id"], "gain");
    let gain_param = gain["params"].as_array().unwrap().iter().find(|p| p["id"] == "gain").unwrap();
    assert!(gain_param["min"].as_f64().unwrap() <= -12.0);
    assert!(gain_param["max"].as_f64().unwrap() >= 0.0);
    let dry = tool(&mut s, "analyze_audio", json!({"track": target, "start": 0, "duration": 2.0}));
    let original = session(&mut s);
    let failed = raw_tool(
        &mut s,
        "apply_effect_chain",
        json!({"track": target, "effects": [{"plugin": "gain", "params": {"gain": -12.0}}, {"plugin": "not-installed"}]}),
    );
    assert_eq!(failed["isError"], true);
    assert_eq!(session(&mut s), original, "bad chain left a partial edit");
    let chain = tool(&mut s, "apply_effect_chain", json!({"track": target, "effects": [{"plugin": "gain", "params": {"gain": -12.0}}]}));
    assert_eq!(chain["effects"][0]["slot"], 0);
    let quiet = tool(&mut s, "analyze_audio", json!({"track": target, "start": 0, "duration": 2.0}));
    assert!((dry["peak_db"].as_f64().unwrap() - quiet["peak_db"].as_f64().unwrap() - 12.0).abs() < 0.05, "dry={dry}; quiet={quiet}");
    tool(&mut s, "undo", json!({}));
    assert!(session(&mut s)["tracks"][0]["inserts"].as_array().unwrap().is_empty());
    tool(&mut s, "redo", json!({}));
    assert_eq!(session(&mut s)["tracks"][0]["inserts"][0]["plugin"], "gain");
    tool(&mut s, "bypass_plugin", json!({"track": target, "slot": 0, "value": true}));
    let bypassed = tool(&mut s, "analyze_audio", json!({"track": target, "start": 0, "duration": 2.0}));
    assert!((bypassed["peak_db"].as_f64().unwrap() - dry["peak_db"].as_f64().unwrap()).abs() < 0.01, "dry={dry}; bypassed={bypassed}");
    tool(&mut s, "set_plugin_params", json!({"track": target, "slot": 0, "params": {"gain": -6.0}}));
    tool(&mut s, "bypass_plugin", json!({"track": target, "slot": 0, "value": false}));
    let revised = tool(&mut s, "analyze_audio", json!({"track": target, "start": 0, "duration": 2.0}));
    assert!((dry["peak_db"].as_f64().unwrap() - revised["peak_db"].as_f64().unwrap() - 6.0).abs() < 0.05, "dry={dry}; revised={revised}");
    tool(&mut s, "remove_plugin", json!({"track": target, "slot": 0}));
    assert!(session(&mut s)["tracks"][0]["inserts"].as_array().unwrap().is_empty());
}

#[test]
fn source_variation_preserves_original_and_offline_processing_is_reversible() {
    let mut s = server();
    let first = create(&mut s, "clean");
    let original = session(&mut s)["tracks"][0].clone();
    let second = tool(
        &mut s,
        "design_sound",
        json!({"source_clip": first["clip"], "purpose": "disappear", "vibe": "retro", "duration": 0.2, "name": "Variant", "at": {"seconds": 1.0}}),
    );
    let variants = session(&mut s);
    assert_eq!(variants["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(variants["tracks"][0], original);
    assert_ne!(first["clip"], second["clip"]);
    assert_ne!(first["source"], second["source"]);
    tool(&mut s, "undo", json!({}));
    assert_eq!(session(&mut s)["tracks"].as_array().unwrap().len(), 1);
    tool(&mut s, "redo", json!({}));
    let before_process = session(&mut s)["tracks"][1]["clips"][0].clone();
    let processed = tool(&mut s, "process_audio", json!({"process": "reverse", "clips": [second["clip"]]}));
    assert_eq!(processed["processed"], 1);
    let after_process = session(&mut s);
    assert_ne!(after_process["tracks"][1]["clips"][0]["source"], before_process["source"]);
    assert_eq!(after_process["tracks"][0], original);
    tool(&mut s, "undo", json!({}));
    assert_eq!(session(&mut s)["tracks"][1]["clips"][0], before_process);
    tool(&mut s, "set_automation_point", json!({"track": second["track"], "param": "volume", "at": {"seconds": 1.0}, "value": -12.0}));
    assert!(!session(&mut s)["tracks"][1]["automation"].as_array().unwrap().is_empty());
    assert_eq!(session(&mut s)["tracks"][0], original);
}

#[test]
fn malformed_calls_and_oversize_auditions_report_errors_without_mutation() {
    let mut s = server();
    create(&mut s, "clean");
    let before = session(&mut s);
    let target = before["tracks"][0]["id"].clone();
    for (name, args) in [
        ("design_sound", json!([])),
        ("design_sound", json!({"duration": 31.0})),
        ("design_sound", json!({"purpose": "not-a-purpose"})),
        ("preview_audio", json!({"duration": 31.0})),
        ("preview_audio", json!({"start": -1, "end": 40})),
        ("preview_audio", json!({"duration": 0.1, "end": 4800})),
        ("analyze_audio", json!({"track": "Missing"})),
        ("list_plugins", json!({"plugin": "not-installed"})),
        ("bypass_plugin", json!({"track": target, "slot": 0, "value": "false"})),
        ("bypass_plugin", json!({"track": target, "slot": 0})),
        ("remove_plugin", json!({"slot": 0})),
        ("set_plugin_params", json!({"slot": 0, "params": {"gain": -6}})),
        ("process_audio", json!({"process": "reverse", "clips": [before["tracks"][0]["clips"][0]["id"], "invalid"]})),
        ("design_sound", json!({"duraton": 0.1})),
    ] {
        let result = raw_tool(&mut s, name, args);
        assert_eq!(result["isError"], true, "{name}: {result}");
        assert_eq!(session(&mut s), before, "{name} changed the session on failure");
    }
}
