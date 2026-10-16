# Flowday: a local-first Akiflow clone with Claude built in

Plain Rust: egui (eframe 0.36.2, same as Calendi) + SQLite. Calls to Claude go over raw HTTP to the
Messages API, because there is no official Rust SDK.

## What Akiflow does, and what Flowday copies
| Akiflow | Flowday |
|---|---|
| Universal inbox fed by integrations | Inbox; Gmail, Notion, Linear and GitHub items land there with a source link |
| Today: task list next to a day calendar, drag tasks onto it to time-block | Same; blocks are tasks with `start` + `duration`, and Google Calendar events are shown next to them |
| Upcoming / week view, Someday, Done | Same |
| Projects (with folders), labels, priorities | Same |
| Natural-language quick add (`tomorrow 3pm 30m #Work !1`) | `core::quickadd`, offline parser; Claude handles input the parser can't |
| Command bar (Ctrl/Cmd K), keyboard-first | Ctrl K palette plus single-key shortcuts |
| Daily planning + shutdown rituals | Guided panes, with Claude drafting the plan |
| Recurring tasks, snooze | rrule (as in Calendi), `snoozed_until` |
| Aki, the AI assistant: chat that creates, plans and schedules | Claude chat panel with tools over the store: list/create/update/schedule tasks, read the calendar, find free time, plan the day, break a task down |
| Two-way calendar sync | Google Calendar: OAuth loopback flow with your own Google Cloud client ID; events are pulled, time blocks pushed |

Out of scope for v1: mobile apps, team sharing, booking links (Calendi already has them).

## Layout
- `crates/core`: SQLite store, model, quick-add parser, free-time and auto-schedule engine. No UI, no network.
- `crates/ai`: Claude client (ureq, streaming off at first) and the tool definitions that run against `core`.
- `crates/sync`: one `Source` trait (pull into the inbox, optionally push back) with Gmail (IMAP + app password),
  Notion (internal integration token), Linear (personal API key), GitHub (`gh auth token` or a PAT),
  and Google Calendar (OAuth).
- `crates/app`: the egui desktop app, binary `flowday`.
- Secrets live in the OS keyring, never in the database.

## Phases (each ends with tests + clippy green and a screenshot)
1. Core store + quick-add parser + scheduling engine, with tests.
2. App shell: sidebar with counts, Inbox / Today / Upcoming / Project views, day calendar with drag-to-block, Ctrl K, quick add.
3. Claude: settings for the key, the chat panel with tools, "plan my day", "break down", NL quick add fallback.
4. Rituals (daily planning, shutdown), recurrence, snooze, Done view.
5. Integrations, one at a time: GitHub, then Linear, Notion, Gmail, Google Calendar (two-way).
6. install.sh, desktop entry, README.

## Progress
- 2026-10-16, session 1: core crate. quickadd parser (9 tests) and store (12 tests) pass, clippy clean.
  `tests/schedule.rs` is written; `src/schedule.rs` (free_slots, auto_schedule, Hours/Slot/Want) is NOT written yet,
  so the next step is to write it and run the tests. Build with `nice -n 19 cargo test -j 2` (the user's PC runs hot).
