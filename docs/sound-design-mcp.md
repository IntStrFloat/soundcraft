# Sound design MCP

Describe **what the sound accompanies**, **how it should feel**, **how long it lasts** and any
existing source to change. The MCP client's language model turns that request into SoundCraft
operations, returns real audio to audition, refines it and exports the result.

Examples:

> Сделай мягкий звук появления заголовка на 0,4 секунды, тёплый, без резкого щелчка.
>
> Нужен тёмный кинематографичный переход на полторы секунды. Дай более тихий вариант.
>
> Из этого клипа сделай короткое ретро-исчезновение, добавь задержку и сравни с оригиналом.
>
> Make this notification more playful, lower the reverb, automate the pitch effect and export WAV.

## Build and connect

Rust **1.95.0** is pinned in `rust-toolchain.toml`: although the original workspace advertises
1.90, its pinned shared dependencies require 1.95. No new crate dependencies were added.

```sh
git clone https://github.com/IntStrFloat/soundcraft.git
cd soundcraft
# If the feature has not been merged into the fork's main branch:
git switch codex/sound-design-mcp
cargo build --release -p soundcraft-cli
```

The headless MCP needs no sound card, desktop app, API key or cloud service. Use the absolute
path to `target/release/soundcraft-cli` (`soundcraft-cli.exe` on Windows) in your client.

