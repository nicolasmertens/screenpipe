# secondbrain — text-first fork of screenpipe

Local "second brain" that journals everything happening on this Mac, all
windows / all screens, ~1Hz. Default mode is text-only and cheap. Audio,
video, and image-vision modes are kept but gated behind a runtime
director that escalates autonomously based on calendar / process / URL /
OCR signals.

## Non-goals

- No cloud, no telemetry, no enterprise features.
- No dependency on `brain.db` (Victor's store) or its compiler. This
  project owns its own SQLite + its own wiki output.
- No always-on video. No always-on audio.
- No iOS parity. iPhone is out of scope until macOS path is proven.

## Architecture

```
+------------------------------------------------------------------+
|                       capture director                          |
|   (calendar | process | URL | OCR-confidence | image-entropy)   |
+------------------------------------------------------------------+
            |        |        |         |          |
            v        v        v         v          v
        text-OCR  audio   short-vid  vision    extractors
       (default) (mode)   (rare)    (1-shot)  (per-app)
            \________\________|________/__________/
                              v
                  ~/Library/Application Support/
                       secondbrain/store.db
                              v
                     wiki compiler (Rust)
                              v
                  ~/Library/Application Support/
                       secondbrain/wiki/*.html
```

## Capture modes (CaptureMode enum)

| Mode             | OCR | Audio | Frame retained | Video |
| ---------------- | --- | ----- | -------------- | ----- |
| TextOnly         | yes | no    | no (ephemeral) | no    |
| TextPlusVision   | yes | no    | yes (1-shot)   | no    |
| TextPlusAudio    | yes | yes   | no             | no    |
| Full             | yes | yes   | yes            | yes   |

Default = `TextOnly`. Director flips at runtime. Mode shows in menubar
dot color. No user confirmation required (local machine, autonomous).

## Director signals

### 1. Calendar (catch-all for in-person / phone / iPhone-joined calls)

- Hourly tick: query EventKit for events in next 60 min.
- If any event in window: dial polling to every minute.
- Arm `TextPlusAudio` 2 min before event start.
- Disarm 5 min after event end.

This catches the cases signal 2 + 3 miss.

### 2. Process watcher (running meeting apps)

Poll `NSWorkspace.shared.runningApplications` every 1s. Trigger
`TextPlusAudio` while any of these are running:

- `us.zoom.xos` (Zoom)
- `com.microsoft.teams2` / `com.microsoft.teams`
- `com.cisco.webexmeetingsapp`
- `com.apple.FaceTime`
- `com.hnc.Discord`
- `com.tinyspeck.slackmacgap` (huddles only — narrow trigger TBD)
- `ru.keepcoder.Telegram` (calls only)

### 3. Browser URL watcher (web-based meetings)

Use existing `browser_urls` table. Trigger `TextPlusAudio` when active
tab matches:

- `meet.google.com/[a-z-]+`
- `zoom.us/j/`
- `*.webex.com/meet/`
- `teams.microsoft.com/l/meetup-join/`
- `whereby.com/`
- `app.gather.town/`

Disarm 60s after URL leaves match set.

### 4. Image-heavy frame fallback

For each captured frame, after OCR:

- If `text_length < 50` AND `image_entropy > threshold` (paintings,
  diagrams, Figma canvas, PDF scans, video stills):
  - Send single frame to Gemini 2.5 Flash for description.
  - Store description in `image_descriptions`.
  - Drop the image.

No video clip needed for this case.

### 5. Manual override

Menubar item + CLI flag (`secondbrain mode <text|audio|vision|full>`)
to force any mode for any duration. Override expires on Quit.

## Storage

New SQLite at `~/Library/Application Support/secondbrain/store.db`.
Independent from brain.db. Designed for this purpose.

### Schema sketch

```sql
-- focus-change-bounded segments (1 per "thing user was doing")
CREATE TABLE segments (
  id            INTEGER PRIMARY KEY,
  started_at    INTEGER NOT NULL,  -- unix ms
  ended_at      INTEGER,
  app_bundle    TEXT NOT NULL,
  app_name      TEXT NOT NULL,
  window_title  TEXT,
  url           TEXT,              -- if browser
  monitor_id    INTEGER,
  focused       INTEGER NOT NULL   -- 1 = frontmost, 0 = background
);

-- raw text extractions, source-tagged
CREATE TABLE extractions (
  id           INTEGER PRIMARY KEY,
  segment_id   INTEGER NOT NULL REFERENCES segments(id),
  captured_at  INTEGER NOT NULL,
  source       TEXT NOT NULL,  -- ocr|dom|whatsapp|imessage|mail|tmux|...
  text         TEXT NOT NULL,
  confidence   REAL,
  raw_json     TEXT            -- for structured sources (DOM tree, mail headers)
);

CREATE VIRTUAL TABLE extractions_fts USING fts5(
  text, content='extractions', content_rowid='id'
);

-- vision-model output for image-heavy frames
CREATE TABLE image_descriptions (
  id           INTEGER PRIMARY KEY,
  segment_id   INTEGER NOT NULL REFERENCES segments(id),
  captured_at  INTEGER NOT NULL,
  description  TEXT NOT NULL,
  model        TEXT NOT NULL
);

-- meeting-mode audio
CREATE TABLE audio_transcriptions (
  id            INTEGER PRIMARY KEY,
  segment_id    INTEGER NOT NULL REFERENCES segments(id),
  started_at    INTEGER NOT NULL,
  ended_at      INTEGER NOT NULL,
  speaker_hint  TEXT,
  text          TEXT NOT NULL,
  engine        TEXT NOT NULL
);

-- Zettel atomic notes synthesized from extractions
CREATE TABLE notes (
  id           INTEGER PRIMARY KEY,
  slug         TEXT UNIQUE NOT NULL,
  title        TEXT NOT NULL,
  body_md      TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  source_segment_ids TEXT  -- json array
);

CREATE TABLE note_links (
  src_slug  TEXT NOT NULL,
  dst_slug  TEXT NOT NULL,
  context   TEXT,           -- surrounding text snippet
  PRIMARY KEY (src_slug, dst_slug)
);

-- entities mentioned across notes (people, projects, urls, places)
CREATE TABLE entities (
  id      INTEGER PRIMARY KEY,
  kind    TEXT NOT NULL,    -- person|project|url|place|company
  name    TEXT NOT NULL,
  slug    TEXT UNIQUE NOT NULL
);

CREATE TABLE entity_mentions (
  entity_id  INTEGER NOT NULL REFERENCES entities(id),
  note_slug  TEXT NOT NULL,
  PRIMARY KEY (entity_id, note_slug)
);
```

## Wiki

Output to `~/Library/Application Support/secondbrain/wiki/`.

### Stack

- Pure static HTML, one file per note + per tag + per entity.
- `cat`-able, `grep`-able, no SPA, no React.
- Compiler is a Rust crate (`secondbrain-wiki`) inside this workspace.
  Reads `store.db`, parses `[[wikilinks]]`, computes backlinks, renders
  via `pulldown-cmark` + `tera`. Watches DB; incremental rebuild on
  new notes (debounced).
- View Transitions API (Baseline 2024) for instant cross-page nav.
  Feels SPA-fast with zero JS framework.
- Client-side search: prebuilt MiniSearch index (one JSON blob,
  lazy-loaded), millisecond full-text.
- Graph view at `/graph.html`: Cytoscape.js, force-directed, click
  node → jump to note. Loaded only on that page.
- Code blocks: Shiki (compile-time, zero runtime).
- Diagrams: Mermaid (lazy-loaded only when present).
- Style: hand-written CSS with `oklch()`, container queries, system
  font stack, dark-mode via `prefers-color-scheme`. No Tailwind, no
  CDN deps. Target <50KB per note inc CSS.

### Page types

- `note/<slug>.html` — Zettel atom + backlinks + outgoing links.
- `entity/<kind>/<slug>.html` — auto-generated entity page with all
  notes mentioning it.
- `tag/<slug>.html` — auto-generated tag index.
- `daily/<YYYY-MM-DD>.html` — auto daily summary (synthesized).
- `graph.html` — full graph view.
- `index.html` — recent notes + search box + graph link.

## Per-source extractors

Built incrementally. Order = highest-leverage first.

### Tier 1 (built first, standalone validators)

- **WhatsApp native** — primary source. `cp` ChatStorage + ContactsV2
  + Labels SQLite to temp every 5 min, diff against watermark, insert
  new messages into `extractions` (source=`whatsapp`). 24K+ messages
  ready on day 1. Pattern lifted from existing iMessage cron.
- **iMessage** — already running on Mac Mini brain pipeline; mirror
  into secondbrain via direct SQLite tap (don't depend on Mac Mini).
- **Apple Mail** — AppleScript or direct `Envelope Index` SQLite tap
  at `~/Library/Mail/V10/MailData/Envelope Index`.

### Tier 2 (extend screenpipe core)

- **Browser DOM** — Safari Web Extension + Chrome/Arc native
  messaging. Sends current-tab readable DOM to local socket on every
  navigation/scroll-stop. Beats OCR for any text-heavy page.
- **tmux** — periodic `tmux capture-pane -p -S -10000` for any
  attached session, shipped to extractor socket.
- **Apple Notes** — `~/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite`
  read-only tap.

### Tier 3 (web shells & secondary apps)

- WhatsApp web-shell ("USA Whatsapp" etc) — DOM scrape via screenpipe
  capture only when focused.
- Slack — DOM scrape (we already have it in process watcher for
  trigger purposes).
- Notion / Linear / Figma — DOM scrape when focused.

## Fork strategy (changes to upstream screenpipe)

### Delete

- `/ee` — proprietary license, must go.
- `crates/screenpipe-connect` — cloud features.
- Sentry init in `apps/screenpipe-app-tauri/src-tauri/src/main.rs`.
- PostHog init in same file.
- `cloud-sync` feature flag in `crates/screenpipe-core/Cargo.toml`.
- `secrets` feature flag in same file.

### Keep dormant (gated by CaptureMode)

- `crates/screenpipe-audio` — kept, but startup gated. Director
  toggles it. Whisper-rs / ONNX deps stay.
- Video chunk writes — kept, gated. Default off.
- JPEG snapshot writes — kept, but ephemeral by default (write → OCR
  → delete unless retained for vision pipeline).

### Add

- `crates/secondbrain-director` — the runtime mode controller. Owns
  EventKit polling, NSWorkspace polling, URL match rules, OCR
  confidence + image entropy thresholds. Emits `CaptureMode` changes
  on a channel.
- `crates/secondbrain-store` — owns the new `store.db`. Separate
  from screenpipe's existing DB. Exposes write API to extractors.
- `crates/secondbrain-extractors` — extractor registry. Tier 1
  (WhatsApp / iMessage / Mail) shipped first, Tier 2 / 3 added
  incrementally. Each extractor is a trait impl with its own poll
  cadence.
- `crates/secondbrain-synth` — segment → Zettel note compiler.
  Calls Claude (via local Anthropic SDK) to atomize segments into
  notes with `[[backlinks]]`. Runs on a debounced queue.
- `crates/secondbrain-wiki` — static-HTML compiler. Watches store.db
  for new notes, regenerates affected pages.
- `crates/secondbrain-vision` — 1-shot Gemini 2.5 Flash caller for
  image-heavy frames.

### Modify

- `crates/screenpipe-screen/src/capture_screenshot_by_window.rs` —
  ensure `capture_unfocused_windows=true` is the default for our
  fork. Background windows sampled at lower cadence (10–30s) than
  frontmost (1s).
- `crates/screenpipe-screen/src/snapshot_writer.rs` — add ephemeral
  mode (write, OCR, delete) gated by CaptureMode.
- `crates/screenpipe-db/src/db.rs` — fork into two DBs: existing
  screenpipe DB stays for internal capture state; secondbrain-store
  is the user-facing store. Or unify under secondbrain-store and
  retire the original — TBD when we touch the DB layer.

## Work order

1. Fork bookkeeping (DONE: branch `textfirst`, this PLAN.md).
2. Rip `/ee`, Sentry, PostHog, cloud-sync, secrets. Verify build.
3. Stand up `secondbrain-store` crate + schema migrations.
4. Build WhatsApp extractor as standalone binary writing to
   `store.db`. Validates extractor interface + store schema before
   touching screenpipe internals.
5. Build `secondbrain-wiki` against the WhatsApp data (synth a
   few notes by hand to test render). Validates wiki stack.
6. Build `secondbrain-director` + integrate with screenpipe capture
   loop. Wire CaptureMode through to existing audio + video
   subsystems.
7. Image-heavy fallback (`secondbrain-vision`).
8. Tier 1 remaining extractors (iMessage, Mail).
9. Synth pipeline (`secondbrain-synth`) — segments to Zettel notes.
10. Tier 2 extractors (browser DOM, tmux, Notes).
11. Daily summary generator.
12. Tier 3 (web shells, Slack, Notion, Linear, Figma).

## Open questions

- DB layout: one unified `secondbrain.db` or split internal/user?
  Decide when we touch `screenpipe-db`.
- Synth model: Claude Sonnet 4.6 via Anthropic SDK on the laptop, or
  route through Victor? Local keeps everything on-device.
- Wiki diff feedback: do we let synth auto-edit existing notes or
  always create new ones + link? Probably auto-edit with a per-note
  changelog at the bottom.
- Slack huddle detection: process-watcher won't catch it cleanly.
  May need a websocket sniff or accept missing it.
