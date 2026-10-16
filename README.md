# Flowday

A local-first daily planner in the style of Akiflow, with Claude built in. Tasks from
everywhere land in one inbox, you plan the day and drag tasks onto the calendar, and
Claude can do any of that for you. Everything lives in one SQLite file on your machine.

Plain Rust: egui for the window, SQLite for storage, raw HTTP to the Claude Messages API.

## Install

```sh
./install.sh          # downloads the binary CI built, installs it with a launcher entry
./install.sh --build  # or build it here
```

Data: `~/.local/share/flowday/flowday.db`. Keys and tokens go in the system keyring.

## Using it

- **Inbox, Today, Upcoming, Someday, Done**, plus projects and labels in the sidebar.
- **Quick add** understands `Ship release #Work @deep !1 tomorrow 10am 1h every monday`.
  The line under the box shows what it read. Anything it can't parse, send to Claude
  with "✨ Let Claude add this".
- **Time-blocking:** drag a task from the list onto the day calendar on the right; drag
  a block to move it, drag its bottom edge to resize it. Google Calendar events show
  next to your blocks.
- **Plan my day** (P): pick tasks from your inbox and the week, then let the scheduler
  block them into free time between your work hours, most important first.
- **Shutdown:** see what you finished, move what's left, write a line about the day.
- **Claude** (Ctrl J): "plan my day", "move everything about the launch to Friday",
  "find an hour for the dentist next week". It works on your tasks and calendar with
  tools. "✨ Break down" on a task asks it for subtasks.
- **Ctrl K** opens the command bar: go anywhere, search, add a task, run a command.
- Keys: `1`–`5` switch views, `N` adds a task, `←`/`→`/`T` move the calendar day.
- `flowday --add "Call mom tomorrow 5pm"` adds a task from a terminal or a hotkey.

## Integrations (Settings)

| Source | What comes in | Needs |
|---|---|---|
| GitHub | open issues and PRs assigned to you, PRs awaiting your review | a token, or your `gh` login |
| Linear | open issues assigned to you, with due dates | a personal API key |
| Notion | pages in databases you pick, skipping ones marked done | an internal integration token |
| Gmail | starred threads | the Google connection |
| Google Calendar | events a week back to a month ahead; your time blocks are pushed back | the Google connection |

Items sync every ten minutes and when you save Settings. When an item is closed at the
source, Flowday marks it done. For Google, create an OAuth client of type "Desktop app"
in Google Cloud with the Calendar and Gmail APIs enabled, paste its id and secret, and
press Connect.

## Development

`crates/core` (store, quick-add parser, scheduler), `crates/ai` (Claude client and
tools), `crates/sync` (integrations), `crates/app` (the window). Tests, clippy and the
release build run in GitHub Actions on every push.