For Codex (the desktop app and CLI share the host's MCP configuration):

```sh
codex mcp add soundcraft -- /absolute/path/to/soundcraft-cli mcp \
  --automation-read-root /absolute/path/to/project \
  --automation-write-root /absolute/path/to/project
```

These folders must already exist. Repeat the read/write flags for additional media/output
folders. Restart the client's MCP connection after configuration. See the
[official Codex MCP instructions](https://learn.chatgpt.com/docs/extend/mcp).

Other stdio MCP clients can use:

```json
{
  "mcpServers": {
    "soundcraft": {
      "command": "/absolute/path/to/soundcraft-cli",
      "args": ["mcp", "--automation-read-root", "/project", "--automation-write-root", "/project"]
    }
  }
}
```

To operate the visible editor instead:

```sh
cargo run --release -p soundcraft -- --control 7801
# Configure the MCP client to launch:
soundcraft-cli mcp --connect 7801
```

In the editor, Window → Search also opens **Design Sound** and **Apply Sound Effect Chain**.
Their native dialogs create a variation or append a chosen style to one selected audio track.
Source/track targets are captured when the dialog opens; automated command calls stay dialog-free.

The connection must target the running app's actual port. Automation roots apply only to the
headless engine and cannot be combined with `--connect`. The desktop backend has the same
sound commands plus UI and playback controls. Headless `preview_audio` returns audio to the
client; it does not start playback on the computer's speakers.

## Complete workflow

1. **Inspect/import.** Use `inspect_session`; import user audio with `import_audio`. Select
   source ids explicitly. `session.audio_health` reports missing/invalid media.
2. **Interpret and plan.** The client model interprets the full brief in the user's language.
   `plan_sound` returns a deterministic starting recipe and assumptions without editing.
3. **Create a variation.** `design_sound` creates original procedural audio or a copy of
   `source_clip` on a **new track**. It returns track, clip and source ids. One undo removes
   the complete variation. The original is preserved.
4. **Shape it.** `list_plugins` supplies exact metadata. `apply_effect_chain` appends or
   replaces an ordered chain in one undo step. Append starts after the last occupied slot;
   it fails if the remaining suffix cannot hold the chain. Only reassigned insert slots
   lose their old parameter automation; track volume/trim automation stays. Refine with `set_plugin_params`,
   `set_automation_point`, offline `process_audio`, and the existing editing commands.
5. **Compare and verify.** `bypass_plugin` provides A/B. `preview_audio` returns actual WAV
   audio and analysis; `analyze_audio` measures without returning the binary. Refine and
   audition again. The client should say it listened only if it actually evaluated the audio.
6. **Deliver and save.** `bounce_mix` writes WAV/AIFF/FLAC; `save_session` stores the editable
   session and audio. Reopening reports missing media. Use explicit bounds to include tails.

The `sound_design` MCP prompt packages this workflow. The same guidance is in server
instructions and `soundcraft://sound-design`; `soundcraft://plugins` exposes stock metadata.

### Starting recipes

| Purpose | Typical use |
|---|---|
| `transition` | Scene change, whoosh, movement |
| `reveal` | Logo/object appearance, opening |
| `disappear` | Exit, vanishing, fade away |
| `typography` | Titles, text reveals, typing punctuation |
| `impact` | Hit, accent, impact |
| `notification` | Notification, success cue |
| `ambience` | Background texture, atmosphere |

Vibes: `clean`, `warm`, `dark`, `cinematic`, `retro`, `futuristic`, `playful`, `organic`.
These are editable starting points, **not a limit on styles**: any available processor,
parameter, clip operation, automation lane and routing command can customize them.

For example, the client can call:

```json
{"name":"plan_sound","arguments":{"brief":"Мягкое появление текста","purpose":"typography","vibe":"warm","duration":0.4,"intensity":0.3}}
{"name":"design_sound","arguments":{"purpose":"typography","vibe":"warm","duration":0.4,"name":"Warm title","at":{"seconds":2}}}
{"name":"list_plugins","arguments":{"plugin":"gain"}}
{"name":"apply_effect_chain","arguments":{"track":"Warm title","effects":[{"plugin":"gain","params":{"gain":-6}}]}}
{"name":"preview_audio","arguments":{"track":"Warm title","start":{"seconds":2},"duration":0.8}}
{"name":"bounce_mix","arguments":{"path":"/project/title.wav","start":{"seconds":2},"end":{"seconds":2.8},"bit_depth":"24"}}
{"name":"save_session","arguments":{"path":"/project/title.scraft"}}
```

Each line is the `params` of an MCP `tools/call`, not a shell command. A mix bounce includes
all audible tracks. To export individual clips or stems, discover `file.export_clips` and
`file.bounce_stems` through `list_commands`.

### Full editor and plugin access

`list_commands` + `execute`/`batch` remain the universal surface: trim/split/move, fades and
crossfades, gain envelopes, stretch, pitch, reverse, normalization, routing/sends/busses,
automation, MIDI/instruments, presets, import/export and session management. Discover exact
parameter docs before executing an unfamiliar command. A general batch can stop after a
partial edit; it is **not** a transaction. Design and effect-chain tools are atomic.

`list_plugins({"include_external":true})` discovers installed CLAP/VST3 and, on macOS, Audio
Units supported by SoundCraft. An explicit external plugin id also requests its metadata.
Discovered native plugins can fail to load and are reported unavailable. SoundCraft loads
them in-process: the native host's compatibility and stability limits apply. The MCP does
not install commercial plugins or unlock licensed content. Stock and available external
effects use the same parameter/insert controls; instruments use the engine's instrument
commands. Offline AudioSuite processing supports SoundCraft's built-in algorithms.

## Measurement and resource limits

- Design: 0.03–30 seconds, mono/stereo, at most 16 million channel samples. A source copy
  bakes clip gain/fades/stretch; excludes source-track inserts/fader. It trims or pads to
  the requested duration at the original speed/pitch. New inserts and fades stay editable.
- Preview/analysis: at most 30 seconds and 8 million channel samples, default at most
  10 seconds. Set `start` and either `end` or `duration`; positions use normal session units.
- Queries render on a session copy. A selected source track is isolated from other source
  tracks while retaining sends/aux/master processing; its mute/inactive state still applies.
  Queries preserve the document, undo, journal and transport and create no files.
- Offline audition starts at the requested range with fresh processor state. There is no
  preroll or automatic tail extension. Include extra time explicitly to hear decay/delay.
- Measurements include peak, true peak, RMS, integrated LUFS, clipped and nonfinite samples,
  and silence/missing-media warnings. Undefined levels are `null`, not invented values.
  Integrated LUFS needs at least 400 ms above its gate; it often does not describe a tiny SFX.
- Preview is PCM16 for client compatibility; levels are measured **before** integer encoding.
  Clipping is reported. Use float export when headroom must be preserved. Peak normalization
  is not LUFS mastering. MP3/AAC export and speech transcription are not provided here.
- There is no bundled language model or generative-audio cloud backend. A client model
  handles arbitrary natural language. Direct `brief` matching uses a small RU/EN keyword
  fallback; `interpretation.assumptions` discloses matches/defaults, and explicit fields win.

## Verification

`cargo test -p soundcraft-engine -p soundcraft-automation -p soundcraft-cli` covers the engine
and MCP surface. `crates/automation/tests/it/sound_workflow.rs` exercises the actual lifecycle:
Russian brief → creation → nonzero WAV preview → measured gain/bypass changes → atomic
failure/undo/redo → source variation → offline reverse/automation → bounce/save/reopen.
Module tests cover every purpose/vibe combination, hostile bounds, silence, catalog metadata
and read-only queries. Run the repository gates with `cargo xtask ci`.
