# Roadmap

The shared plan for reaching the product described in the README, for people and agents alike.
`docs/STATUS.md` says what works today; this file says what comes next, who owns it and what an
agent may pick up; [roadmap-history.md](roadmap-history.md) keeps what's finished. Rebuilt on
2026-11-02 from `main` at `2e4afc5`, with v0.3.0 the latest release.

## How to use this file

- Each item has an **id** (`R<milestone>.<n>`), an **owner**, a **status** and a list of
  **pending changes**. Pending changes are small, independently mergeable units. Ids are never
  reused or renumbered: commits, code comments and PRs quote them.
- **Claiming:** before you start, put `— claimed: <name>, branch <branch>` on the pending change
  and push your branch early (or open a draft PR) so others see it. Don't take a change someone
  else has claimed. `— in PR #n` marks one a PR already covers: review that PR rather than starting
  again. The full working rules are in `AGENTS.md` ("Working rules", mandatory). Items owned by
  **maintainers** are steered by the project maintainers: agents may take their *unclaimed pending
  changes*, but design decisions marked **decision** wait for a maintainer.
- **Finishing:** tick the change (`[x]`) where it is, add the commit or PR, update `docs/STATUS.md`
  if what works changed, and add a line to `log/devlog.md`. Maintainers move ticked lines to
  `roadmap-history.md` when a release ships.
- **New work:** add a `- [ ]` line under the item it belongs to; a new item takes the next free id.
- **Status:** `todo` · `in progress` · `blocked (<why>)` · `done`.
- Follow `AGENTS.md` (layering, never crash, everything is a tool, `platform::process` for every
  external program) and the architecture rules below.

## Architecture rules for new work

The project should be easy to fork and bend. New features land behind a seam, not inside a
`match`:

1. **Traits plus a registry, not enums.** Target kinds, checks, builders, asset providers and
   external engines are traits with a registry that holds built-in implementations. Adding one
   means adding a file and one registration line (as tools already work).
2. **Data over code for catalogues.** Stacks, game-engine build recipes, model ids and prices,
   and engine install pins live in typed tables (or TOML files embedded with `include_str!`), not
   in string matches or prompt text.
3. **Headless first; flows are tools.** Everything works without the GUI. Anything a person
   can start from the UI (rebuild, follow-ups, answering questions, engine installs, launching a
   result) is reachable through `Engine::call` and the CLI, so MCP, agents, scripts and other
   frontends get it for free. Orchestration, persistence and process control never live in
   `ui-egui` or `apps/*`.
4. **Versioned files.** Every file we write and read back (`craft.json`, `config.toml`,
   `project.toml`) carries a version, has a JSON schema, and has a migration path.
5. **Structured results.** Decisions such as "did the build pass" read typed fields, never
   English text prefixes.

## Milestones at a glance

| Milestone | Goal | Owner | Where it stands (2026-11-02) |
|---|---|---|---|
| M0 Foundations | Docs match the code, licences checked, one live end-to-end run | maintainers | Done |
| M1 Extensibility | Seams for target kinds, checks, builders, asset providers, engines, plugins | maintainers | Done, except engine pins (R1.5) |
| M2 Product completeness | Every kind of target gets study → build → check → refine with a real check | maintainers | Websites complete; apps and games build and run; API-key builds have never run live (R5.5); agents other than Claude Code are only partly wired (R2.2) |
| M3 Engine depth | More tools, and the known partial ones | open | 314 tools; the open lane |
| M4 Release | Packaging, signing, a managed tier | maintainers | v0.3.0 released, signed, on every platform; a macOS CLI question and the managed tier remain |
| M5 Works for real people | What people hit now that the project is public | maintainers | Comes first |

## Open work

Every unticked `- [ ]` line in this file is open work, under the item it belongs to.
`cargo xtask pending` lists them all, grouped by item (`--count` for the totals). When you find
work that's missing, add it as a `- [ ]` line under the right item in the same PR, rather than
leaving it in a PR description or a chat.

## Order and dependencies

Work in phases. Items in the same phase can run in parallel. A phase says what comes first; what
stops an item from starting is its **Depends on** column, which must be done first. A *soft*
dependency should come first but doesn't block.
Issues are ranked by people affected and how badly, after leaving out what's already fixed.

| Phase | Item | Depends on | Why here |
|---|---|---|---|
| 0 Ship | R5.1 The next release | — | The Windows start-up crash and a build-breaking resume bug are fixed on `main` but not released |
| 0 | R5.2 Keeping contributions moving | — | `main` isn't protected, first-time contributors' CI is held, and a waiting PR fixes a safety bug |
| 1 Builds that work | R5.3 A builder that can't make progress stops | — | Costs people money or quota on every backend; small |
| 1 | R5.4 Local models can build | R5.3 *(soft)* | Largest open gap, with no fix anywhere yet |
| 1 | R2.2 Every builder gets the same tools (that group), and Approvals | **decision** for Approvals | Agents other than Claude Code start but can't do the job, and Clean room's rule doesn't reach them |
| 1 | R5.5 API-key builds fail in words people can use | — | This path has never been run live, and its errors say nothing |
| 1 | R5.6 Tools report only what they saw | — | Confirmed false successes on macOS; golden rule 3 |
| 2 Proof | R5.7 Live checks owed | — | Built but never run for real; each can reopen work above. Lines run when the machine or key they need is at hand, Clean room and the quick start first |
| 3 Depth | The rest of R2.2 | — | Detection waits for reports from 0.3.0 or later; ACP after the adapters work |
| 3 | R2.4 Runtime study, R2.5 Behaviour checks | scripted input (R2.4) before R2.5's sheet check | Makes "ready" mean more for apps and games |
| 3 | R2.7, R2.8 | — | Small gaps left in finished work |
| 3 | R2.9 Everyday use, R2.10 Pinned builds | — | Open to anyone at any time; most lines stand alone |
| 3 | R1.5 | — | Small leftovers |
| Any | M3: R3.3, R3.9, R3.11 | — | Open lane |
| Later | R4.1 macOS CLI, R4.2 Managed tier | **decision** | Not before M5 |

---

## M5 Works for real people

### R5.1 The next release
Owner: maintainers · Status: todo
Two reported failures, the Windows start-up crash (#414) and a builder resuming another's
session (#471), are fixed on `main` and still broken for everyone on 0.3.0. This is the
cheapest help we can give anyone, so nothing below should hold it up for long.
- [x] #490 merged (`549ae5e`) with the CLI fix the review asked for: a string-or-object input is
      `any`, so a JSON object given to `--left` stays an object (tested both ways). What it reports
      is 0.2.0's, fixed by #93 (`f6725c2`): on Windows the
      published 0.3.0 CLI with Claude Code 2.1.179 mounted 314 of 315 tools (the 315th is the
      approval tool, which Claude Code doesn't list but calls), and a real Deep build reached Ready
      with `core_action_approve` answering five prompts. The reporter was asked for `--version`.
      macOS is still owed (R5.7)
- [ ] Cut the next release (`docs/releasing.md`). The version is 0.4.0 (#526) and `main` at
      `fb7ac86` is green, but pushing `release` is refused: its ruleset blocks every update and
      lists no one who may bypass it. A maintainer who can push it (or adds a bypass) goes next.
      0.3.0 lacks `0748ef1` (#414: Windows starts DX12 only, the crash behind "it won't open"),
      `1e8f661` (#471: a builder never continues another builder's session) and `3357daf` (#430:
      Claude Code's message on stdin)
- [ ] See that the launcher offers the new version: #74, #104 and #420 installed through it, and
      nothing in this repository says how it picks one
- [ ] After it ships, close what it fixes: #166, #346, #420, #478. #74, #104 and #222 give no
      detail, so they may have another cause: keep them open until their fixes are validated
      against the published release, following `docs/releasing.md`.
      Ask #201 whether `3861132` (#379) ended its early compression (a guess at the link)
- [ ] Close the tickets 0.3.0 already fixed, and the spam: #47, #53, #66, #95, #112, #113, #130,
      #133, #138, #152, #179, #182, #185, #202, #214, #329, #344, #377, #378, #400, #449; spam #194,
      #195. #113, #329 and #377 are 0.2.0's missing CLI, fixed by `1b54cbf`, not what #490 reports.
      Not yet: #64 (its cause is fixed, its retries aren't: R5.3), #142 (R5.3), and #462, which says
      `none`, not #490's failure, and gives no version: ask for it
- [ ] Sometimes starting the app shows nothing: after the release, see from `desktop.log` whether it
      still happens. DX12-only may have ended it; a GOG Galaxy Vulkan overlay layer was seen loading
      into the app
- [x] A program that is found but can't start (an npm script whose `node` is missing) says where it is and
      what to look at, not "isn't installed" (#152, #95); on Windows a program found is started by its
      path, not its name
- [ ] Live-check #152 on Windows 11 with OpenCode from npm, and on a machine with the agent but no
      Node.js
- [x] A start that dies before its window (a native driver crash leaves no dialog or log, #74, #104,
      #33) is remembered: `starting` beside `desktop.log` is written before the window is asked for
      and removed once it's open, and a start that finds it draws with WARP that once
      (`apps/craft-anything/src/start_guard.rs`)
- [x] Live-check the start fallback on Windows: make a start die before its window (kill the
      process at launch), start again, and see `desktop.log` say it drew without the graphics card
      and log `drawing with` an adapter of type `Cpu` (WARP), and that the window opens. Passed on
      2026-11-04 (`main` at `869eb85`, release build, Windows 11, RX 6900 XT): a start killed right
      after `drawing with` the Radeon left `starting`; the next logged "the last start never opened
      its window", drew with "Microsoft Basic Render Driver" (`Cpu`, DX12), logged `window open` and
      removed `starting` — claimed: 00xJS, branch docs/r5.1-start-fallback-live-check
- [ ] The software retry still lets wgpu start every DX12 adapter before `pick_adapter` runs, so a
      driver that crashes while its device is created crashes the retry too. wgpu has no way to
      skip hardware adapters; needs a different route (another renderer, or a helper that starts
      the app with only WARP visible)
- [x] #794: on some systems DX12 refuses egui's own pipeline (`egui_pipeline`, 0x80070057 from
      `CreatePipelineState`) on AMD, Intel and NVIDIA cards and on WARP too (lightcraft#812,
      pdfcraft#1046, wgpu#10771), so the software retry failed the same way every start, and the
      dialog only said to update the driver. Each failed start now moves on to the next renderer
      (DX12 → WARP → Vulkan → GL, `apps/craft-anything/src/renderer.rs`), keeps the one that worked
      for this app version, and starts again by itself after a wgpu error. Checked with a build
      that failed every DX12 start; the real failure wasn't reproduced (STATUS.md)
      — claimed: 00xJS, branch bugfix/r5.1-renderer-fallback
- [ ] Live-check #794's fallback on a machine where DX12 refuses `egui_pipeline` (the reporter's
      780M, or lightcraft#812's)
- [x] A window that fails after it was shown no longer says "The next start will draw with …": its
      start mark is already gone, so the next start draws as before (#909 review); the dialog says
      it only when the window was never shown (`next_start_note`) — claimed: 00xJS, branch bugfix/r5.1-retry-text-after-shown
- [x] Add to `docs/releasing.md`: before publishing, a Deep build through Claude Code from the
      finished package, on a Mac and on Windows. Starting the CLI from the package (#91) doesn't
      show whether Claude Code accepts its tools — PR #982; claimed: doubektomas,
      branch docs/r5.1-packaged-deep-build-check. The checklist is documented; live package runs
      remain release validation work.
- [x] Add to `docs/releasing.md`: after a release, close every ticket marked as waiting for it, so
      they don't stay open marked `fixed` — PR #1006; claimed: doubektomas,
      branch docs/r5.1-post-release-tickets. The checklist is documented; closing tickets waits
      for publication of the release containing their fixes.

### R5.2 Keeping contributions moving
Owner: maintainers · Status: in progress
- [x] Fix PR #1030's Windows descendant fixture startup and stop the watched launch before
      startup assertions can fail; cover the failure cleanup path
      — claimed: Codex, branch bugfix/r5.2-watched-launch-tests
- [x] Run PR #1030's shell-free parent/descendant fixture and startup-timeout cleanup test
      on Windows; the contributor reproduced startup failure and leaked processes on 07671fe.
      Verified on Windows 11: `cargo test -p craft-anything-platform --lib process::launch_tests`
      passed five runs (11 passed, 6 ignored, the six being the stand-ins),
      `stopping_a_watched_program_stops_its_descendant` and
      `a_descendant_startup_timeout_stops_the_watched_launch` among them; after the runs and a 35 s
      wait `tasklist` showed no `PING.EXE`, and the only `cmd.exe` and `powershell.exe` were ones
      already running before the first run
      — claimed: muraa-p, branch test/r5.2-watched-launch-windows-check
- [x] Clarify PR #1030's test Rustdoc: post-stop heartbeat sampling, child-fixture guard,
      and the direct-run no-write check
      — claimed: Codex, branch bugfix/r5.2-watched-launch-tests
- [x] Address PR #1030 review: take the heartbeat baseline after stopping, guard the descendant
      stand-in against direct ignored-test runs, and document the test helpers
      — claimed: Codex, branch bugfix/r5.2-watched-launch-tests
- [x] `regression_preview_limits_connections_and_releases_listener` (platform) fails on macOS
      with `Invalid argument` in its socket fixture, also reported before PR #1030's review fixes.
      The fixture set its excess clients' read timeout after dropping the server, and macOS
      refuses socket options on a connection it has reset; the fixture now sets them when connecting
      — PR #1207; claimed: rmsff, branch test/r5.2-macos-preview-socket-fixture
Review comes before new work while the queue is long, smallest and clean PRs first. A PR is merged
only when it's green and a maintainer has fully reviewed it, code as well as wording.
- [ ] Maintainers: protect `main` (PRs required, squash only); the only ruleset covers `release`
- [x] `cargo xtask secrets` treats every environment variable whose name contains `AUTH` as a
      secret, git's `GIT_AUTHOR_NAME` among them, so the pre-commit hook refuses any added line with
      the committer's own name when it's 12 characters or more (a roadmap claim by a contributor
      whose git name is their handle). Leave git's identity variables out of `env_values`
      (`NOT_SECRET`); `OAUTH`, `AUTHORIZATION` and other `AUTH` names are still checked
      — claimed: youkaicountry, branch bugfix/r5.2-secrets-git-identity
- [x] Auto mode's delete check is a safety bug that shows up as Mac test failures. It splits a
      command on spaces, so `rm -rf "C:/x/Game Backup/y"` from a project at `C:/x/Game` goes ahead
      unasked, and it lower-cases paths, so on a Mac or Linux a delete inside a project with
      capitals in its path is refused; `auto_mode_keeps_going_but_never_does_the_dangerous_things`
      fails on a Mac for that. `real_binary_end_to_end_if_present` fails there too, expecting
      `/bin/ls` to be Linux: both fixed (#357)
- [ ] **decision** First-time contributors' CI: GitHub holds their runs for approval, so their PRs
      show no `gate` run (#490 and #413 were among them). Approve each push, or
      change the repository's approval setting
- [ ] #193 changes the language list that #486 (merged) changed too: it needs a rebase. #468 looks
      redundant with #500: ask its author
- [ ] **decision** Wide PRs with no roadmap line: #416 (project goals, 34 files, cites an R2.11 that
      doesn't exist; that id stays free for it) and #503 (two asset providers and spending controls,
      37 files, cites the finished R1.4). Decide whether we want them before more is built on them
- [ ] **decision** Replies don't reach people who file through the ticket relay (each such ticket
      says so), yet the bot asks them for details. About half the open issues came that way. Get
      replies back to them, or stop asking
- [x] `which::tests::usual_windows_dirs_come_after_path` (platform) failed on Windows (10 found, 9
      expected, "nothing twice"): it didn't count Antigravity's second folder, fixed by #770
- [ ] `capture::console_tests::reads_what_the_program_drew_and_stops_at_the_deadline` failed once
      with the whole platform suite running and passes alone (passed 3 whole-suite runs on Windows 11
      on 2026-11-04). Find what it depends on
- [x] `process::launch_tests::a_launcher_that_hands_off_keeps_its_group_until_stopped` failed on a
      busy Windows machine: it killed the run 500 ms after the start, and a `cmd` that hadn't exited
      by then made a cancelled run, not a hand-off. The test now waits for the launcher's mark, and
      a kill that finds the program already exited reports its exit, not a cancel
      — claimed: 00xJS, branch test/r5.2-hand-off-race
- [ ] `website_tools::tests::a_browser_that_exits_at_once_is_reported_at_once` (engine) failed once
      on Windows with the whole engine suite running (66 s under load) and passes alone, three
      times in a row. Find what it times
- [x] `a_remote_query_expands_only_the_remote_working_directory` compares the shell's `$PWD`
      with the fixture's canonical path, so macOS's `/var` → `/private/var` alias does not fail
      the test. Remote expansion and special-character quoting checks are retained — PR #949
      — claimed: Codex, branch test/r5.2-macos-remote-query-path
- [x] `which::tests::usual_windows_dirs_come_after_path` (platform) failed on Windows with 10
      found, 9 expected ("nothing twice"): #748's second Antigravity folder, counted since #770
- [x] `capture::console_tests` (platform) timed out now and then with the whole platform suite
      running. On Windows Server 2022 a program another thread starts while a console opens
      inherits its handles and holds it open until it ends, and closing a console that programs
      are still attached to can hang its host. Program starts and console opens (and Unix terminal
      opens, which have a shorter gap of the same kind) now take one lock (`process::STARTING`),
      and a timeout ends the program's whole tree before closing
      — claimed: SoldierPePe, branch test/r5.2-console-capture-flake
- [ ] A console program that exits by itself and leaves programs attached to its console: closing
      it can still hang the console host on Windows Server 2022 (only a timeout ends the tree)
- [x] `process::launch_tests::a_watched_program_can_be_stopped` leaves its `sleep 30` (Windows:
      `ping -n 30`) running, on `main` too: `stop` ends the shell but not what the shell started
      (#1027). Verified on Windows 11: after the test (5 runs) and after the whole
      `craft-anything-platform` suite plus a 35 s wait, `tasklist` showed no `PING.EXE`. Not
      checked: a panic before `stop`, dropping the handle without `stop`, Windows 10 or Server
      — claimed: yashpatadiya1724-cmyk, branch docs/r5.2-watched-stop-windows-check
- [x] `a_helper_is_judged_by_whether_it_runs` (ai crate) sometimes failed when several crates'
      tests ran at once: a script just written can't start while another thread's child still
      holds it open ("Text file busy"). #787 tries again: on Linux 4 of 72 runs failed before
      it, none after
- [ ] Revisit the two advisory exceptions in `deny.toml`: `rsa` (no fix; we only verify with public
      keys) when rsa 0.10 lands, and unmaintained `zbus_names`/`zvariant` (Linux-only via keyring
      and accesskit) when those move on. `deny.toml`'s comments pointed at R0.3, which never held
      them; they name this line now — claimed: kuseidos, branch docs/r5.2-deny-points-at-its-own-roadmap-line
- [ ] Check whether the release workflows still run actions that target Node.js 20. None is named
      here any more: every job calls `storytold/craft-common-actions@main`, so a fix belongs there
- [x] Correct STATUS.md's obsolete GUI-only limit: document the shared project feed, engine
      setup, launch and preview tools, with their consent and lifetime limits — PR #1014
      — claimed: Codex, branch docs/r5.2-headless-status
- [x] A spinner on screen no longer makes the window redraw at the display's rate: egui's own asks
      for a frame at once every frame, ours at the animation's pace (`widgets::Spinner`, #579)
- [x] A job whose progress isn't known no longer makes the window redraw at the display's rate:
      the jobs list used egui's animated `ProgressBar`, which asks for a frame at once every frame
      like egui's spinner did (`widgets::ProgressBar`)
      — claimed: Claude, branch bugfix/r5.2-paced-progress-bar
- [ ] Measure #579 on Windows 11 (0.3.0 portable): the app's CPU while idle, while a build runs and
      during a website capture, before and after; and why a website capture is slow and sits for a
      long time (no details were given, so nothing was changed for it)
- [x] `remote_discovery_uses_the_remote_program_and_connection_context` fails on a computer with
      OpenCode installed: its made-up path is missing, so `program_of` falls back to `opencode`
      — claimed: justinjohn0306, branch test/remote-discovery-without-opencode
- [x] Idle CPU on an AMD Radeon, measured (2026-11-04, `main` at `869eb85`, release build, Windows 11
      build 26300, RX 6900 XT, driver 32.0.21045.16010, Ryzen 5 2600X; the window open on a fresh
      profile, nothing changing). DX12 on the card: one core at ~100% for as long as the window is
      open; Vulkan: the same; GL: 0%; WARP: ~280% for the first 5 s after start, then 0%. The busy
      thread isn't the app's: on DX12 it starts in `amdxc64.dll`, on Vulkan in `amdvlk64.dll`, and
      it starts with the device, not the main thread. A bare eframe window does the same with every
      present mode and frame latency tried (AutoVsync, Fifo, Mailbox, Immediate; latency 1, 2, 3),
      so nothing in our frame pacing causes or avoids it; GL alone stays idle. Seen in the same way
      with PdfCraft and PhotoCraft (pdfcraft#3255) — claimed: 00xJS, branch docs/r5.2-idle-cpu-amd
- [ ] **decision** On the one Radeon measured (RX 6900 XT, driver 32.0.21045.16010, Windows 11
      build 26300), AMD's idle driver thread costs a core on DX12 and Vulkan; other Radeons
      (integrated ones too), drivers and Windows builds haven't been checked. Say so in the
      troubleshooting docs (with `WGPU_BACKEND=gl` as the way out, which loads AMD's OpenGL
      driver), report it to AMD, or leave it
- [x] Limit the decision line above to what was measured (one RX 6900 XT, one driver, one Windows
      build) and say other Radeons and drivers aren't checked (#912 review)
      — claimed: 00xJS, branch docs/r5.2-radeon-idle-scope
- [x] STATUS.md has rows twice, kept by squash merges of PRs that edited the same row: OpenCode
      models and thinking (three copies that disagree about SSH), Claude desktop app without Claude
      Code, Websites as targets, Desktop app and CLI; `docs/opencode.md` has a paragraph twice.
      Keep the newest complete copy of each
      — claimed: youkaicountry, branch docs/r5.2-doubled-status-rows — PR #1226
- [x] Have `cargo xtask` refuse a STATUS.md table with two rows of the same name, so a merge
      that keeps both copies of an edited row fails CI instead of reaching `main`
      — claimed: Afloat16, branch test/r5.2-detect-duplicate-status-rows
- [x] `docs/opencode.md` still has one passage twice, missed by #1226: in "Configuration and
      execution", the older copy of the standalone-server sentence, cut off after "both"
      — claimed: youkaicountry, branch docs/r5.2-opencode-doubled-passage
- [x] Let a UI change land with its new strings in English while native speakers review their
      translations: `cargo xtask i18n-coverage` accepts strings listed as awaiting translation,
      and the translation PR empties the list
      — claimed: tanish2k09, branch chore/r5.2-awaiting-translation
- [ ] `tools::android_tools::jeb::tests::not_configured_refused_and_remote_urls` fails now and
      then: it binds port 0, drops the listener and expects nothing to answer there, while the
      stand-in JEB servers of the tests beside it bind port 0 too and can be handed that port.
      Seen once in six whole-suite runs on Windows 11: `engines_jeb_check` got a stand-in's answer.
      Give the negative probe an address no other test can take

### R5.3 A builder that can't make progress stops
Owner: open · Status: todo
Every round spends the person's money or quota. One that can't do anything shouldn't start.
- [x] Preserve the interrupted request when an agent cannot restore its session: ACP starts a new
      session, or a registered CLI adapter falls back to Plain (#552 review).
      — claimed: Afloat16, branch bugfix/r5.3-interrupted-request-recovery-20261103
- [x] A build stops and says why after two builder turns in a row that changed nothing in `app/`,
      on every backend (#44, #110, #417, #481). The count is saved in the project, so it holds
      across runs: a run ends at 7 checks (`max_rounds`) but check numbers carry on, so #44's check
      28 and #417's 35+ look like several runs (a guess: Try again is the only restart found, and
      Auto mode doesn't start runs) — implementation: `2fc2970948315b64d47359543b458509da42ab3f`
- [x] A builder whose tools didn't start isn't resumed. `turn` (`rebuild/mod.rs`) tries 3 more times
      ("Picking up where it left off", #377) because `worth_retrying` doesn't know this error. Give
      it a typed signal, as `Conversation::session_lost` does, not one more word in that function's
      list. Say what failed and stop (pitfalls: never retry what can't succeed) — commit `f80a16a66834`
- [x] `worth_retrying` decides on English words in a message (AGENTS.md: structured results). Move
      its other cases (sign-in, credit, a missing install) to the same typed signal — commit `f80a16a66834`
- [ ] Find what repeated #64's attempts and stop it: it logged 41, about 38 s apart, on 0.2.0, but
      `turn` allows 3 retries 30, 60 and 90 s apart, there too, so something else starts them
- [x] A rate limit is waited for only on agents with a session: `turn` retries only when
      `agent.session_id()` is set, which API backends and OpenCode's plain adapter never set (read
      in the code, not run). So #148 didn't reach #142, which was OpenCode. Wait and send the turn
      again for them too, then close #142 — commit `f80a16a66834`
- [x] A builder step on an API backend stops after 25 failed build commands in a row (none
      succeeding between), says so and keeps the project (`MAX_FAILED_BUILDS` in `agent.rs`, #399:
      9.4M tokens in 19 minutes of the first stage, never compiled). Try again carries on
- [ ] **decision** Whether a build gets a token or cost limit of its own, beyond rounds and that
      stop (#399): a model that changes code without ever running a build isn't caught by it
- [ ] Check live that the stop fires on a real API-backend build and the message reads well. It
      fired on a local Ollama build (the same built-in loop) after two turns that changed nothing
      in `app/` ("The build stopped after two builder turns changed no files in app/…"); an
      API-key build is still owed
- [x] #736: the build check and the stop message say when `app/` is empty, so a builder that
      claims a build can't hide that nothing was written. v0.3.0 (the reporter's release) had no
      stop after two unchanged turns (#557, merged after it); main stops, and this adds the
      empty-folder wording to the build check (it failed first, before #749's website check) and
      to the stop message — claimed: claude, branch bugfix/r5.3-empty-app-loop

- [x] A build whose checks keep finding no working build stops after 10 in a row, across "Try again"
      presses, until a new request starts the count again (`NOT_WORKING_CHECK_LIMIT`, kept in craft.json as
      `not_working_checks`). "Try again" used to start six more rounds every time with nothing counted across
      runs, so #736's build went on to check 40 and 49 times. A run at the limit stops before a builder turn.
      The unchanged-turn guard already stops a builder that writes nothing; #736's exact Windows run (gemini-cli)
      is not reproduced here. An empty `app/` is named as having no files yet (#771's wording) and the builder is asked to
      write the project and show its build's output, not to fix a build (`NO_PROJECT_PROMPT`). — claimed: fabox-os, branch bugfix/736-stop-empty-build-loop
- [ ] Live-check #736 on Windows with gemini-cli: an empty app/ and a builder that claims `npm run build`
      stops after two turns (or 10 checks) with the "no files yet" note, and Try again doesn't restart it
- [x] A finished builder turn whose own CLI process lingers after its stream already said it was
      done (e.g. a stuck shutdown hook) no longer leaves the build on "Building"/"Pausing" for as
      long as the idle timeout allows (hours, for a builder turn): `run_cli` ends the turn
      `RESULT_GRACE` after its terminal `result` line instead of waiting for the process itself to
      exit. A wall or idle deadline that fires during that grace period (the result was already
      parsed) counts as completion, not a failure; a deadline with no parsed result still stops the
      turn as before — claimed: mytrade2909-design, branch bugfix/builder-stuck-after-turn-completes, PR #1426
- [x] A program that's stopped (time limit, pause, or leftovers after it exits) is waited for
      only 5 s: stopping it used a wait with no limit, the one step of a run that neither its time
      limit nor Pause could reach (#1438, #981: a check that sits for hours with Pause dead). Found
      by reading the code; the reporters' hangs are not reproduced here
      — claimed: justinjohn0306, branch bugfix/r5.3-stopped-program-wait
- [ ] Confirm on a reporter's Windows machine that #1438 / #981 no longer sit at a check (their
      `build-activity.log` after the stall, and whether "still there 5 s after it was stopped" is
      in the app's log). `Launched::stop` (Play) and the console capture still use the same
      unlimited `kill()`

### R5.4 Local models can build
Owner: open · Status: todo
The built-in agent loop assumes a capable tool-calling model. Reports #24, #44, #110, #403, #417 and
#498 say otherwise (#44's backend is a guess; #116 may be the same), and no local model build has
been run live.
- [x] A live local build through Ollama, small target, to Ready or to a clear stop, and note in
      STATUS.md which models it worked with: Ollama 0.40.7 with `qwen2.5-coder:14b` on Windows
      (RTX 5090, 32k context, no prompt cut) stops clearly at "No project to build found in app/
      yet", twice. The lines below say why
- [x] A builder on a local model is offered 13 tools (about 10,000 characters of descriptions)
      instead of 33-41 (23,000-30,000): the Build tools and the Core tools the brief names, not the
      study set a model explores with instead of writing (#417, `LOCAL_BUILDER_CORE`). Measured as
      text, not on a model; Ollama's default context (4k) was too small for the old list alone
- [x] Run #417's case live (Ollama, `qwen2.5-coder:14b`, "a scientific calculator") before and
      after this: does it call `build_file_write` now? If it still doesn't, the model can't build.
      Yes, with the three fixes below: over about 50 calls it wrote `app/Cargo.toml` (eframe), fixed
      the two errors the checks reported, and check 3 said "Build succeeded". But `src/main.rs` was
      still cargo's "Hello, world!": no calculator was written, and the screenshot check rightly
      failed it at the round limit (#753)
- [x] The system prompt told every model to call `core_brief_write` and `core_plan_update` first and
      `core_handoff_write` last, tools a lean local builder isn't offered, so the model asked only
      for `core_brief_write` and did nothing else for three checks. A lean builder's prompt leaves
      the memory steps out; live, the same model then used the Build tools
- [x] A call written after a sentence ("Let's list the files.\n```json {…}```") isn't run: #554
      reads a reply that is only the JSON. Half the model's calls came this way and the turn ended
      with nothing done. A reply that ends in complete `json` fences, each a call to an offered
      tool, with only words before or between them, now runs them all (`fenced_calls`, ids
      `react_N` as other text calls); words after the last fence, other code, an unclosed fence or
      a tool that wasn't offered keep it text (#753)
- [x] A call followed by a summary isn't run, and the model is told to retry it without trailing
      text: in a second live run of
      #417's case both turns put "### Summary:" after their one `json` fence, nothing ran, and the
      build stopped after two turns with no files. The next message now says that it wasn't run
      — PR #1028, branch bugfix/r5.4-fenced-call-summary
- [x] Keep the retry condition when `Action:` or `Final Answer:` appears between fenced
      calls, without rejecting ordinary words between calls (PR #1031 review)
      — claimed: Codex, branch bugfix/r5.4-fenced-call-followup
- [x] A call to a tool that isn't offered comes back from a llama.cpp server as text in Qwen's
      markup (`<tool_call><function=web_search><parameter=query>…`), and the loop takes it as the
      final answer: a live build on a 512k-window Qwen model stopped at its second check in 3
      minutes. Read that markup as a call when it names an offered tool; when it names one that
      isn't, say so and name the tools it has, and ask again
      — claimed: youkaicountry, branch bugfix/r5.4-text-tool-calls
- [ ] A local builder spends its rounds on the project file and never writes the app: run 4 had a
      compiling `Cargo.toml` by check 3, its last, with `main.rs` still "Hello, world!". See whether
      more rounds get there, or whether a small model needs the window and the `--screenshot`
      option written for it (a template in the Rust steps)
- [x] The model ran `cargo new app` in `app/` (commands run there), so the project landed in
      `app/app/` and every check said "No project to build found in app/ yet". The check now says
      where a cargo or npm project one folder down is, and the Rust steps say `app/Cargo.toml`,
      not a folder inside `app/` (a brief can't say `cargo init`: not every builder's commands
      start in `app/`). Live, with calls after a sentence run too, the model then made
      `app/Cargo.toml` and the build compiled (#751)
- [x] `usage.json` after a local build says about 4,200 input tokens and 6.5 s of work, while
      Ollama logged about six requests of 4,000-5,000 tokens each: find what it leaves out.
      Cached tokens: Ollama 0.40 reports them apart (`cached_tokens`), and the built-in loop's
      `TurnDone` left them out while coding agents count them. It counts them now; a live run on
      Ollama 0.40.7 had recorded 5,323 input tokens of 75,014
      — claimed: SoldierPePe, branch bugfix/r5.4-usage-cached-tokens
- [ ] Told that its `cargo new app` made `app/app/`, the model kept editing `app/app/Cargo.toml`
      and running `cargo build` in `app/`, and checks 2 and 3 still found the project one folder
      down (live, GPU). See whether naming the move gets it out, or whether the run should move it
- [x] The model reads file paths from the project folder and commands from `app/` as one base:
      it ran `cargo init app` and `cd app && cargo build` after writing `app/Cargo.toml`.
      `build_command_run`'s result now says where it ran (`ran_in`), a command that makes a project
      in `app/app/` is told it goes in `app/` itself, and one that starts with `cd app` where
      there's none is told to leave it out. Live (Ollama, `qwen2.5-coder:14b`) the first command
      was still `cargo new app`, before any result could say so. On a GPU (Ollama 0.40.7) the note
      then reached the model; on the CPU a second run wrote the call with a summary after it, so
      nothing ran (the open line on a call followed by a summary)
      — claimed: SoldierPePe, branch bugfix/r5.4-build-command-folder
- [x] An assistant turn with no text sends an empty string, not `content: null`, which Ollama
      refused as `invalid message content type: <nil>` (#403, #536)
- [x] Understand a tool call a model writes as JSON in its text (#417; implementation `dfe9a4db2dcb3e27edbd67311cbb4202584e7af6`). The fallback protocol reads
      only `Action:` and `Action Input:` lines, so anything else ends the turn with nothing done
- [x] Preserve `length` and `content_filter` stop reasons on native tool-call streams; a parsed
      call must not override truncation or refusal and execute tools. Pair other retained
      unexecuted calls with results before the conversation continues — PR #544
- [x] Return a rejected tool argument's JSON Pointer when known and guidance to correct the call
      (#498), keeping schema types strict
      — commit `0c3ae5cca569`
- [ ] Find how each local server is told its context size and set it, or stop with a plain message
      when the prompt won't fit. We assume 24k and send no `num_ctx`, which an OpenAI-style endpoint
      may ignore anyway; LM Studio refused a 12.6k prompt on its 8k default (#73)
- [x] A builder that can't see pictures (the API loop) is told it can't, then sent to open
      screenshots with Read: the website capture lines and the `--screenshot` and reference-picture
      lines in `brief.rs`, the website check's "Open each pair", the reminder after each round, the
      `--screenshot` contract and `capture/README.md`'s window picture. Give it only what it can
      use, and say what the check measures instead. `BriefCx::sees_pictures` (today: not the API
      loop) picks the wording: a website's `layouts` and each round's `capture.json`, a program's
      `ui-tree.json` and the check's likeness, and asking the person what their pictures show
      — claimed: youkaicountry, branch bugfix/r5.4-blind-builder-prompts
- [x] The carry-on and follow-up prompts (`continue_prompt`, `follow_up`) ask an API-loop builder
      to check its work with rebuild_check, which the loop refuses while it builds; only its first
      message says so. Say "Craft Anything checks it after your turn" there instead
      — claimed: Afloat16, branch bugfix/r5.4-api-follow-up-check
- [x] When a server refuses a request for its context window and says the size (Ollama, llama.cpp,
      OpenAI-style), fit the conversation to that window and ask once more, else say how to raise it (#962)
- [x] Ollama fits a long prompt to its context by dropping the oldest messages, and a Qwen 3.8 model
      then refuses what's left with 500 "no user query found in messages" (#1245): say the context is
      too small and how to raise it, and don't retry what can't fit
      — claimed: justinjohn0306, branch bugfix/r5.4-ollama-dropped-request
- [ ] Make room on its own after #1245's refusal: the server doesn't say its window, so this waits
      for the window read from the server (#1155)
- [x] A per-backend context budget in `[[ai.backends]]` (`LoopConfig::context_tokens` exists), for
      services whose real window differs from 150k (#374) — implementation: `6bd94b84828ec89b805cb8a246f557d0a778c037`
- [x] A per-backend request rate in `[[ai.backends]]`, for services that limit requests per
      second or minute (#668): `requests_per_minute`, shared by every conversation on the backend.
      Not run against a live provider — claimed: justinjohn0306, branch `feature/r5.4-requests-per-minute`
- [ ] Offer `requests_per_minute` in Settings → AI, and say in the feed when a build is waiting
      for its turn (#668); a tokens-per-minute limit isn't covered either
- [x] Find LM Studio on another port (#73): the port in its settings file (when it is there) and
      the ports 1235-1239 are probed like the usual servers; OpenCode's own `baseURL`s were already
      read (#83)
- [ ] Check live that LM Studio on `--port 1235` is offered, and where its settings file is on
      Windows, macOS and Linux (`LM_STUDIO_SETTINGS` is from memory of its folders, not verified)
- [x] "Look for AI" (`detect::probe_local`) saved the first chat model a local server lists; LM
      Studio lists every model downloaded and FastFlowLM every model it can download, so it could
      load or download one the person isn't using (#1434). Prefer the loaded model: `/api/ps`
      (Ollama, FastFlowLM) or LM Studio's `/api/v0/models` (`state: "loaded"`)
      — claimed: jgt87, branch bugfix/r5.4-detect-loaded-model — PR #1437
- [ ] See why Bionic or LM Studio can't be picked (#140)
- [x] Do not select an embedding, ranking or transcription model as a chat backend when a local server has no chat models — claimed: Afloat16, branch bugfix/r5.4-nonchat-model-detection-20261103
- [x] Recognize specialised model file paths and numeric task-name versions during local chat setup
      — claimed: Afloat16, branch bugfix/r5.4-model-task-identifiers-20261103
- [x] Adding a server by its address with no model took the first chat model its `/models` lists;
      FastFlowLM lists every model it can download, so the saved model started a 5.5 GB download
      and unloaded the one in use (#1434). Prefer a chat model the server says is loaded
      (`/api/ps`, which Ollama and FastFlowLM answer) before the list
      — claimed: jgt87, branch bugfix/r5.4-add-server-loaded-model — PR #1435
- [ ] **decision** When nothing says which model is loaded and a server lists more than one chat
      model, ask the person to pick one (the list in the address form's model field) instead of
      taking the first (#1434): `/api/ps` helps only while a model is loaded, and LM Studio and
      llama.cpp routers don't answer it
- [x] The first run's AI step can add a model server on another computer by its address (llama.cpp,
      Ollama or LM Studio on a home server), the same form as Settings → AI
- [ ] A chat sent while a llama.cpp router is still loading the model ended after 31 s with "The
      answer stream ended early." and no next step (seen once live, 27B model); say the server may
      still be loading it and to try again in a minute
      — claimed: yashpatadiya1724-cmyk, branch bugfix/r5.4-stream-loading-remediation
- [x] An AI on this computer (Ollama, LM Studio) that can't be reached, or that took a request and
      didn't start answering, is told apart: "Couldn't reach Ollama (local)" for a server that was
      reached and "check your internet connection" for one on this computer misled, and an answer
      that never started didn't say a large model may still be loading or reading a long prompt
      (#1239 gave it the `timeout_secs` advice)
      — claimed: jgt87, branch bugfix/r5.4-local-server-messages — PR #1242
- [x] Adding a server by its address showed nothing while it connected and gave up after 10 s with
      "check the address" for a server that was reached but still starting or loading a model
      (#1429): show "Connecting to <name>…" with a spinner and the seconds waited, wait a minute,
      and give advice for a server that was reached
      — claimed: jgt87, branch bugfix/r5.4-add-server-connecting — PR #1430
- [x] Let a person stop adding a server while it's asked for its models (#1429): the request runs
      on a helper thread and the wait checks the job's cancel, so Stop beside "Connecting to…"
      ends it at once (`AiHub::add_server_until`); the request given up on ends by its own limits
      — claimed: FuryNocturn, branch bugfix/r5.4-stop-adding-a-server

### R5.5 API-key builds fail in words people can use
Owner: open · Status: todo
- [x] `rebuild --backend <an API backend>` still adds the first agent app it finds to the person's
      settings and prints "Using Claude Code (…)": `ensure_agent` (`rebuild_cmd.rs`) runs whenever no
      agent app is set up, though this run won't use one. Run it only when the build needs one
      — claimed: AlexJoaquimPereira, branch `bugfix/r5.5-no-agent-with-api-backend`
- [x] `--backend <a name that is turned off or unknown>` is refused with a specific message: a
      turned-off backend is named and must be turned on, and a name that matches nothing is named
      instead of saying to install an agent or add a key
      — claimed: AlexJoaquimPereira, branch `bugfix/r5.5-disabled-backend-name`; commit a380b3e
- [x] When a project's saved builder was turned off, Start builds with the default AI without
      saying so: the UI picker filters the stale name out (`ui-egui/src/builders.rs`) instead of
      reporting that the chosen AI was replaced. The Make picker says so in a toast, and every
      start (Start, Continue, a note, Try again) names the saved AI and the one building instead
      — claimed: AlexJoaquimPereira, branch `bugfix/r5.5-stale-builder-notice`
- [x] `rebuild --continue-in`, `rebuild_start` on a project and `rebuild_question_ask` name the
      saved builder that is off or gone and the AI building instead: the CLI prints it before the
      run, `rebuild_start` adds `builder_notice`, and an answered question carries it
      — claimed: AlexJoaquimPereira, branch `bugfix/r5.5-headless-builder-notice`
- [ ] The saved-builder replacement notice is English only for now; add it to the catalogs after a
      native speaker checks the wording
- [x] Bound accumulated data across all lines of one server-sent event, so short lines cannot
      grow a streamed API response without limit
      — claimed: Afloat16, branch bugfix/r5.5-bounded-sse-events-20261103
- [x] Show the provider's own reason: #369 bought $10 of credit and got only "OpenRouter answered
      400: Provider returned error". Find the reason in the response body and show it — PR #542
- [x] OpenRouter picks an upstream provider per request, and one without tool calling answered
      "400: Provider returned error" (#369): a request with tools now sets
      `provider.require_parameters`, and a 404 "no provider for these parameters" falls back to the
      text protocol like any server that rejects tools
- [x] OpenRouter reserves a model's whole output limit against the key's credit when no `max_tokens`
      is sent, so a key with $35 left of a daily limit got 402 "You requested up to 131072 tokens, but
      can only afford 35063" (#1213): ask again once with the answer capped at what it can afford
      — claimed: justinjohn0306, branch bugfix/r5.5-openrouter-afford-tokens
- [x] A saved fal.ai key reaches an API-key build's asset tools; every call said "No fal.ai key"
      (#923). Outside a project the tools now say that instead. Not checked with a real fal.ai key
      or API build
- [x] An API-key build works alone: it shares the app's session, whose `attach` points at whichever
      project started last, so another build beside it sent its files and asset spending to the
      wrong project (`build.parallel` is 2 by default). Others wait their turn in the queue
- [ ] Let API-key builds run side by side: give each an engine of its own, as a question has. This
      also closes the gap left in working alone: the app and the `rebuild_start` tool each keep a
      queue, so a build the in-app chat starts isn't kept apart from one the app started
- [ ] Check #369 live with Kimi K3 and a key limited to Kimi/Moonshot models (needs the person's
      OpenRouter key): that the build starts, or that the error now names the provider's reason
- [ ] A live API-key build (needs a key on the test machine)
- [x] A Google AI Studio (Gemini API) key is its own choice in Settings → AI and first run (#646):
      the `gemini` backend kind uses Gemini's OpenAI-compatible endpoint and `GEMINI_API_KEY`
- [ ] Check a Gemini build live with a real Google AI Studio key (tool calls through the
      OpenAI-compatible endpoint)
- [ ] Failure logs for the other ways an AI stops (API backends, MCP server start-up), like the
      ones agent CLIs write to `logs/` in the config folder
- [x] Settings → AI can replace or forget an API backend's saved key, so a key rotated in the
      environment isn't hidden behind an older saved one (#1358); `ai key <name>` shows, replaces
      or forgets it headlessly, and never prints it
      — claimed: AlexJoaquimPereira, branch bugfix/r5.5-replace-saved-key

### R5.6 Tools report only what they saw
Owner: open · Status: todo
Golden rule 3: the result is evidence. Each fix needs a Mac to test on.
- [x] macOS: a denied Accessibility permission is reported as success with an empty tree (#240)
      — claimed: mavericzz, branch bugfix/r5.6-accessibility-denied; PR #887
- [x] macOS: `runtime_ui_read` captures a requested screenshot (#248): one on-screen window
      belonging to the pid, restricted by `window_id` when supplied; PNG/base64 up to 2048 px
      per side. Combined JSON (including escaping and the newline) stays within 16 MiB by
      trimming the tree with explicit truncation and partial coverage, preserving the image.
      Missing Screen Recording permission, a missing window or a capture failure produces
      an explicit screenshot limitation and partial evidence. Denied Accessibility permission with
      the tree enabled (the default) returns a tool error before capture; `accessibility: false`
      skips the tree. Requires macOS 14+ and existing Screen Recording permission; opens no dialog
      — claimed: Codex, branch bugfix/r5.6-macos-screenshot
- [x] macOS: `runtime_ui_read` honours `accessibility: false` (#307): the Swift helper skips
      the tree and its permission check, returns no nodes without truncation, and the evidence
      records the flag and says the tree was skipped. Walkthroughs still require Accessibility
      — claimed: Codex, branch bugfix/r5.6-macos-ui-flags
- [x] macOS: call observation's lldb callback was a literal `\n` in its Python, so the Python was
      invalid and it reported zero calls (#251). Fixed under R2.4: the callback is one valid line
      covering every breakpoint, and a failed lldb run is an error, not "0 calls"; only the live
      lldb run is still owed (R5.7)
      — claimed: mavericzz, branch docs/r5.6-stale-251; PR #1070

### R5.7 Live checks owed
Owner: open · Status: todo
The code for each of these is done; what's left is to run it for real and record the result in
STATUS.md. A failure becomes a new line under its item. Grouped by what the run needs. Start with
the Clean room build and the README quick start: Clean room is the default and has no live build
yet, and the quick start is how every new person begins.

**A signed-in coding agent:**
- [ ] A live Clean room build of a website and of an app, checked end to end
- [ ] A live build in a picked folder each way (edit in place, start from), checked end to end
- [ ] Thresholds under pressure: a clone that's off by a few pixels (a page with fluid type or
      `clamp()` spacing) to see whether 4 / 2 / 8 px is too strict
- [ ] Screenshot check: in a live run the final check once got no picture from an Electron app
      within 90 s while the same check passed in the builder's turn and on a re-run; not
      reproduced. The app waited on animation frames, which a window behind others may not get;
      the Electron steps now say to capture after loading instead
- [ ] A live Windows build whose check stalls (Codex, a Godot project, as in the reports): the
      feed names the check it is in, the builder's own checks stop at their 30-minute limit,
      Pause acts between checks and stops a running build step, and a caught bug leaves "Needs you"
- [x] A live chat run with a big comparison result, to see Claude Code page it with
      `core_evidence_browse` (and whether 80 KB is the right size). On macOS with `chat --deep`,
      Claude Code 2.1.322 and two arm64 builds of `cargo` (Rust 1.97.1 and 1.99.0, ~30 MB each),
      whose `app_builds_line_up` page is 581 KB: Claude Code asked for `detail: summary` itself,
      then paged with `core_evidence_browse` (`matching`, `fields`; 7–14 KB a page) and quoted the
      rows; no result reached the budget (largest 25 KB) in 23 calls ($1.14). Without `detail`,
      the server answered the 581 KB page summary-first (4.8 KB, `continue_with`). 80 KB holds: a
      79.5 KB page (`limit: 15`) reached Claude Code whole as 37.7 KB of text, about 17k of its
      25k-token tool limit, and it quoted the page's last row correctly
      — claimed: chordpli, branch test/r5.7-evidence-browse-live
- [x] Neither the line-up's summary nor `core_evidence_browse`'s error names the lists it can
      page: Claude Code asked for `list: "changed"` (a `show` value), got "changed isn't a list of
      records… Leave them out to page it as it is", and ran the 20 s line-up again with
      `show: changed` before finding `list: "pairs"` with `matching`. Name the lists (`pairs`,
      `ambiguous`, `only_left`, `only_right`) in both. Every record's summary view (a
      `detail: summary` answer, the summary-first answer over a connection's budget, browse's
      `kind: summary`) now has `lists`, name → length, for the names browse reaches a list with;
      browse's errors for a name that isn't a list name them too
      — claimed: chordpli, branch bugfix/r5.7-browse-list-names
- [x] Live-check the named lists in a Claude Code chat: the same two `cargo` builds, and the
      agent pages `pairs` straight from the summary, without a wrong list or a second line-up.
      Claude Code 2.1.322, same question: the summary carried `lists` (`pairs` 1448, `ambiguous`
      612, `only_left` 29746, `only_right` 32342); it ran the line-up once and every browse
      call asked for `pairs`, none failed (22 calls, $1.06)
      — claimed: chordpli, branch bugfix/r5.7-browse-list-names
- [x] `app_builds_line_up` paired 1,448 of ~36,000 functions across two Rust toolchains: every
      v0 crate hash (`Cs<hash>_`) changed, so exact names rarely match, and some masked-bytes
      pairs are unrelated functions (`hex_encode` → `_git_oid_tostr`). Compare Rust names with
      the crate hash left out. A new `name_without_hash` stage compares demangled names with the
      instantiating crate kept, and calls are compared the same way: the same two `cargo` builds
      now pair 14,036 (13,356 by name), with masked-bytes pairs down from 888 to 233
      — claimed: chordpli, branch bugfix/r5.7-rust-hash-free-names
- [ ] After that, 1,347 of the 1,630 name groups left ambiguous hold one symbol at several
      addresses in one build (a local copy per codegen unit): pair copies whose bytes agree, or
      say they're copies. `hex_encode` → `_git_oid_tostr` is now a low-confidence structure pair
      (the new build inlined `hex_encode`)
- [ ] Live check on Windows: rebuild a project while its frozen pin runs, and the build isn't
      blocked

**An API key or a bill:**
- [ ] Run Nano Banana Pro once with a real key (a picture, then a 3D model from it) and check the
      picture's size and the invoice ($0.15)
- [ ] fal bills pictures "by rounding up to the nearest megapixel"; estimates use the exact size.
      Confirm on a real invoice whether a 1024×1024 picture bills as 1 or 2 megapixels, then round
      in the table (needs a key)
- [ ] A live extraction run with Kimi K3.1 Pro (needs the person's OpenRouter key)

**A second computer or another system:**
- [ ] A live run of a project copied between two real computers with Claude Code (#451, #452); done
      on one Mac with a second `CLAUDE_CONFIG_DIR` standing in for the other computer (STATUS.md)
- [ ] Ghidra setup tried for real on Linux and macOS (the Java pins were checked by tests only)
- [x] A Ghidra with no decompiler program for this computer (public downloads leave out macOS) is
      refused before the long analysis, with the command that builds it; checked on a real Mac
- [ ] `engines_install` builds Ghidra's native programs on macOS, so a Mac install works unaided
- [ ] Follow the README quick start on a clean machine (Windows, and Linux or macOS)
- [ ] Build the new DMG on a Mac (release dry run) and look at the window once
- [ ] #490 on macOS: a real Deep build through Claude Code with the published 0.3.0 CLI, with
      2.1.179 and a current version. Windows passed; the Mac CLI was only checked to contain the
      fixed schema
- [ ] macOS CLI zip blocked by Gatekeeper when downloaded in a browser (#94), though Apple has its
      notarization tickets: find why the online lookup fails for people (it passes on our Mac, which
      may be exempt), on a machine that shows it

**A real program, device or file:**
- [ ] Cutter sessions: try them against a real Cutter, including whether `=h&` keeps Cutter
      responsive (only a stand-in server so far)
- [x] Vineflower: run it once on a real JAR (no Java on the machine it was written on)
      — claimed: hasnain607, branch docs/r5.7-vineflower-live-run
- [ ] Verify the 23 unverified adb tools; record results in tool-status.md
- [ ] Android locales: try `only_locales` on a real APK (only the test fixture so far)
- [ ] `app_graphs_compare` on the two real Electron releases again, to count how many of the
      162 "removed" now pair (and pair the `index-<hash>` chunks by structure if many stay)
- [ ] A real IL2CPP `global-metadata.dat` (needs a Unity build or a shipped game); synthetic so far.
      Compare a real APK's components against another tool's reading
- [ ] Stand-in backend: a browser-made HAR and a real front end talking to it (only a hand-written
      HAR so far); GraphQL and tRPC on real bundles (only hand-written minified shapes)
- [ ] Check the Kimi Code, Factory Droid and WorkBuddy registrations in the real apps

---

## M1 Extensibility

### R1.5 External engine trait in L3
Owner: open · Status: in progress
- [x] Find Chrome, Brave, Chromium and Edge installed as Flatpaks on Linux (system and per-user
      exports), and keep a Flatpak or Snap browser's private profile in the home folder its
      sandbox shares, since its /tmp is its own (#747). Unit-tested; not run on a real Flatpak
- [x] Find browsers under their other Linux names, and say why one isn't found: a set path that
      isn't there, or a Flatpak's sandbox (#841). Unit-tested; not run on Linux or in a real Flatpak
- [ ] **decision** Let the Flatpak start browsers on the computer. It can't see system ones today;
      a per-user Flatpak browser in `~/.local/share/flatpak/exports/bin` is found, but its start
      script needs `flatpak`, which the sandbox doesn't have. `flatpak-spawn --host` would need
      `--talk-name=org.freedesktop.Flatpak`, which lets the app run anything outside its sandbox (#841)
- [x] Say in the feed while a decompile waits on Ghidra, and for how long, as the one-time analysis
      does (#1714): once a minute, "Ghidra is decompiling this function (N min so far; it gives up
      at 15 min)", or that it waits for another decompile to finish. The builder's tool server
      writes the line to `inbox/activity/`, which the run reads into the feed. Unit-tested; not run
      with Ghidra — claimed: justinjohn0306, branch `feature/r1.5-decompile-wait-in-feed`
- [x] Bound unguarded jump tables when Ghidra decompiles a function (#1715): with no bounds check it
      reads up to 256 entries, follows targets inside other functions and the decompile grows
      without end. Before a decompile the bridge finds a byte-indexed table with no guard, decodes
      the function's stretch (its `.pdata` entry on Windows) in a row, ends the table where another
      starts or at the first entry that isn't an instruction start there, and writes the override.
      Live on Ghidra 12.1.4: a Rust async function that timed out after 15 min decompiles in 16 s
      (121 tables) — claimed: justinjohn0306, branch `bugfix/r1.5-bound-unguarded-jump-tables`
- [ ] More pins once their hashes are checked against the publisher: Chromium (no published
      checksums found yet), Rizin for Linux (the tar.xz unpacks to ~800 MB, so it needs streaming
      extraction) and macOS (a .pkg)

### R1.6 Headless first
Owner: maintainers · Status: in progress
- [x] Give Linux-specific Clang guidance in Unreal's guided installer when the bundled compiler
      is missing (the requirements summary is already platform-specific).
      — claimed: AlexJoaquimPereira, branch `bugfix/r1.6-linux-unreal-clang`
- [x] Give macOS the right C++ guidance when Xcode is missing: the status line and
      requirements summary now name Xcode, matching the guided step.
- [x] Find Visual Studio's C++ tools on Windows when Visual Studio is installed outside Program
      Files (through the Visual Studio Installer's `ProgramData` instance state or developer
      environment variables, #669, [PR #726](https://github.com/storytold/craft-anything/pull/726))
      — claimed: riyaz, branch `bugfix/r1.6-windows-vs-cpp-tools`
- [x] Turn off web and capture tools by default in restricted MCP mode (#856, #855).
- [x] Find a Unity Editor that was added to Unity Hub with "Locate" (#1077): read the Hub's own
      list of editors (`editors-v2.json`, `editors.json`), on macOS and Linux too. Unit-tested with
      both layouts of the list
      — claimed: justinjohn0306, branch `bugfix/r1.6-unity-located-editors`
- [ ] Check on a computer with Unity Hub that a located editor is found (#1077): the list's
      layout was taken from descriptions of it, not from a real file

---

## M2 Product completeness

### R2.2 More coding agents
Owner: open · Status: in progress
- [x] Insert a missing YAML server table before an explicit document end marker.
      — claimed: Plastic-time, branch rrfz/bugfix-r2.2-yaml-document-end
- [x] Find Codex bundled with ChatGPT.app/Codex.app after a macOS Dock launch, and
      verify the selected CLI's sign-in through the headless connection check and Settings.
      — claimed: dbzx6r, branch bugfix/r2.2-macos-codex-signin-upstream
- [x] A saved agent program path that no longer exists (Codex desktop moves its CLI to a new
      build folder on every update) says so and names the path, instead of "isn't installed or
      isn't on your PATH" (#721); it stays a missing program, so a builder turn isn't retried
- [x] A relative program path started in another folder is looked up where it was started
      before it's reported as missing (Unix: a script whose interpreter is missing was said to
      have no file there)
- [x] `ai test` names the program it ran and the saved one it replaced, and `doctor` says a saved
      path has no program there instead of blaming PATH (#721)
- [x] `doctor` looks a registration's bare command (`craft-anything-cli`, as a hand-made entry
      names it) up on PATH, as the app starting it does, instead of calling it missing.
      — claimed: Plastic-time, branch bugfix/r2.2-doctor-cli-on-path
- [x] Settings says when a saved agent program is gone and which one runs in its place
      (`AiHub::replaced_programs`, read on a worker with the AI list), with "Use it" to save
      that one when it's a file of its own (#721, #1468)
      — claimed: FuryNocturn, branch feature/r2.2-saved-program-gone
- [x] A build says in its feed when it runs another program than the one saved in Settings → AI
      (#721) — claimed: Agapenordev, branch bugfix/r2.2-build-says-replaced-program
- [x] GitHub Copilot through its own CLI login and ACP, without credential import:
      discovery, MCP-only tool restrictions, model selection and fresh read-only questions;
      unit tests and CLI 1.0.105 with an isolated home and loopback model, all five access modes
      — claimed: qu-gg, branch `feature/r2.2-copilot-integration` (recorded during PR preparation)
- [x] Grok Build as a coding agent over ACP (`grok agent stdio`): discovery in `~/.grok/bin`,
      the person's own `grok login` (a missing `auth.json` is unknown, so an API key still
      starts), model and reasoning effort, and our MCP server on the session with `"type": "stdio"`
      only for Grok. Clean room, deep chat, and questions are refused, because Grok can read and
      edit without a permission request. Verified against Grok 1.0.77's ACP handshake and
      `session/set_config_option` (the CLI `--model` flag does not stick after `session/new`). A
      live deep build on 2026-11-06 (Grok 1.0.77) finished, called the Craft Anything server, and
      answered one permission request. Clean room and deep chat were not run live; those runs
      are refused.
      See [Grok Build](grok.md).
      — branch `feature/r2.2-grok-agent`
- [x] Review hardening: cap target handoff at 128 while preserving the active target; propagate
      context-discovery cancellation, normalize CLI-only efforts for Claude requests, and name
      the configured agent in shared picker warnings, with focused regressions and translations
- [x] Local agent chats share a saved investigation and open-target metadata with MCP; background
      refresh makes recorded findings visible in the workbench Evidence pane. `core_finding_edit`
      corrects a finding with checked evidence and an optional revision guard, preserving confidence
      when omitted. Builder refresh failures warn instead of ending the turn; shared ACP errors
      use the current agent and its sign-in guidance. Engine, process-level MCP and headless
      rendering tests cover sharing, mode boundaries and finding ids; regressions cover confidence
      preservation, explicit overrides and agent-specific errors
- [x] Serialize project-backed investigation load, revision check, mutation and all rendered
      writes under one bounded, cancellable interprocess lock. Refuse malformed state rather
      than replacing it; separate-process regressions retain concurrent appends and reject one
      of two updates based on the same revision
- [ ] Refresh externally changed investigation/evidence records incrementally, avoiding repeated
      full reloads under the session lock while retaining mid-turn findings visibility
- [x] Offer OpenCode locally and over SSH with its own visible chat models and supported thinking
      depths (Low, Medium, High, Xhigh, Max) in settings and headless clients. Choices are validated
      before saving; explicit choices are refused at execution until native MCP support is available.
      See [OpenCode](opencode.md).
- [x] Capture OpenCode settings and project together; refuse a busy session instead of treating
      it as no project, and preserve open model pickers and matching results during contention.
- [x] Report cancelled model validation as stopped, while preserving error events for other failures.
- [x] Finish rejected or cancelled model validation with a terminal event so headless chat exits;
      document functions touched by the OpenCode model-selection changes.
- [x] Integrate the five-level `Effort` type from #395 for agent variants, and share a strict
      disk-based selection save between the hub and headless tools, preserving unrelated edits.
- [x] Rebase onto main after #395's squash merge, preserve extra model arguments without a saved
      selection, and render the remote-host placeholder visibly in the setup instructions.
- [x] Hold the UI settings worker's session guard through its disk save so it cannot overwrite
      a newer model selection; reverify the existing terminal-event regression for validation failures.
- [x] Label the settings-save guard so filesystem I/O is identifiable as a running job by the lock tracker.
- [x] Finish a failed CLI worker-thread start with a terminal event after its error so headless chat stops polling.
- [x] Synchronize the UI exit-time settings save with the session's latest AI table and other writers;
      an isolated exit-save regression preserves the latest model/depth and pending app settings.
- [x] Never wait for a running tool's session lock at shutdown: defer non-AI app edits in a versioned
      sidecar when busy, then recover against the latest committed AI settings at startup.
- [x] Discard superseded deferred settings after a successful full exit save; document model-only
      and model-with-variant command forms, and share transport selection between preflight and runs.
- [x] Rebase after #567 and translate the ten model-selection strings and two agent tool titles
      in the Russian catalogue, restoring coverage and catalogue key parity.
- [x] Supply and verify the run's Craft Anything MCP server in native OpenCode before enabling
      explicit model/thinking execution: a config file in `OPENCODE_CONFIG` with `run --standalone`
      (verified live locally). — claimed: advokat11, branch bugfix/r2.2-opencode-native-mcp
- [x] Hand native OpenCode runs over SSH the Craft Anything MCP server (the config file is local).
      One `ssh` run checks `craft-anything-cli` is on the host's PATH and writes the config on the
      host (`~/.cache/craft-anything/`, mode 600, atomic rename); the builder fails fast with the
      fix if the helper is missing. Verified: POSIX-shell regression test (`the_remote_line_runs_on_a_posix_shell`)
      and an SSH round-trip to localhost (probe, byte-exact write, mode, cleanup). Not run with a live
      agent over SSH. — claimed: jelassiaymen94, branch feature/r2.2-opencode-remote-mcp
- [x] Make `the_remote_line_runs_on_a_posix_shell` accept the shell's missing-command status:
      assert failure rather than 127, isolate PATH from installed helpers, and verify that a
      missing helper writes nothing in a fresh home. Fixture utilities use host lookup rather than
      assuming `/bin`; the shell uses the bounded process runner.
      Reproduced the old failure on macOS; the AI suite now passes (446 passed, 2 ignored). PR #1018
      — claimed: Codex, branch test/r2.2-posix-remote-helper
- [x] Find OpenCode's CLI inside its Windows desktop app (`opencode-cli.exe`), for detection and for runs (#1060)
- [x] OpenCode Go gets one session id per installation, saved at startup for any saved Go backend, so
      a restart keeps it (#1225). An OpenCode error without a message shows its name, status and the
      start of its response body. Not run live; the run-5 cause in #1225 still needs its log.
      — claimed: fabox-os, branch bugfix/1225-opencode-go-session
- [ ] Settings → AI can only add an agent it detected: add one by choosing its program, for a CLI
      kept somewhere we don't look (#1060)
- [ ] Verify OpenCode model discovery and an explicit thinking selection on a live SSH host,
      and complete a live generation with the selected model and variant (local catalogue verified).
- [x] A stopped, failed or killed OpenCode turn deletes its session, so OpenCode's shared service
      can't resume it and keep running the builder's commands later (#1115). Probed live with
      OpenCode 2.2.5 on Windows: a killed `run --standalone` session stays `running` and the
      service resumes it; `session delete --standalone` removes it without resuming it
      — claimed: justinjohn0306, branch bugfix/r2.2-opencode-interrupted-sessions
- [x] When Craft Anything itself is killed mid-turn (uninstalled, a crash; closing the app during a
      build isn't known to wait for the delete either), no turn is left to delete the OpenCode
      session, and the service still resumes it (#1115): each turn records its session under a
      lock the system drops when the process ends; the next agent turn and
      `craft-anything-cli uninstall` delete the sessions of records nobody holds. Over SSH, the
      delete runs on the host too
      — claimed: justinjohn0306, branch bugfix/r2.2-opencode-leftover-sessions
- [x] Sweep leftover agent sessions when the desktop app starts, not only at the next agent turn
      or `craft-anything-cli uninstall` (#1115) — a background thread at start-up, so a delete
      over SSH can't hold up the window
      — claimed: AlexJoaquimPereira, branch bugfix/r2.2-startup-session-sweep
- [ ] A turn stopped before OpenCode's first event leaves no session id to delete: OpenCode names its
      session only in that event, so the session it may already have made stays (#1115, review on
      #1176). Needs a session id known before the run (one we create and pass in), if OpenCode allows it
- [x] OpenCode's model discovery (`api model.list`, run before every turn to check the chosen
      model) has no `--standalone`, so it starts the shared service, and the service resumes any
      unfinished session on the computer, in any project (#1115). Seen live with 2.2.5 on Linux: a
      build's model check woke it, and a session left from days before went on editing another
      project. Discover with a private server unless the person chose `--server`
      — claimed: youkaicountry, branch bugfix/r2.2-opencode-discovery-standalone
- [x] `docs/opencode.md` says "OpenCode V2" but not where to get it: OpenCode's install script
      and npm's `opencode-ai` give the 1.x line, whose `run` has no `--standalone` (npm's
      `@opencode/cli` is 2.x), so the check before a build
      can't confirm it. Say where 2.x comes from and how to point the backend at it, and record
      the Linux live runs in STATUS.md
      — claimed: youkaicountry, branch docs/r2.2-opencode-live-findings — PR #1241
- [ ] OpenCode's shared service, started while a `--standalone` run is going (opening the desktop
      app does), takes over that run's session: both servers ran its commands, and the turn ended
      with an error that had no details (seen live on Linux, a 2.2.6 service beside a 2.2.5 run).
      Keep a run's session out of the service's reach, if OpenCode allows it; until then
      `docs/opencode.md` says not to open OpenCode during a build
- [ ] OpenCode logs "Received a progress notification for an unknown token" for our tool server
      after calls through 2.x's `execute` (seen live on Linux, 2.2.5): check that our MCP server
      sends progress only for a request that carried a `progressToken`, and only before its result
- [x] Stop with a plain error when the `opencode` found is 1.x (its `run --help` has no
      `--standalone`), instead of running it anyway and showing its usage (#1111)
      — claimed: Agapenordev, branch bugfix/r2.2-opencode-v1-refusal
- [ ] When the `opencode` on PATH is 1.x and an OpenCode 2 is installed elsewhere, find and use
      the 2.x one, and confirm a live OpenCode 2 build end to end (#1111 stays open until then)

"Found on this computer" has to mean "can do the job". Today only Claude Code gets everything.
**Every builder gets the same tools:**
- [x] A table of what each adapter gets (our tool server, approvals, resume, notes mid-turn, a
      message of any length), as typed data, with a test that fails when one is left out.
      `AgentAdapter` already answers some, such as `resumes` and `prompt_on_stdin`. The lines below
      fill it in. `cli_agents::grants` states the columns an adapter can't answer (our tool server,
      approvals, notes mid-turn, and whether the run is per message or ACP), reports resume and a
      long message from the adapter itself, and four tests hold every row to what the adapter
      actually hands a builder: a run's arguments and environment (or the file an argument names,
      for the harness's overlay) must carry the server it claims, the notes hook must be in the
      arguments exactly when a note mid-turn is claimed, and a claimed approval channel must be
      there. A cell nobody has checked says `NotYet` with the gap in words rather than guessing;
      today that is DeepSeek Harness's approvals and every agent with no adapter of its own
      — claimed: muraa-p, branch feature/r2.2-adapter-grants
- [ ] OpenCode's plain adapter never passes our tool server, so its builder has none of our tools
      and hunts for a setting that doesn't exist (#481). Hand it the server, or build over ACP by
      default once that passes
- [x] Aider has no adapter, so the plain one hands it the prompt as a file name, and it can open a
      browser sign-in nobody agreed to (#72). Give it an adapter, or stop offering it — claimed: Afloat16, branch bugfix/r2.2-aider-recommendations-20261103
- [x] Add a non-interactive Aider adapter before recommending it again; detection currently leaves it out.
      Aider gets the message with `--message`, and flags that stop it asking, updating, editing
      `.gitignore` or opening a page; a chat outside a build also gets `--dry-run`. Before a run it
      refuses when no model or key is set up, or an OpenRouter model has no OpenRouter key: both
      would offer a sign-in and wait for a browser nobody is at (#72).
      — claimed: hasnain607, branch feature/r2.2-aider-adapter
- [x] Aider: a Clean room run is refused in `validate_access`, but only the ACP path called it, so a
      native run (`run_cli`) still started Aider and its unchecked reads. Call it before the setup
      check, with a test through the launch path
      — claimed: hasnain607, branch bugfix/r2.2-aider-clean-room-run-cli
- [ ] Codex (#499): in a chat the tool server has no active target. The reporter attached a patch
      for chat continuity; look at that first
- [x] Codex (#538): list every permitted tool from startup, since Codex doesn't refresh its callable
      list after `enable_toolset` changes it — PR #805
- [x] Codex (#540): `rebuild_start` was refused by `codex exec`'s approval policy and showed as
      `null`; setup and the builder now approve our tools for Codex (we still ask before running
      code) and a refusal explains the fix (`docs/codex.md`)
- [ ] Codex (#540): run `codex exec` on a real install and check `rebuild_start` gets through
- [ ] Codex: resume support (`codex exec resume <id>`), verified on a real install
- [ ] Aider: move the message to `--message-file` once the launcher can write one (see the argv item above).
- [x] Aider: read project-folder `.aider.conf.yml` and `.env` in `check_setup`, which needs the working directory passed into `check_native_options`.
      It reads the run's folder, the top of its git repository, then the home folder, as Aider
      does — PR #1210; claimed: rmsff, branch feature/r2.2-aider-project-settings
- [ ] Aider over SSH: its setup check still reads this computer's environment and home folder.
      Read the remote host's `~/.env`, `~/.aider.conf.yml` and `~/.aider/oauth-keys.env` there
- [ ] Every adapter but Claude Code's still takes the message in argv (`prompt_on_stdin`), so a long
      one can pass Windows' command-line limit as Claude Code's did (#402); hand each its message on
      stdin or in a file where its CLI reads one
- [ ] A note sent during a build reaches the builder at its next step on other coding agents with
      tool hooks, and on the API-backend loop between tool calls (Claude Code only today)
- [ ] Session copies for other agents that keep sessions (ACP agents' `session/load`) (#451)
- [x] Hermes Agent's native Windows install (`%LOCALAPPDATA%\hermes\bin`) is searched, so an app
      started before the install, or by a launcher, finds it (#116)
- [x] `mcp_servers: null` (or `~`) in a Hermes `config.yaml` takes our entry like an empty table,
      instead of being refused as flow style.
      — claimed: Plastic-time, branch bugfix/r2.2-yaml-null-table
- [ ] Live-check #116 on Windows: install Hermes natively, start the app from a launcher, and see
      Settings → AI offer it; #116's "error after every message" has no text yet (asked)
- [x] Refuse multi-document YAML configs before planning edits or removing registrations.
      — claimed: Plastic-time, branch rrfz/bugfix-r2.2-yaml-single-document
- [x] A Hermes `config.yaml` that starts with a byte-order mark gets our entry in its own
      `mcp_servers` table, not in a second one that hides the person's servers from Hermes.
      — claimed: Plastic-time, branch bugfix/r2.2-yaml-config-bom
- [ ] See whether Hermes still fails on 0.3.0 (#460: the fix its `fixed` label names was already in
      0.3.0 when it was filed, and it gives no version)

**Approvals:**
- [ ] **decision** One approval behaviour across agents, and whether we want an auto-approve mode
      apart from Clean room and Deep (Auto mode only governs what's asked through us). Today it
      differs by agent: Claude Code asks our approval tool; OpenCode's builder runs with `--auto`;
      Hermes's `-z` turns its approvals off on every run, chats included. Clean room's approval
      rule doesn't reach an agent that never asks us
- [x] Clean room outside builds (#564): a chat or question with Claude Code refuses the agent's own
      file, shell and web tools outright (`--disallowedTools`), not through its defaults; an ACP chat
      refuses them through an approver instead of a consent card (`docs/claude-code.md`)
- [ ] Say on each adapter, as typed data, what it can be held to (asks us, its own sandbox,
      nothing), in a build, a chat and a question, and have runs act on that, once the decision above
      is made. Today Codex, Gemini CLI, OpenCode, Antigravity, Hermes and Maki are not held to
      Clean room in a chat (their own sandbox, if any, is not established)
- [x] A Deep build through Claude Code can start subagents again: `--tools` left out `Agent` when
      #637 trimmed the list (#1105) — claimed: justinjohn0306, branch bugfix/r2.2-builder-subagents
- [ ] Check live that a Clean room chat with Claude Code can't read a file outside its folder, and
      that an ACP chat in Clean room still reaches our tools (its permission requests for them are
      allowed, only its own read/write/command/fetch are refused)
- [ ] Check live what Hermes over ACP asks about: whether its file reads reach our approval policy
- [ ] Maki's plain `maki -p` builder doesn't ask us before commands, and in Clean room its reads
      aren't checked (as with every agent that never asks us): say so in the approval decision
      above, or run Maki over ACP by default once that passes. Outside a build, tools the person
      allowed in Maki's own permissions (an MCP server's, or all of them with `always_yolo`) still
      run; the adapter can't see that config
- [ ] Maki over ACP writes inside its working folder without a permission request (its built-in
      rule), and that folder is a build's project folder, so writes to our own records (inbox,
      manifest) skip the approval policy; `maki acp` takes no tool flags to stop it (0.7.0). Start
      it in `app/` (paths in the prompt relative to that), or ask Maki for a way to make those
      writes ask; part of the approval decision above

**ACP:**
- [ ] Gemini CLI, opencode, goose via ACP or native adapters; each verified live, then turn ACP on
      by default for the ones that pass
- [x] Over ACP, suppress `session/update` notifications until `session/load` answers, keeping
      replayed messages and tool activity out of the current turn
      — commit `38ffc39be084`
- [ ] Over ACP, turns report 0 tokens: the protocol sends only `usage_update` (context used and
      size). Read real counts where an agent keeps them (Maki: `token_usage` in its session file
      under `~/.local/state/maki/sessions/`) or say the count is unknown instead of 0
- [ ] Devin over ACP: `devin acp` asks the host to `authenticate` (`meta.api_key`) before
      `session/new`, so the backend's stored key is sent — claimed: NibrasAz7, branch
      feature/r2.2-devin-agent

**Finding them:**
- [ ] #105, #135, #259, #385 and #423 predate 0.3.0, which looks in more places, and came through
      the ticket relay, so nobody can ask them for `doctor` output. Close them pointing at 0.3.0,
      and add search paths only for a new report that has that output
- [x] #423: an agent that is set up but switched off is never offered again, so "No coding agent was
      found" stays on screen with Claude installed: the setup card and "Look again" leave out every saved
      AI by name, on or off. Offer a switched-off agent as found and turn it on, keeping its options.
      Cause is not confirmed on the reporter's machine (needs their config.toml); the fix covers the
      switched-off case that the code leaves out (`is_on`, `leave_out_set_up`, `builder_backend`). — claimed: fabox-os,
      branch bugfix/423-disabled-agent-not-offered
- [x] Say plainly when what's found is the Claude desktop app rather than the `claude` program
      (#259, a guess at its cause)
- [ ] macOS: check that `Claude.app` is where the desktop app is found (#259's hint is untried there)
- [x] #259's hint, the gaps left in review of #781: it's hidden when only `~/.ssh/config` suggestions are
      found, a Store install without its `claude-desktop` alias isn't seen, and a desktop app's
      `claude.exe` on PATH is taken for Claude Code (#824) — claimed: FuryNocturn, branch
      bugfix/r2.2-claude-desktop-hint-gaps
- [x] macOS: `/Applications/Antigravity.app` ended up as the agent's command (#467). Found: the "Where is
      the program?" picker (`pick_file`) returns the bundle, and `set_command` saved any path. Refuse a
      folder or app bundle when it's saved (`set_command`), and don't run one that was saved before
      (`program_of` uses the usual name). Tests: `an_app_bundle_is_refused_as_an_agents_program`, `program_of`
      cases. — claimed: fabox-os, branch bugfix/467-agent-program-app-bundle
- [x] Windows: look in `%LOCALAPPDATA%\agy\bin` too (#62) — claimed: Afloat16, branch bugfix/r2.2-antigravity-search-20261103
- [x] Windows: `usual_windows_dirs_come_after_path` counts Antigravity's second folder (#748
      added it; the test runs only on Windows, which CI doesn't)
- [x] Windows: setup and doctor take the user profile (USERPROFILE) for home, as the apps do,
      not HOME, which MSYS2 and Cygwin shells point at their own folder.
      — claimed: Plastic-time, branch bugfix/r2.2-windows-home-is-the-user-profile
- [ ] CI runs the tests on Ubuntu only, so `cfg(windows)` tests (search paths, consoles, Job
      Objects) are checked only on contributors' machines: run them on a Windows runner too
- [ ] "Claude Code on <ssh host>" is offered for any SSH host, whether or not it has Claude Code
      (#62)
- [ ] Gemini CLI's sign-in reportedly refuses individuals now (#400): check, and stop recommending
      it in the builder setup dialog if so
- [x] DeepSeek Harness: an adapter runs `dsh --profile headless "<task>"` and hands it our tool
      server as a generated `--patch` overlay for that run (its own project, inbox and mode), since
      the harness reads tool servers only from profile layers; a question gets none and the person's
      own profile is left untouched. Verified with adapter and overlay tests and a live follow-up
      website build whose journal shows `rebuild_check`, `core_unknown_add` and `core_handoff_write`
      going through our server — claimed: 0231071, branch `feature/dsh-agent-adapter`
- [x] DeepSeek Harness: send tasks on stdin so multiline and long messages can start through the
      Windows desktop launcher (see the argv note above)
      — claimed: gtxy27, branch `bugfix/r2.2-dsh-stdin`
- [ ] DeepSeek Harness: no resume (its headless profile keeps no session), and a build on the harness's
      desktop profile is untried
- [x] Apply repeated client selections and aliases once, preserving their first selection order.
      — claimed: Plastic-time, branch rrfz/bugfix-r2.2-deduplicate-clients
- [ ] `setup` writes the plain `mcp` command, so it drops an `--all-tools` an entry already has, and
      `doctor` then calls that entry "a different copy or shape of the command" and suggests the very
      command that breaks it. The installed skill now warns against running `setup` on such an entry;
      `doctor` and `setup` should keep the person's extra arguments instead
      — claimed: 18923837179wjt-dot, branch fix/setup-skill-warns-before-setup-drops-all-tools
- [x] Windows: a `.cmd` that only runs one `.exe` by its full path (`"C:\…\codex.exe" %*`, written
      by hand or by another tool) was started through `cmd.exe`, which refuses an argument with a
      line break ("batch file arguments are invalid"), expands `%` and caps the line at 8191
      characters; that `.exe` is now started directly, as an npm shim's is (#1237)
      — claimed: jgt87, branch bugfix/r2.2-cmd-wrapper-exe — PR #1238
- [x] Codex's and Grok Build's `config.toml` keep their Windows line endings when setup adds or
      removes our entry.
      — claimed: Plastic-time, branch bugfix/r2.2-toml-config-crlf

### R2.4 Runtime study for apps and games (with consent)
Owner: maintainers · Status: in progress
- [ ] UI tree on Linux (AT-SPI)
- [x] `native_window_capture` on macOS: no Screen Recording is refused before the program starts
      (it was "didn't show a window", which said to wait longer), and the name fallback matches
      whole words and never the system, so `win` no longer pictures the menu bar
- [x] `native_window_capture` keeps looking for up to 10 s past `wait_secs` when no window is there
      yet, instead of deciding from one look; tried with a program that opens its window late
- [ ] Close a pictured program politely before killing it: a helper it started through launchd
      outlives the process group, and can stay running with the program's data files open
- [x] `apple_calls_observe`: the breakpoint callback is one valid line (it had a literal backslash
      and `n` between statements, so lldb stopped before launching, #251), covers every breakpoint
      instead of only the first, a failed lldb run is an error and not "0 calls", and names or
      arguments with line breaks are refused (they'd add commands to lldb's command file). The
      callback body was compiled with Python; lldb itself was not run
- [ ] Run `apple_calls_observe` on a Mac against a small program with `lldb` (the #251 probe):
      the hit for `alpha` is recorded, and a program lldb can't debug says so
- [ ] Window capture tried on macOS and Linux (only Windows so far); games behind a launcher
- [ ] Short scripted input sessions (enigo) to record screens of the original

### R2.5 Behaviour checks for non-web builds
Owner: open · Status: in progress
- [x] Keep mechanics behavior unverified when only the sheet has been read: no literal-use claims,
      persisted unknowns and an explicit usable-build summary; no builder retries for a missing oracle
- [x] Give console run comparisons a numeric-output oracle when needed: word likeness drops
      single-character tokens, so bare `4.5` and `9.0` both compare as empty output. An output
      that is only numbers is now compared as numbers, in order, when the original's repeated run
      can't say which ones vary: `4.5` against `9.0` fails the claim and says both, the same number
      still matches, and printing nothing where the original prints a number fails too. With words
      beside the numbers, likeness still decides, so a differing timing isn't a failed build
      — claimed: muraa-p, branch bugfix/r2.5-numbers-only-output
- [x] Compare console output's numeric spelling and order at positions stable across two
      original runs, and explain when varying numbers or unavailable repeats limit comparison
      — claimed: Afloat16, branch bugfix/r2.5-console-numeric-results-20261111
- [ ] Behaviour checks that use the sheet: run the build with scripted input and measure values
      (speed, jump height, fire rate) against it (needs R2.4's scripted input)
- [ ] Hold the self-test to what the check can see too: picture the game after its scripted
      input and require it to differ from the `--screenshot` picture (a report alone is the
      build's own word)

### R2.7 The request decides how to build
Owner: open · Status: in progress
- [x] Recognize Phaser and Bevy requests in the stack catalogue so Make can start those games without a Unity, Unreal or Godot editor.
      — claimed: Afloat16, branch feature/r2.7-phaser-bevy-game-stacks
- [ ] Make: a game whose request names a technology the stack table doesn't know (Phaser, Bevy…)
      still needs the default engine installed to start, though the builder is told to use what
      was named; ask the person, or let such a game start without an engine
- [x] The website check's missing-site note names every place `site_dir` accepts (dist/, build/,
      out/, or a plain index.html with no package.json) and the app's real place, "app/" or "the
      project folder" (#494)
- [x] A project edited in place (Work on my project, or `rebuild --existing edit`) keeps what its
      files are built with, unless the request names another stack: an engine's project file, then
      package.json and Cargo.toml dependencies, then `found_by` in `stacks.toml`, and a project
      whose dependencies name no stack there is left to the builder. A request that read as a game
      used to ask for Unity even in a Godot project, and an app got the kind's default
      — claimed: xerenon, branch bugfix/r2.7-stack-from-files; PR #755
- [x] A person's own project (edited in place, nothing to recreate) is judged by what it does for
      itself: its own build, then its own tests, through `[[toolchain]]` in `engine/src/toolchains.toml` (Rust,
      Node.js, Python, Go, .NET, Maven, Gradle, Make). Nothing is added to it for our sake: no
      `--screenshot`, `--self-test` or mechanics sheet. A site stack with nothing to recreate is a
      website, whatever the request says
      — claimed: xerenon, branch feature/r2.7-own-project-checks; PR #760
- [ ] Run the Go, .NET, Maven and Gradle rows of `[[toolchain]]` on real projects, and every row
      on Windows (`windows_program`): Rust, Node.js, Python and Make were run for real, on a Mac
      only. Also a person's own Unity project: it's opened in batch mode, which should exit with
      an error when its scripts don't compile (not run: no Unity on that machine)
- [ ] A person's own tests that already failed before their request fail the check too, so the
      builder is sent to fix them. If that proves a nuisance, run them once before the first turn
      and hold it only to what it broke
- [x] New projects build through the shared `[[toolchain]]` catalogue. Python root entrypoints,
      Go's root `main.go`, and one root `.csproj` also launch for screenshots, self-tests and Play;
      existing packaged programs, Rust, Electron and game-engine paths keep priority — PR #950
      — claimed: Codex, branch bugfix/r2.7-new-project-toolchains
- [x] Runtime window capture follows runner descendants (including `go run`) and reads controls
      from the same process that supplied the picture, without falling back to unrelated names — PR #950
- [x] Shared toolchain commands make project-local executables absolute before changing the
      working directory, preserving virtualenv paths for relative project placements — PR #950
- [x] Launch a Go project with one `cmd/<name>/main.go` as a package when the root has no
      `main.go` — claimed: Afloat16, branch feature/r2.7-go-command-layouts
- [x] Launch the single installed console script of a Python project from its virtual environment
      — claimed: Afloat16, branch feature/r2.7-python-console-entrypoint
- [x] Extend launch recipes to Java build outputs and Python packages; build checks already use
      their toolchain commands. A single `<name>/__main__.py` runs as `python3 -m <name>`, the
      single runnable jar in `target/` or `build/libs/` with `java -jar` (Maven now builds with
      `package`); build wrappers stay out of launch
      — PR #1209; claimed: rmsff, branch feature/r2.7-python-java-launch
- [x] Audit relative project placements in build, option-run and Play paths too: runtime window
      capture already normalized its request paths; engine build arguments, option runs, console
      run checks, Play and pins now resolve the program, working directory and output file to
      absolute paths before starting a child in app/
      — claimed: AlexJoaquimPereira, branch bugfix/r2.7-relative-project-paths
- [x] Decline a Godot launch when its project path cannot be represented as UTF-8 instead of silently replacing bytes in the selected path.
      — claimed: Afloat16, branch bugfix/r2.7-native-runner-project-args
- [x] Resolve Godot's project argument and the local Electron binary and project argument before
      starting either runner from a relative project folder — claimed: Afloat16, branch bugfix/r2.7-project-launch-paths
- [x] "player" alone doesn't make a request a game: a video or music player is an app (#953)
      — claimed: justinjohn0306, branch bugfix/953-player-not-a-game
- [x] System-tool and media-player phrases keep the app target: "a low level disk tool",
      "an MPEG 2 player" and "a single player app for movies". Independent game cues still apply
      — claimed: Afloat16, branch bugfix/r2.7-media-and-system-apps

### R2.8 Clean room means clean room
Owner: maintainers · Status: in progress
- [x] A remote (SSH) builder needs a craft-anything-cli that knows `--deep` and `--clean-room`.
      Every release from 0.1.0 declares both as global flags and applies them in `mcp`, and has
      `mcp --all-tools`, the only flags a remote tool server gets; a test fails when `mcp_for`
      gives it another. Claude Code and Codex builds over SSH now check the host has
      `craft-anything-cli` before starting, as OpenCode's did — PR #1211; claimed: rmsff, branch
      test/r2.8-remote-cli-flags
- [ ] A remote builder's `craft-anything-cli` should know `CRAFT_ANYTHING_AGENT` (#1022), which no
      release up to 0.5.0 reads: there, a CLI the builder starts calls tools as the person. Today
      that loosens nothing of the build's (the remote tool server has no project or inbox). Check
      it before a build once a release has it: version numbers can't tell (`main` says 0.4.0), so
      ask the CLI itself

### R2.9 Everyday use: typing, projects, settings
Owner: maintainers · Status: in progress
Gaps found while using the app for real projects.
- [ ] Make Stop interrupt a running agent promptly, including a shared-investigation refresh
      or an agent that ignores its protocol cancellation. Add cancellation regressions.
      — claimed: Codex, branch bugfix/r2.9-agent-stop
- [x] Windows source builds without craft-fonts show Chinese text as boxes. Load a validated,
      bounded system-font fallback on a worker thread for every UI font family.
      — claimed: Codex, branch bugfix/r2.9-windows-cjk-fallback
- [x] Each project chooses, per kind of action, what the builder's approvals get (allow, ask or
      refuse): reading the web, commands, files outside the project, pushing and publishing,
      deleting outside the project, system changes, other tools. Unset kinds follow Auto mode as
      today. Found when Auto mode refused every `git push` without a card, so the person's OK in
      the prompt box never reached the check, and refused `gh repo view` as publishing too.
      Tried live by a maintainer on Windows 10 (a release build of `main` at `b700e1d`,
      2026-11-06): a builder's push was approved and went through
      — claimed: tanish2k09, branch feature/r2.9-project-approvals
- [ ] Agents that don't ask our approval tool (OpenCode `--auto`, Hermes `-z`, Maki's `-p`) aren't
      held to a project's approval choices either; part of R2.2's approval decision
- [ ] While "commands" are allowed (Auto mode's default), a builder can run any program. Commands
      that name `craft.json`, the inbox or the approvals tool count as system changes, and our CLI
      started inside an agent calls tools as an agent, but a script that builds the path, or
      unsets `CRAFT_ANYTHING_AGENT`, isn't caught. Keep the person's choices where a builder's
      shell can't write them, or say plainly that "commands: allow" trusts the builder
- [ ] A typed note ("yes, push it") still reaches the builder only as text: say in the card's
      place, when the builder is refused, how the person can allow that kind
- [x] The project page and a slash command change those choices (like `/btw`): the Approvals
      menu under the prompt box and `/approvals <kind> <allow|ask|refuse|default>`
      — claimed: tanish2k09, branch feature/r2.9-approvals-ui
- [ ] The new approvals strings in every catalog were written with an AI; not reviewed by native
      speakers yet — pt-BR claimed: marlonferreira1800-hue, branch docs/r2.9-approvals-pt-br-review
- [x] A prompt that changes nothing in app/ still builds and starts the app again, often twice (the
      builder's own rebuild_check, then the check after its turn). Keep a check whose round all
      worked while the app's files and what it's compared with are unchanged
      — claimed: tanish2k09, branch feature/r2.9-reuse-unchanged-check
- [x] A check starts the app in front, with focus, on the main monitor, in the way of whatever the
      person is doing. The `--screenshot` contract now asks for the window without focus, and on
      Windows behind other windows (egui's `with_active(false)` and `AlwaysOnBottom`, which winit
      shows with `SW_SHOWNOACTIVATE` at `HWND_BOTTOM`; on macOS and X11 a covered window is
      reported occluded and eframe stops painting it, so it isn't lowered there); it must keep
      drawing unfocused (Unity's `runInBackground`). Play still opens it in front. It's the builder's
      to do, so a build made before reads it only in a new brief, and nothing checks it was done.
      Not run live — claimed: tanish2k09, branch feature/r2.9-check-window-behind
- [ ] A runtime study's check also starts the build normally (`native_window_capture`) to picture
      it beside the original, and that start still comes up in front with focus. Starting a program
      without focus needs `STARTUPINFO`'s show mode, which std has only unstable
      (`CommandExt::show_window`, rust-lang/rust#127544), or a safe window crate to send it back
- [x] Keep New project's content column fixed while the vertical scrollbar animates in or out,
      hiding it when there is nothing to scroll, so switching tabs never shifts the page horizontally
      — claimed: Codex, branch bugfix/r2.9-home-scrollbar-gutter
- [x] Agent paragraphs with glossary terms and long file links wrap together without pushing
      their card or later messages under the sidebar (#523); Project and Ask bounds checked at two sizes
      — claimed: carl-dourado, branch hotfix/r2.9-523-message-width
- [x] Create atomic-write temporary files exclusively with short names distinct from the destination and private Unix permissions from creation.
      — claimed: Afloat16, branch bugfix/r2.9-atomic-temp-creation-20261103b
- [x] A prompt brought back with Up keeps its pictures (their paths are absolute, so they don't
      come back after the project is moved)
- [ ] Change a project's builder from its page (a tool, so the CLI and MCP can too) — in PR #245
- [x] `chat --backend <a name that is turned off>` and `compare` refuse it before model or
      agent setup: `AiHub::start` checks `enabled` and says how to turn it on or choose another
      — PR #1008; claimed: Codex, branch bugfix/r2.9-disabled-chat-backend
      (the API regression uses invalid tunnel settings to detect setup before the guard)
- [x] Ctrl+V (Shift+Insert, Cmd+V) in a prompt box that takes pictures pastes a copied image (saved
      as a PNG) or copied picture files; attachments show as square picture tiles, the whole
      picture on hover, decoded on a worker
- [x] A picture over 20 MB dropped on a prompt box or picked in its dialog is refused when it's
      added, as a paste does (`workspace::attachable_picture`), before anything is copied
- [ ] When the builder decides a question, show the question and its choice as an answered card
      in the feed (the run sees only the tool call today; the builder's words say what it chose)
- [x] A question's own cost: a forked Claude Code session reports the whole session's running
      total, so a forked answer shows none; count questions in the project's usage — a question's
      cost is what its total added since the builder's last turn, unknown without a kept total;
      its cost is added to the project's usage, and concurrent usage writes in the app are serialized
      — claimed: AlexJoaquimPereira, branch bugfix/r2.9-question-cost
- [ ] "Make this change" on an answer that suggests one (sends it as a normal message)
- [x] "Just ask" is told it has fewer tools than the builder and that the build may be running
      beside it, so it no longer reports the build's tools as disconnected or its work as cut off
- [x] "Change it" knows what was just talked about in "Just ask": the next message tells the builder
      the latest few questions and answers, once, and a Clean room build only those asked in Clean
      room (#857). Not checked in a live build
- [x] A note sent while a build is running tells it what was asked in "Just ask" (#857): the
      note's hand-over file carries the answers, and the after-turn path puts them before the
      notes; only answers the run's mode may show, marked told when read or sent
      — claimed: AlexJoaquimPereira, branch bugfix/r2.9-note-just-ask; commit 78d3963
- [x] A note still waiting when the app closes is sent by the next run instead of deleted when it
      starts (#1620): its hand-over file keeps the person's words (`notes::Left`), and a run started
      without a message takes them as its own waiting notes, so they stay on disk until its builder
      takes them; a folder it can't read is moved aside (`notes-unread-…`), not deleted. A file
      from before this has only the hand-over text, whose Just-ask answers may be from the other
      mode, so it's still dropped. Unit-tested; not run live
      — claimed: justinjohn0306, branches `bugfix/r2.9-notes-survive-restart`, `bugfix/r2.9-left-notes-kept`
- [x] Record which mode a builder's session ran in: `agent_session` is kept across modes, so a Clean
      room build or question can resume a session from a Deep build and carry what it read, and such
      a question's answer is recorded as asked in Clean room. Don't resume one for Clean room
      — `agent_session_deep` in `craft.json` records it (one saved before the field counts as Deep);
      a mismatch starts from the brief, and a question is answered from the files; tests cover both
      directions and the legacy case. Claimed: AlexJoaquimPereira, branch bugfix/r2.9-session-mode
- [x] The shared investigation chat (`SharedConversation`) doesn't continue a Deep chat's agent
      session once the chat is in Clean room; Clean room to Deep still continues — claimed: rmsff,
      branch fix/r2.9-shared-chat-mode
- [x] A Clean room build or question must also not resume an API backend's kept transcript:
      `ws.transcript_path()` is loaded whenever it exists, so switching modes can carry what a Deep
      API build read. Clear it, or record its mode, when the mode changes; add a regression
      — an API transcript records the mode it ran in (`TRANSCRIPT_VERSION` 2); one from the other
      mode, or a version-1 one (counts as Deep), isn't resumed. A same-mode continuation keeps its
      transcript and carries on; a mismatch studies again from the brief. Claimed:
      AlexJoaquimPereira, branch bugfix/r2.9-api-transcript-mode
- [x] Study can still be skipped after a mode switch: a coding agent's `builder_started()` was
      mode-unaware, and an API follow-up message with an existing capture left `first` false even
      when the transcript was rejected. Gate both on the run's mode — `builder_session_in` matches
      the saved session's mode, and a run studies again when its only kept conversation is in the
      other mode; a follow-up with no kept conversation keeps its old path
      — claimed: AlexJoaquimPereira, branch bugfix/r2.9-mode-switch-study
- [ ] Exercise a mode-switched API follow-up end to end: an other-mode transcript, a capture and a
      message must study again, not only the decision helper
- [x] Record which mode wrote `capture/`, so a failed run that leaves a capture from the other mode
      isn't handed to a same-mode conversation; remove Deep-only capture files when a Clean room
      study replaces them
      — claimed: AlexJoaquimPereira, branch bugfix/r2.9-capture-mode
- [ ] A saved Claude `--resume`/`--continue` in the backend's extra arguments is refused in a
      Clean room build or question when it would reuse the project's Deep session, leaving
      unrelated arguments alone; unit and scripted CLI tests cover the build and question paths
      (Clean room refuses, Deep allows), but no live signed-in Claude run is done, so this stays
      open — claimed: AlexJoaquimPereira, branch bugfix/r2.9-saved-resume
- [x] #504: the builder's plan card can be folded away with its X, or with `project_plan_fold`
      (between builds: a fold during a build can be overwritten by its next save). The steps stay
      in the conversation (`FEED_VERSION` 4, `FeedItem::PlanDismissed`), and a later run's
      checklist shows again — claimed: claude, branch bugfix/r2.9-plan-card-dismiss
- [x] A run that ends with steps still open marks them as stopped on its card, instead of
      showing "Doing" until someone folds it (#504: Claude's runs left the plan unticked)
      — claimed: rust-craft, branch feature/R2.9-stop-open-steps; PR #876
- [x] #666: a long line in a feed's tool-work row (a Deep-mode decompile, a status note) ran off
      the window and pushed the feed left; the header title can't wrap in egui, so it's the note's
      first line now, cut at 90 characters, and the full note wraps in the body — claimed: claude,
      branch bugfix/r2.9-feed-text-overflow
- [ ] **decision** A program started from the CLI (`project_play`) keeps the shell's output pipe
      open, so the command doesn't return until it closes (#207). Every fix found needs `unsafe` or
      an unstable API: an exception to rule 9, or a limit we document. Meanwhile a reproducing test
      marked `#[ignore]` and a line in the CLI docs are welcome
- [x] macOS: the window drew Windows-style controls at the top right, since the app turned off the
      system's title bar (`with_decorations(false)`); it now keeps the system's buttons, frame and
      resize edges under a see-through title bar, which is 32 pt there so the buttons sit centred
      in it (#20)
- [ ] macOS: the app's icon is bigger than the system's (#20)
- [ ] Show a command's output while it runs: Claude Code reports a tool's result only when it
      ends, so a long `cargo build` is one quiet line until then. Watching the agent's child
      processes from outside could say what's running and for how long
- [x] Release the keep-awake helper before a rebuild reports `Finished`, waiting up to five seconds
      for its supervised cleanup and logging a timeout. Real subprocess stand-ins checked on Linux (#541)
      — claimed: Afloat16, branch bugfix/r2.9-keep-awake-cleanup-20261103
- [x] Recheck keep-awake cleanup with a real Windows CLI `rebuild`: the helper once outlived it by
      more than 9 minutes and held the caller's output pipe; not reproduced in 8 more runs. Found:
      closing the desktop app during a build ends it without dropping the guard, and the helper
      (an endless PowerShell loop, started with a week's time limit) carried on. It now ends by
      itself once the process it serves is gone: the Windows loop checks that process every 30 s,
      and macOS uses `caffeinate -w`. Live on Windows: a CLI build killed outright, its helper
      ended 30 s later. Not run on a Mac
- [x] Zoom (Cmd/Ctrl + / − / 0) between 50% and 200%: egui's own limits are 20% and 500%, where
      text is unreadable or the window has room for almost nothing
      — claimed: sanketdofe, branch feature/r2.9-zoom-bounds
- [ ] Find why a site capture lost its browser ("the WebSocket is closed", 419 ms into
      `web_site_capture` for a second page, seen once; the same capture passed 41 s earlier)
- [x] Delete a private browser's temporary profile when it closes or fails to start (#810) — PR #822
- [x] Stop private browsers (and Electron apps we drive) that outlive the app after an abrupt
      exit, and delete their profile (#810): a watcher stops the browser's process group once the
      app is gone (`platform::end_with_this_process`, Unix). Unix unit tests added; its script was
      run against real process groups, but not on a Mac with Brave
      — claimed: justinjohn0306, branch bugfix/r2.9-browser-ends-with-app (stopping private browsers)
- [ ] Investigate the Brave Helper crash on launch (#810); needs macOS reproduction
- [ ] Check whether a private browser outlives an abrupt exit on Windows (its Job Object may not
      be killed when closed); add a watcher there if it does
- [ ] Claude Code's Ultracode switch (a setting, not an effort level; the effort environment
      variable we use can't turn it on)
- [x] A model the list doesn't offer (`opusplan`, `[1m]` variants, one another harness serves) can
      be typed in beside the model list in Settings → AI, as `ai model` already allowed (#470)
- [x] The preview server answers only requests addressed to this computer (a Host and Origin
      check against DNS rebinding), streams large files instead of reading them into memory,
      sends no body for HEAD on a missing file, and caps open connections and the time a
      request may take to arrive (#597, #598, #600, #601, by Afloat16)
- [x] The command palette stays where the wheel scrolls it; it follows the selection only when the
      arrow keys or typing move it (#584)
- [x] Linux: draw on the GPU that drives the screen. egui-wgpu's default picks the discrete GPU, so
      on a laptop whose integrated GPU drives the panel the window took 20 s to open and the whole
      desktop lagged (#571). Pick by the connected displays in `/sys/class/drm`: the built-in
      panel's GPU, else the only GPU with displays, else the discrete one; `WGPU_POWER_PREF` and
      `WGPU_ADAPTER_NAME` still decide when set. Checked live on an AMD + NVIDIA laptop only (#578)
      — claimed: Nghiathan13, branch bugfix/r2.9-linux-gpu-choice
- [ ] Linux GPU choice on a desktop with monitors on two GPUs, and on an Intel + NVIDIA laptop:
      covered by unit tests on fake `/sys/class/drm` folders, not run on such a machine yet (#571)
- [x] "Work on my project" on New project, beside "Make my own version" and "Just look inside":
      pick the folder of a project you already have, say what to change, and its files are edited
      in place, with nothing asked; the project takes the folder's name, an empty folder is
      refused and a folder that's already a project opens. Until now that was only reachable
      through "Choose a folder…" under Make's Start, on a screen that asks what to recreate
      — claimed: xerenon, branch feature/r2.9-work-on-my-project; PR #686
- [x] Leaving "Work on my project" forgets a folder it would edit in place and drops a folder
      dialog that ends later, so a version made on Make can't edit that project, and an address
      in its request isn't read as an original to recreate (review of #686) — claimed: xerenon, branch bugfix/r2.9-own-project-review; PR #750
- [x] `build_files_list` lists an edited-in-place project from the project folder itself, not a
      missing or nested `app/`, and leaves Craft Anything's own records out
      — claimed: AlexJoaquimPereira, branch `bugfix/r2.9-build-files-in-place`; commit 1ffeb40
- [x] Choose Claude Code's folder (`CLAUDE_CONFIG_DIR`) per AI, so one computer can build with
      two Claude accounts; an app opened from the Dock never sees the shell's variable. Settings →
      AI (a typed folder, and Add another) and `ai folder`; live-checked from the CLI with an empty
      folder (not signed in, and the fix names the folder) overriding the shell's own variable,
      and by a build in the app with a typed `~/` folder. A sign-in that expired says so instead
      of "isn't signed in" (the folder's `.claude.json` still names the account)
      — claimed: Claude (Opus 5.5), branch feature/claude-config-dir
- [ ] A live build with two Claude Code AIs signed in to two accounts, and one over SSH with a
      `~/` folder (the remote line is unit-tested, not run on another computer)

- [x] The Advanced console's status stays clear of its tabs when the console is narrow
      — claimed: justinjohn0306, branch bugfix/r2.9-console-status-overlap
- [ ] Keep the builder-availability smoke regression independent of live agent discovery.
      — claimed: Afloat16, branch test/r2.9-builder-discovery-fixture-20261104
      Test fixture implemented with controlled discovery; automated validation remains pending.
- [x] macOS: a menu bar with the app's own actions (open a file or folder, new project, settings,
      Simple / Advanced, Clean room / Deep, the command palette, help); the system's menu bar
      showed only the app menu
      — claimed: sanketdofe, branch feature/r2.9-macos-menu-bar
- [x] A Traditional Chinese README (`README.zh-hant.md`) translated from `README.md` and linked
      from the language switcher in every README
      — claimed: monarchsofchaos004-gif, branch docs/readme-zh-hant; PR #926
- [ ] macOS: an Edit menu (Undo, Cut, Copy, Paste, Select All). The keys work in text boxes, but
      the system's Edit items need the text box to answer them, so they'd show greyed out today
- [x] The Advanced console's prompt box starts tall enough to read what's typed (#942)
      — claimed: justinjohn0306, branch bugfix/942-console-input-height
- [x] A French README (`README.fr.md`) translated from `README.md` and linked from the language
      switcher in every README (#831)
- [x] A Thai README (`README.th.md`) translated from `README.md` and linked from the language
      switcher in every README
- [ ] Vietnamese letters with stacked marks (ạ ả ấ ế ơ ư ữ…) aren't in the bundled DM Sans or egui's
      default fonts: without craft-fonts they show as boxes, and with it each such letter comes from
      Noto Sans CJK while the rest of the word is DM Sans. Bundle a DM Sans build that has the
      Vietnamese set (with its OFL notice) so Tiếng Việt renders in one face
- [x] A build told craft-fonts are required (`CRAFT_FONTS_REQUIRED=1`) finished with none
      embedded when `CRAFT_FONTS_DIR` was missing: the build script read the switch only when the
      folder was set and bad. It fails now for no folder, a bad one, and a manifest that lists no
      fonts; `CRAFT_FONTS_REQUIRED` set to nothing or `0` means not required. The decision is
      `plan` in `crates/ui-egui/build.rs`, tested from `tests/craft_fonts_build.rs`. Built for
      real on Windows only
      — claimed: hichang4u, branch bugfix/r2.9-fonts-required
- [x] An engine download the server can't answer right now (a 5xx, 429) says to try again later,
      not "update the pinned URL", which nobody installing an engine can do (#1193)
      — claimed: justinjohn0306, branch bugfix/r2.9-download-server-errors
- [x] Settings → AI: a card for an AI found on this computer wraps its description beside its Add
      button; the line didn't wrap and ran on under the button
      — claimed: jgt87, branch bugfix/r2.9-detected-cards-wrap — PR #1235
- [x] Settings → AI: Providers, Harnesses and Detect share consistent spacing below the discovery
      heading and wrap together, so the Detect action cannot overlap a tab
      — claimed: istaari, branch bugfix/ai-detection-spacing — PR #1530
- [x] The same action reads the same everywhere: a pin's menu said "Show folder" for what the project
      page calls "Open folder" (both `project_open_folder`), and a game's chosen folder was shown as a
      full path in its row, where other chosen folders are shortened, cut to fit and whole on hover
      — claimed: jgt87, branch bugfix/r2.9-same-words-open-folder — PR #1243
- [x] Colours, corners and text sizes off the theme in the chat, evidence, prompt box, sidebar,
      palette and Just ask: the Send button's own white and black, two hand-set corner radii,
      code at 12.5 pt where the rest is 12, an evidence summary at 13/15, and a question drawn in
      the mode's default size where other card titles are 15 semibold
      — claimed: jgt87, branch bugfix/r2.9-theme-tokens-type-scale — PR #1279
- [x] A build's AI cost estimate counts each Claude Code turn's own cost: a resumed session reports
      its running total, so the build summed the whole session at every turn (#1187). A turn's cost
      is now its total less the total kept for its session; a resumed session with nothing kept has
      no cost. Live check (2026-11-09, Haiku 5.5): the second turn's own tokens cost $0.00146751, and
      the reported running total was $0.00350053. Earlier builds' totals stay as they were.
      — claimed: fabox-os, branch bugfix/1187-claude-resumed-cost
- [x] A Deep-mode build told the builder "You haven't decompiled anything yet" after every request
      from the person, though it had read decompiled functions in an earlier run: the flag lived
      only for one run, and each request starts a new one. It is now kept in craft.json
      (`decompiled`), so a run reminds the builder only while it has never decompiled in this
      project. Unit test only; not run live
      — claimed: NeuTroNBZh, branch bugfix/r2.9-decompile-reminder-every-request
- [x] #1421: a run after the first skipped the study, so in Deep mode its builder's tool server had
      nothing open and the first `native_*` calls said "Nothing is open yet". Such a run now opens
      the program the study looked at (`core_target_open`, only when nothing is open, never in
      Clean room); a failure is said in the feed and the builder can still open it. Unit test
      only; not run live
      — claimed: NeuTroNBZh, branch bugfix/1421-reopen-project-target
- [x] `rebuild_check` said "no build is running" when the run had taken the check and one of its
      steps hadn't finished: on a Windows Unity build, three checks in a row stopped at
      "game-self-test" and the builder gave up after its default 20 minutes, below the run's own
      30-minute limit on a check. The run now marks a request it takes, the tool tells "still
      going" (`TimedOut`) from "no run took it", and its default wait (35 min) outlasts the run's
      limit, so the run reports a stuck check itself. Unit tests only; the stuck self-test wasn't
      reproduced (the same game's `--self-test` now exits in 16 s)
      — claimed: NeuTroNBZh, branch bugfix/r2.9-rebuild-check-wait
- [x] Review of #1417: an answer landing as `rebuild_check`'s wait ended could still read as "no
      build is running", since the run removed its mark when it answered; only the asker removes
      it now. A build stopped during a check answers it ("stopped before this check finished")
      instead of leaving the builder waiting, including a stop seen before the check starts
      — claimed: NeuTroNBZh, branch bugfix/r2.9-rebuild-check-mark-race
- [ ] Find why a builder's own check stayed on "game-self-test" for over 20 minutes on Windows,
      past the run's 90 s limit on the game's run (seen three times with a Unity 6000.6 build)
- [x] Let waiting notes scroll with the conversation so a long note leaves the live build
      activity and prompt box reachable — claimed: jeffreylm, branch bugfix/r2.9-keep-build-activity-visible
- [x] Re-running at a folder that already holds a project keeps its saved builder, asset budget,
      `generate_assets` and stack, and uses the typed request as a follow-up instead of dropping it
      (regression from #987); new-project-only options are refused on an attached project
      — claimed: AlexJoaquimPereira, branch bugfix/r2.9-reattach-keeps-settings
- [x] A failed external program's error says "printed last" from the first `stderr_cap` bytes and never
      says the rest was dropped; keep its last output for the message and say when it was cut
      — claimed: AlexJoaquimPereira, branch bugfix/r2.9-stderr-tail
- [x] After #1682: an attached project's run uses its saved builder rather than the Make draft's,
      keeps its live feed when it is already building, and keeps a queued request with its options;
      a start that fails keeps the Make draft
      — claimed: AlexJoaquimPereira, branch bugfix/r2.9-attached-builder-feed
- [ ] A queued run's request lives in its `RunOptions` and the in-memory queue; an app exit before
      it starts loses it. Let the run side keep it (a queue record or a left note) across a restart
- [x] A config.toml that is there but can't be read (another program holds it, no permission, not
      UTF-8) loaded as defaults like a missing one, and the next save replaced it; one that couldn't
      be parsed was renamed aside with the rename's result ignored. Only "not found" is plain
      defaults now. A file that can't be understood (not TOML, not UTF-8) is set aside whole under
      the first free name (`config.toml.broken`, `.broken.1`… up to 16). One that can't be read, or
      can't be set aside, stays where it is, and the defaults standing in for it
      (`Settings::not_loaded`) refuse `save_to` and a deferred save, also after `settings_set`'s
      round trip. Tested on Windows with a file held without sharing; the Unix permission test
      wasn't run here
      — claimed: hichang4u, branch bugfix/r2.9-config-not-loaded
- [ ] When config.toml wasn't loaded (`Settings::not_loaded`), the app says so only in the log and
      when a save fails: say it at startup where the person is looking, with the file's name
- [ ] A config.toml that can't be read for a moment at startup (a virus scan or another program
      holding it) isn't tried again, so the whole session runs on defaults and refuses every save:
      try the read again for a short while before giving up

### R2.10 Pinned builds
Owner: maintainers · Status: in progress
Keep a build that works at hand: pin it, and open it from the sidebar whatever the builder does
to the project next.
- [ ] Say when a light pin ended up copying what it meant to share (a drive without hard links)
      — in PR #220
- [ ] Pins of one project sharing with each other, so a third frozen copy of the same assets
      takes no more space
- [ ] Pin a website (a frozen copy of the built site, each served on its own port)
- [ ] A pin started before the app restarted shows as running (keep its process id and start
      time in the pin, and check both), and can be stopped
- [x] Cap a run's log (`last-run.log`, `play.log`): it grows as long as the program prints. The
      program keeps writing straight to an append-only file, and while we are alive a watcher cuts
      that file back to empty once it passes 1 MiB, so its writes never fail or block and a program
      that outlives us is untouched; past our exit the file grows again, as it did before.
      — claimed: muraa-p, branch bugfix/r2.10-cap-run-log
- [x] The log-cap test assumed a cut lands within a few seconds of the stand-in's printing, so a
      loaded machine could grow the file past the bound with nothing wrong. The stand-in now reads
      its own log and stops once it has seen a cut and written past it
      — claimed: muraa-p, branch test/r2.10-log-cap-waits-for-cut
- [ ] Opening a pin that's already running brings its window to the front, instead of a message
- [x] How much space pins take in Settings
      — claimed: Afloat16, branch feature/r2.10-pinned-copy-storage

---

## M3 Engine depth

An open lane: anyone may take these at any time.

### R3.3 Frida sessions for real
Owner: open · Status: todo
- [ ] Replace the in-memory session bookkeeping with real attach, resume and detach
- [x] The Frida and adb output parsers cut a 64 KiB line at a byte index, so a multibyte character
      at the boundary keeps the whole line; cut at a character boundary and test both (#1095
      follow-up) — a shared `capped_line` helper walks back to a character boundary; regression
      tests cover both parsers and the helper
      — claimed: AlexJoaquimPereira, branch bugfix/r3.3-engine-line-cap
- [x] `adb` `parse_ps` reads its header line before the 64 KiB cap, so a hostile header can
      materialize a full token list; cap it like the body lines
      — claimed: Afloat16, branch bugfix/r3.3-cap-adb-process-header

### R3.9 Game assets out
Owner: open · Status: in progress
- [ ] Unity: sounds out (the stored stream, usually FSB5; PCM ones as WAV)
- [ ] Unity: crunched textures (DXT1Crunched, DXT5Crunched; two of the games checked use them)
- [x] Unity: decode R16 and RG16 texture pixels through the shared bounded PNG export path.
      — claimed: Afloat16, branch feature/r3.9-unity-integer-textures-20261109
      Source `540b4947aa1b`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42633003467/job/129213811395) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642583301/job/129247271153) passed.
- [x] Unity: the float formats (RHalf, RGHalf, RGBAHalf, RFloat, RGFloat, RGBAFloat)
      — claimed: Afloat16, branch feature/r3.9-unity-float-textures
- [x] Decode Unity RGB9e5 shared-exponent textures through the existing bounded picture exporter.
      — claimed: Afloat16, branch feature/r3.9-rgb9e5-textures-20261110
- [ ] Unreal 5 and Oodle: IoStore packages, unversioned properties and Oodle-compressed paks
- [ ] Game files: save pictures from IoStore containers (needs the Unreal 5 support above)
- [x] Decode Unreal packed 10-bit normalized and 11-bit floating-point textures through the bounded picture exporter.
      — claimed: Afloat16, branch feature/r3.9-unreal-packed-textures-20261111
- [x] Unreal: decode cooked half- and full-precision floating-point textures with the shared bounded pixel reader.
      — claimed: Afloat16, branch feature/r3.9-unreal-float-textures-20261109
      Source `75b507c0b96f`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42634877637/job/129220281335) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642583301/job/129247271153) passed.
- [ ] Unreal: sounds and meshes out (so a Deep-mode rebuild needs no outside converter for them)

- [x] Decode Unity EAC single-channel and dual-channel textures through the bounded picture exporter.
      — claimed: Afloat16, branch feature/r3.9-unity-eac-textures-20261111
- [x] Decode Unity ETC1 RGB textures through the bounded picture exporter, including both block modes and clipped edges.
      — claimed: Afloat16, branch feature/r3.9-etc1-textures-20261110
- [x] Decode Unreal DXT3 (BC2) texture blocks, including explicit alpha and clipped edge blocks.
      — claimed: Afloat16, branch feature/r3.9-dxt3-textures-20261110
- [x] Decode Unity RG32, RGB48 and RGBA64 textures without discarding their 16-bit channel layout.
      — claimed: Afloat16, branch feature/r3.9-unorm16-textures-20261110

### R3.11 Single-file apps, game files and stand-in backends
Owner: open · Status: in progress
- [x] Bound native value-flow decoding and incoming parameter nodes by the requested graph node budget, and mark omitted code as partial.
      — claimed: Afloat16, branch bugfix/r3.11-value-flow-budget-20261110
- [x] Resolve AArch64 address evidence only from known operands, with the right arithmetic width and retained writeback effects.
      — claimed: Afloat16, branch bugfix/r3.11-a64-address-proofs-20261110
- [x] AArch64 shifted and extended register operands lose their shift: `sub w9, w9, w9, lsl #3` lifts
      to `w9 -= w9;` and `add w9, w10, w9, lsr #31` to `w9 = w10 + w9;`. Keep the shift or extension
      on the operand and show it in pseudo-C
      — claimed: htcom-code, branch bugfix/r3.11-a64-shifted-operands
- [ ] AArch64 memory operands with an extended index (`ldr w0, [x1, w2, sxtw #2]`) drop the extension:
      the index reads as the plain register, so a sign-extended index looks unsigned in pseudo-C
- [x] Preserve AArch64 exclusive-memory status results and paired load definitions without presenting them as unconditional stores.
      — claimed: Afloat16, branch bugfix/r3.11-a64-exclusive-value-flow-20261110
- [x] Keep the first 100000 byte-search matches in file-offset order across hex, UTF-8 and UTF-16LE queries, including capped pages.
      — claimed: Afloat16, branch bugfix/r3.11-byte-search-match-order-20261110
- [x] Validate the remaining loopback fixture corrections for complete and fragmented request
      headers, negative probes and stand-in cancellation on the current main
      — claimed: Afloat16, branch test/loopback-fixture-stability-20261103
      Revision `cd468aca`, synchronized with main `41d2175e`, passed
      [full Linux CI](https://github.com/storytold/craft-anything/actions/runs/41765850311).
      The fragmented-header regression withholds the final byte until the next read and keeps
      the existing request and response assertions. The request-encoding, negative-probe,
      cancellation and builder-discovery smoke checks passed in the same run.
- [x] Verify stand-in session cleanup by the original answerer's lifetime rather than polling a
      released port that another listener may reuse
      — claimed: Afloat16, branch test/r3.11-standin-session-lifetime-20261103
- [ ] Stand-in backend: writes that change later reads (recorded answers are replayed as they are)
- [ ] Tell the builder about the stand-in backend in a website's brief, and start one from a
      recording
- [x] Stand-in backend: retain 304 samples without a usable cached body for diagnostics, skip them
      during replay, and report `cached_body_unavailable` when the best matching group has no
      usable answer. Build results count them; cached bodies still answer as 200 (#543)
      — claimed: Afloat16, branch bugfix/r3.11-standin-empty-304-20261103
- [ ] Godot format 4 and IL2CPP metadata 38 and 39 are header-only; read their tables once a real
      file can be checked
- [x] Bun: take each module's stored source map and bytecode out as files.
      — claimed: Afloat16, branch feature/r3.11-bun-module-payloads
- [x] A standalone `.pyz`: a PyInstaller PYZ archive given on its own is its own packed reader
      (`pyinstaller_pyz`), listing the program's modules and handing one back as a `.pyc`; a
      Python zip application named `.pyz` stays with the ZIP reader
      — claimed: muraa-p, branch feature/r3.11-standalone-pyz
- [x] Collect native function evidence from every body block and infer parameters from its actual entry block.
      — claimed: Afloat16, branch bugfix/r3.11-native-function-block-evidence-20261110
- [ ] `native_source_folder_write` uses the built-in decompiler only: use Ghidra in Deep mode when
      it's there
- [x] Keep source registers alive when an AArch64 arithmetic result is discarded into the zero register.
      — claimed: Afloat16, branch bugfix/r3.11-a64-discarded-results-20261111
- [x] Keep stand-in response samples distinct when headers, content types or body-availability metadata differ.
      Compare header names without case and ignore ordering between different names, preserving each name's value order.
      At the sample cap, a usable answer can replace an unavailable 304 for the same path, query and request key;
      other requests keep their missing-body evidence. Refused bodies cannot replace that evidence.
      Compare complete samples before charging retained bytes, using #717's retained-budget accounting.
      Historical revision `f79dc0bd` passed [full Linux CI](https://github.com/storytold/craft-anything/actions/runs/41793241151):
      all five original metadata regressions and nine header and capacity regressions passed.
      All 24 combined replay regressions pass locally on the refresh onto `cf414ff4`.
      See this PR's checks for the refreshed revision's complete gate.
      — claimed: Afloat16, branch bugfix/r3.11-standin-sample-metadata-20261103b
- [x] Keep evidence-based call paths within one input binary, rejecting mixed or missing source content identities.
      — claimed: Afloat16, branch bugfix/r3.11-call-path-input-identity-20261110
- [x] `native_publisher_show` hashes the file minus the signature fields, not section by section;
      a file with unusual gaps between sections can read as different — the hash now follows
      Windows' order (headers, each section by its data's file offset, then the bytes after them,
      which Windows counts without the certificate table), the tool no longer warns that a gap
      could read as different, and a regression puts a gap between sections
      — claimed: muraa-p, branch bugfix/r3.11-publisher-hash-sections
- [x] Preserve source registers and link-register writes for AArch64 returns and authenticated control transfers.
      — claimed: Afloat16, branch bugfix/r3.11-a64-control-registers-20261110
- [x] AArch64 move-wide immediates lose their value: `movk w9, #0x9249, lsl #16` lifts to
      `w9 = 0x92490000;`, dropping the low half set just before, and `mov w0, #-1` (MOVN) to
      `w0 = 0;`. Keep MOVK's other bits and invert MOVN's immediate
      — claimed: htcom-code, branch bugfix/r3.11-a64-move-wide-immediates
- [ ] Fold an AArch64 MOVZ/MOVK sequence into one constant in pseudo-C: `mov w9, #0x2493` then
      `movk w9, #0x9249, lsl #16` reads as two statements, not `w9 = 0x92492493;`
- [x] Enforce stand-in per-body and total byte budgets on the text that is actually retained after redaction;
      deduplicate consecutive samples before charging the capture budget.
      — claimed: Afloat16, branch bugfix/r3.11-standin-redacted-budget-20261103b
      Revision `d601bc4e` passed [full Linux CI](https://github.com/storytold/craft-anything/actions/runs/41788735130):
      all four retained-budget regressions and six deduplication regressions passed.
- [x] Preserve every observation when comparing evidence bundles, including repeated operation/subject records and identical-record multiplicity.
      — claimed: Afloat16, branch bugfix/r3.11-bundle-record-multiplicity-20261110
- [x] Read only regular files for binary, text and evidence-bundle inputs, enforce whole-file byte caps while reading, and hash prefixes only after EOF instead of trusting a prior size report.
      — claimed: Afloat16, branch bugfix/r3.11-bounded-file-inputs-20261110
- [x] A text source (a script, Markdown) had no tool that reads it as text outside a build (#539):
      `build_file_read` works only while a project is being built, and the native tools stop at
      "Unknown file magic", while triage pointed to `native_bytes_show`, whose error points back
      to triage. Reproduced on `main` (`0c1f7f7`) with a `.md` and a `.ps1`. `core_text_read`
      (Deep only) reads it paged by lines with coverage, counts invalid bytes and cuts long lines;
      triage names it for a text file it doesn't otherwise recognise
      — claimed: chordpli, branch feature/r3.11-text-source-read
- [x] Keep text-read coverage partial when a line was cut, even when that line is the last one in the file.
      — claimed: Afloat16, branch bugfix/r3.11-cut-text-coverage-20261110
- [x] Live-check #539 in an agent chat (Codex, as reported, and Claude Code): the agent reaches
      `core_text_read` for a script or Markdown target, instead of searching bytes. On macOS with
      `chat --deep` and a `.md` or `.ps1` target: Claude Code 2.1.322 (`.md`) called triage and
      `core_text_read` together; Codex 0.162.1 read the `.ps1` with `core_text_read` straight away,
      and for the `.md` tried `core_overview_show` ("Unknown file magic"), then triage, then
      `core_text_read`. All three quoted the file exactly; none used a byte or string search
      — claimed: chordpli, branch test/r3.11-text-read-live-chat
- [ ] A text file is still "A file we don't recognise yet" everywhere but triage's answer: the
      target's kind stays `unknown`, so `core_targets_list` and triage's `description` say so, and
      Codex tried `core_overview_show` on a `.md` before triage. Give text its own target kind (or
      description) so the list and the app call it text — claimed: Martin-Aziz, branch bugfix/r3.11-text-target-kind
- [x] AArch64 register and bitmask-immediate moves read their zero register: `mov x19, x0` (ORR from
      `xzr`) lifts to `x19 = xzr;` and `mov w0, #0xff00ff00` to `w0 = wzr;`. Move the last operand
      — claimed: htcom-code, branch bugfix/r3.11-a64-mov-alias-source
- [x] A folder walk (`walk_dir`) was bounded only by the files it kept: after `max_files` it went
      on through the whole tree, it read every entry of a folder before looking at a limit, and it
      kept every folder and link, so a tree of empty folders or links, or one huge folder, had no
      limit. `WalkLimits` bounds the entries examined and the path bytes held (from the file limit,
      4,000,000 entries at most); one folder is listed through a bounded first-names-in-order
      selection, so a cut listing is the same every time; the walk stops at the first file or entry
      over a limit; cancel is checked while a folder is read. An ASAR's `.unpacked` folder and
      `source_paths` folders say when their listing was cut. The tree-of-links test skipped itself
      on the Windows account used (it can't create links); it needs a run on Linux or macOS
      — claimed: hichang4u, branch bugfix/r3.11-walk-limits
- [ ] `walk_dir` still reads each `.gitignore` whole, whatever its size: cap it like other input
      reads and say when one was skipped
- [ ] The walk's limits are chosen, not measured (4,000,000 entries at most, 256 path bytes an
      entry): time a walk at the ceiling on a real disk and measure what it holds

### R3.12 Format parser correctness
Owner: open · Status: in progress
- [x] Keep source-to-bundle comparisons uncertain when source-map inventories are truncated, preserving positive matches and reporting partial tool evidence.
      — claimed: Afloat16, branch bugfix/r3.12-source-map-comparison-uncertainty-20261111
- [x] Bound expanded source-map inventories by retained path bytes and entries before allocation, preserving bounded prefixes and reporting truncation.
      — claimed: Afloat16, branch bugfix/r3.12-source-inventory-budget-20261111
- [x] Preserve readable ZIP64 end-record fields with framing warnings and classic fallback for unusable locators; reject undersized, overflowing or truncated fixed records.
      — claimed: Afloat16, branch bugfix/r3.12-zip64-record-extent
- [x] Recover ZIP64 records shifted by prepended data with a bounded lookbehind and preserve the fallback for locator-shaped classic comments.
      — claimed: Afloat16, branch bugfix/r3.12-zip64-prefix-recovery-20261111
- [x] Preserve fractional seconds when projecting binary plist dates and keyed-archive NSDate values, including dates before the reference epoch and rounding across a day boundary.
      — claimed: Afloat16, branch bugfix/r3.12-plist-fractional-dates-20261111
- [x] Mark Python code inventories as cut short whenever a name or constant is shortened, and count truncation markers in the retained-text budget.
      — claimed: Afloat16, branch bugfix/r3.12-pyc-truncated-text-evidence-20261110
- [x] Keep worker, service-worker and utility-process entry modules in their own execution role instead of inheriting the parent role through spawn edges.
      — claimed: Afloat16, branch bugfix/r3.12-worker-role-boundaries
- [x] Read the explicit module-format byte in current Node SEA blobs while retaining the older layouts;
      report the stored format and recover the main script, assets and execution arguments at their real offsets.
      — claimed: Afloat16, branch bugfix/r3.12-node-sea-module-format-20261110
- [x] Deduplicate modules before queueing role propagation, so repeated imports do not exhaust the application graph's traversal limit before reachable modules receive their roles.
      — claimed: Afloat16, branch bugfix/r3.12-module-role-queue
- [x] Resolve webpack module-loader calls by the wrapper parameter's binding, so nested parameters and local variables with the same name do not create dependency edges.
      — claimed: Afloat16, branch bugfix/r3.12-webpack-require-bindings
- [x] Keep sequence splitting from introducing JavaScript strict-mode directives.
      — claimed: Plastic-time, branch rrfz/bugfix-r3.12-js-sequence-directives
- [x] Match numeric bundle module and chunk identities by their JavaScript value, so hexadecimal, separated and exponent literals connect to equivalent dependency references.
      — claimed: Afloat16, branch bugfix/r3.12-bundle-numeric-identities
- [x] Allocate distinct recovered module paths after repeated content-hash collisions.
      — claimed: Plastic-time, branch rrfz/bugfix-r3.12-js-module-collisions
- [x] Refuse oversized GNU tar long names and link targets instead of silently truncating member identities.
      — claimed: Afloat16, branch bugfix/r3.12-tar-long-identities-20261109
      Source [`de2c6390`](https://github.com/Afloat16/craft-anything/commit/de2c63908909cab675887103921ed2601b1bae9b) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the long-name identity regression.
- [x] Refuse managed engine installation when GNU tar name metadata stops the archive walk, before writing files; retain partial listings for inspection and accept complete member data without an end marker.
      — claimed: Afloat16, branch bugfix/r3.12-tar-install-stop-reasons-20261109
      Source `3e588c55` passed [formats and engines tests, formatting, clippy and L0 gates](https://github.com/Afloat16/claude-mem/actions/runs/42649935938/job/129273248608).
      The [unchanged-production baseline](https://github.com/Afloat16/claude-mem/actions/runs/42648642576/job/129268658248) reproduces four failed guards while two compatibility controls pass.
- [x] Require the complete padded extent of GNU tar name and link records before allowing installation, including extents that overflow checked offsets; preserve ordinary member reads without trailing padding.
      — claimed: Afloat16, branch bugfix/r3.12-gnu-tar-padded-records-20261109
      Exact source `25c45ef0` passed [formats and engines tests (1,200 passed, one existing network test ignored), fmt, clippy and L0 gates](https://github.com/Afloat16/claude-mem/actions/runs/42659466852/job/129306793039).
      The [unchanged-production baseline](https://github.com/Afloat16/claude-mem/actions/runs/42659466852/job/129306793039) reproduces four failed guards while seven compatibility controls pass.
- [x] Extract legacy PyInstaller PYZ data as raw files and report modern namespace packages without fabricating bytecode.
      — claimed: Afloat16, branch bugfix/r3.12-pyz-member-kinds
- [x] Bound retained expanded PYZ paths across nested archives before allocating repeated prefixes;
      preserve complete entry identities and declared module counts in partial listings.
      — claimed: Afloat16, branch bugfix/r3.12-pyz-expanded-path-budget-20261110
- [x] Stop a PyInstaller CArchive listing at a member name without its required NUL terminator, preserving earlier readable members.
      — claimed: Plastic-time, branch bugfix/r3.12-pyinstaller-name-terminator-1111
- [x] Keep PyInstaller CArchive member reads before the table and cookie, including signed files with trailing bytes.
      — claimed: Afloat16, branch bugfix/r3.12-carchive-payload-range-20261109
      Source [`a5bcfaed`](https://github.com/Afloat16/craft-anything/commit/a5bcfaedcb1ad67c8bb7f06c7143befc8292409c) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226891917)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.
- [x] Build only the requested keyed-archive object rows and count shared class names without copying them for every object.
      — claimed: Afloat16, branch bugfix/r3.12-keyed-archive-page-before-copy-20261110
- [x] Bound rejected ASAR names by the archive listing limit.
      — claimed: Afloat16, branch bugfix/r3.12-asar-limit-bounds-20261107
- [x] Bound retained ASAR entry and rejected-name paths before expanding shared directory prefixes;
      preserve complete member identities and report a partial listing when the byte budget is reached.
      Extract recovered members with partial coverage and no claimed total when a listing is incomplete.
      — claimed: Afloat16, branch bugfix/r3.12-asar-path-budget-20261110
- [x] Keep embedded Godot pack tables and member reads before the executable's pack trailer.
      — claimed: Afloat16, branch bugfix/r3.12-godot-trailer-boundary-20261109
      Source `9744a0569a7c`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42633003467/job/129213811429) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642583301/job/129247271153) passed.
- [x] Keep .NET bundle member reads before the manifest.
      — claimed: Afloat16, branch bugfix/r3.12-dotnet-data-bounds-20261107
- [x] Bound retained Node SEA asset and argument metadata while still validating the complete blob layout.
      — claimed: Afloat16, branch bugfix/r3.12-node-sea-retained-budget-20261109
      Source `ce34c2446499`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42642329977/job/129246378958) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642329977/job/129246378958) passed.
- [x] Check the decoded byte count for packed files whose manifest declares their original size.
      — claimed: Afloat16, branch bugfix/r3.12-packed-member-size
- [x] Validate gzip packed-member trailers and decode concatenated members within one output budget.
      — claimed: Afloat16, branch bugfix/r3.12-packed-gzip-integrity-20261109
      Source [`3f3ec6a4`](https://github.com/Afloat16/craft-anything/commit/3f3ec6a4bd97bb987d9cad906ce4b2f5028ff88b) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226891924)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.
- [x] Decode all frames of concatenated Zstandard members and account for their combined output.
      — claimed: Afloat16, branch bugfix/r3.12-zstandard-multiple-frames
- [x] Read concatenated XZ streams and four-byte stream padding while sharing one output budget.
      — claimed: Afloat16, branch feature/r3.12-xz-concatenation-20261109
- [x] Recover only the first XZ stream in a SquashFS block, ignoring trailing bytes without losing valid data.
      — claimed: Afloat16, branch bugfix/r3.12-squashfs-xz-first-stream-20261111
- [x] SQLite TEXT values cut at the 4 KiB cap can end in a spurious U+FFFD when the cut lands
      inside a character; cut at a UTF-8/UTF-16 boundary and keep the `truncated` flag
      — tests cover a UTF-8 character, a UTF-16 surrogate pair and a UTF-16 continuation-like
      byte at the cut; invalid bytes keep their lossy prefix. Claimed: AlexJoaquimPereira,
      branch bugfix/r3.12-sqlite-text-cap
- [x] WITHOUT ROWID rows stored before ALTER TABLE ADD COLUMN show the added columns' defaults,
      as rowid tables already do, instead of NULL.
      — claimed: Plastic-time, branch bugfix/r3.12-sqlite-without-rowid-added-columns
- [x] Discard decoded SQLite rows before the requested offset instead of retaining every preceding row, and keep zero-sized pages bounded while preserving readable-row offsets and key order.
      — claimed: Afloat16, branch bugfix/r3.12-sqlite-page-retention-20261110
- [x] Fill added SQLite columns whose DEFAULT is a hex integer or a literal in parentheses
      (`DEFAULT 0x10`, `DEFAULT (5)`); they show NULL with a note today. Hex integers keep
      SQLite's 64-bit two's-complement value; one parenthesized literal is filled, while
      expressions and nested parentheses stay unknown with the note.
      — claimed: AlexJoaquimPereira, branch bugfix/r3.12-sqlite-added-defaults
- [x] Give filled-in SQLite defaults their column's affinity, as SQLite reads them: a TEXT column
      reads `DEFAULT 5` as '5' and `DEFAULT 007` as '7'; an INTEGER or NUMERIC column reads
      `DEFAULT '7'` as 7 and `DEFAULT 5.0` as 5; a REAL column reads `DEFAULT '1.5'` as 1.5; a BLOB
      or untyped column, and an ANY column in a STRICT table, keep the literal as written.
      — claimed: Chr0mium24, branch bugfix/r3.12-sqlite-default-affinity
- [x] Fill added SQLite columns whose DEFAULT is a bare or double-quoted word (`DEFAULT abc`,
      `DEFAULT "abc"`), a signed string (`DEFAULT -'5'`) or a number with digit separators
      (`DEFAULT 1_000`); they show NULL with a note today. Bare and double-quoted words are text
      constants; a signed numeric string and single digit separators take the column's affinity,
      and a separator-bearing REAL fills a missing TEXT field with its unseparated spelling.
      — claimed: AlexJoaquimPereira, branch bugfix/r3.12-sqlite-default-literals
- [x] Read a TEXT column's REAL default as SQLite does: `DEFAULT 1e3` and `DEFAULT 5.` are
      '1000.0' and '5.0' in SQLite. A non-integer REAL default now uses SQLite's REAL text form,
      including `Inf`/`-Inf`; integer literals keep integer text.
      — claimed: AlexJoaquimPereira, branch bugfix/r3.12-sqlite-real-text
- [x] Preserve decimal literal spelling when filling a missing TEXT field from a SQLite default with digit separators.
      — claimed: Afloat16, branch bugfix/r3.12-sqlite-added-default-spellings
- [x] Read a signed quoted numeric default with SQLite's coercion before the column's affinity:
      `DEFAULT -'1e3'` is '-1000.0' on TEXT and -1000 on INTEGER, and `DEFAULT -'1.50'` is '-1.5';
      integer text keeps integer formatting. The quoted text is coerced to an INTEGER or REAL
      before affinity, and TEXT's REAL value uses SQLite's REAL text form.
      — claimed: AlexJoaquimPereira, branch bugfix/r3.12-sqlite-signed-quoted
- [x] Whole numbers in SQLite REAL columns read as REAL: SQLite stores a REAL with no fractional
      part as an integer and turns it back into a REAL when it reads the column, so a REAL, FLOAT
      or DOUBLE column holding 1.0 shows 1.0 instead of 1.
      — claimed: Chr0mium24, branch bugfix/r3.12-sqlite-real-affinity
- [x] Require a complete DNS question's type and class fields before retaining it, and keep
      truncated questions out of packet inventories while reporting the incomplete message
      — claimed: Afloat16, branch bugfix/r3.12-dns-question-class-review-20261104
- [x] Preserve DNS wire-label identity and require valid HTTP start lines before reporting packet evidence.
      — claimed: Afloat16, branch bugfix/r3.12-capture-evidence
- [x] Reject impossible IPv4 header and total lengths.
      — claimed: Afloat16, branch bugfix/r3.12-ipv4-header-lengths-20261105
- [x] Validate the version in IPv6 frames.
      — claimed: Afloat16, branch bugfix/r3.12-ipv6-version-20261105
- [x] Decode VLAN-encapsulated network packets in Linux cooked v1 and v2 captures.
      — claimed: Afloat16, branch bugfix/r3.12-cooked-vlan-packets
- [x] Reject negative GNU base-256 values in unsigned tar header fields.
      — claimed: Afloat16, branch bugfix/r3.12-tar-octal-fields
- [x] Recognize GNU sparse PAX metadata and report unsupported reconstruction instead of returning packed extents as complete files.
      — claimed: Afloat16, branch bugfix/r3.12-tar-pax-sparse-20261109
      Source [`daaeb4fe`](https://github.com/Afloat16/craft-anything/commit/daaeb4fe6c2db3f805fb9b02f9fbdfe55d80eb4a) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895529)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.
- [x] Recognize ZIP directories from DOS directory attributes and Unix file-type metadata when the entry name has no trailing slash.
      — claimed: Afloat16, branch bugfix/r3.12-zip-directory-attributes
- [x] Read ZIP bzip2 and XZ members through the existing bounded pure-Rust decompressors.
      — claimed: Afloat16, branch feature/r3.12-zip-bzip2-xz-20261109
      Source [`4cac3638`](https://github.com/Afloat16/craft-anything/commit/4cac36386e426e332bd89cb2cfa63894f3a367a5) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226892219)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.
- [x] Use a verified Info-ZIP Unicode Path extra field when listing a member stored under a legacy filename.
      — claimed: Afloat16, branch feature/r3.12-zip-unicode-path
- [x] Decode ZIP method 14 property headers and size or end-marker framing with bounded LZMA output.
      — claimed: Afloat16, branch feature/r3.12-zip-lzma-20261109
      Source [`61fdab39`](https://github.com/Afloat16/craft-anything/commit/61fdab39da4849eb5a84196df53c55e19c869134) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226892042)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.
- [x] Stop decoding extension headers inside noninitial IPv6 fragments.
      — claimed: Afloat16, branch bugfix/r3.12-ipv6-fragment-payload-20261105
- [x] Enforce the expanded DNS name wire-length limit.
      — claimed: Afloat16, branch bugfix/r3.12-dns-name-wire-length-20261105
- [x] Report truncated DNS TXT strings as incomplete.
      — claimed: Afloat16, branch bugfix/r3.12-dns-txt-truncation-20261105
      Preserve empty ANY/NONE UPDATE prerequisites with zero TTL; keep TXT validation scoped to parsed answers/prerequisites.
      Preserve empty multicast/unicast DNS-SD TXT records using wire-label service instance names.
- [x] Verify ECDSA keys using their declared named curve and reject missing, malformed or unsupported parameters instead of selecting a curve from the point length.
      — claimed: Afloat16, branch bugfix/r3.12-ec-named-curve-20261110
- [x] Verify RSASSA-PSS signatures using their declared digest, mask and salt parameters, enforce PSS public-key constraints, and require signed and outer certificate parameters to agree.
      — claimed: Afloat16, branch bugfix/r3.12-rsa-pss-parameters-20261110
- [x] Retain readable ASN.1 elements with redundant length encodings, mark non-minimal lengths, and report the not-strict-DER limitation in certificate and signature views.
      — claimed: Afloat16, branch bugfix/r3.12-canonical-der-lengths
- [x] Validate DER algorithm identifiers, signature and key bit strings, and name fields before using them as certificate verification inputs.
      — claimed: Afloat16, branch bugfix/r3.12-der-verification-fields
- [x] Reject certificates whose signed signature-algorithm OID conflicts with the outer algorithm before reporting a verified signature.
      — claimed: Afloat16, branch bugfix/r3.12-certificate-signature-oids-20261110
- [x] Decode complete BMPString and UniversalString values, legacy TeletexString as Latin-1 and the supported ASCII name strings; retain malformed or unknown values as hexadecimal DER instead of lossy text.
      — claimed: Afloat16, branch bugfix/r3.12-certificate-name-identity
- [x] Distinguish literal punctuation and multi-valued attributes in the displayed X.509 issuer and subject.
      — claimed: Afloat16, branch bugfix/r3.12-certificate-name-rendering
- [x] Match CMS signer certificates by issuer and serial number or subject key identifier; leave absent matches unknown instead of selecting an unrelated certificate and chain.
      — claimed: Afloat16, branch bugfix/r3.12-cms-signer-identities-20261110
- [x] Read HTTP Host fields only from the header section.
      — claimed: Afloat16, branch bugfix/r3.12-http-host-boundary-20261105
- [x] Select certificate issuers by successful signature verification among matching names, follow self-issued key rollover, and report interrupted chains as incomplete.
      — claimed: Afloat16, branch bugfix/r3.12-certificate-issuer-selection-20261110
- [x] Distinguish certificate-chain cycles from the 16-link walk limit when explaining why issuer verification stopped (follow-up to #1518).
      — claimed: Afloat16, branch bugfix/r3.12-certificate-cycle-note-20261110
- [x] Keep capture flow timestamps in chronological order.
      — claimed: Afloat16, branch bugfix/r3.12-capture-flow-time-range-20261105
- [x] Reject incomplete simple packet block payloads.
      — claimed: Afloat16, branch bugfix/r3.12-pcapng-simple-block-length-20261105
      Count rejected lengths against the packet scan limit and summarize them once at the end of the walk.
- [x] Bound rejected packet records, validate timestamp fractions and check pcapng section-local interfaces.
      — claimed: Afloat16, branch bugfix/r3.12-capture-timing-framing
- [x] Report capped ZIP, tar and cpio member walks as incomplete firmware extraction, even when skipped directory entries hide later files.
      — claimed: Afloat16, branch bugfix/r3.12-firmware-member-scan-completeness-20261110
- [x] Recognize Unix file modes from macOS ZIP hosts and the DOS directory attribute from VFAT hosts, preserving directory and symlink identity without a trailing slash.
      — claimed: Afloat16, branch bugfix/r3.12-macos-vfat-directory-attributes
- [x] Keep ZIP directory entries within the declared directory.
      — claimed: Afloat16, branch bugfix/r3.12-zip-directory-bounds-20261105
- [x] Refuse split ZIP archives without their other volumes.
      — claimed: Afloat16, branch bugfix/r3.12-zip-multidisk-20261105
- [x] Check Unity bundle blocks against their declared output lengths.
      — claimed: Afloat16, branch bugfix/r3.12-unity-bundle-decompressed-length-20261105
- [x] Read only requested byte ranges from uncompressed Unity bundle blocks so small metadata and asset reads do not allocate or reject entire stored blocks, while preserving size checks and compressed-block caching.
      — claimed: Afloat16, branch bugfix/r3.12-unity-stored-block-ranges-20261110
- [x] Apply standalone LZMA output budgets to its internal dictionary and reject oversized declared output before decoding.
      — claimed: Afloat16, branch bugfix/r3.12-standalone-lzma-budget-20261109
      Source [`5da8880d`](https://github.com/Afloat16/craft-anything/commit/5da8880d3b56785498cf0c09ae46f300f070322d) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226892150)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.
- [x] Expand repeated LZ4 match bytes in blocks when reading Unity bundles.
      — claimed: Afloat16, branch refactor/r3.12-expand-lz4-matches
- [x] Require terminated cpio pathnames and a complete, empty trailer before reporting an archive as complete.
      — claimed: Afloat16, branch bugfix/r3.12-cpio-path-and-trailer
- [x] Identify regular files sharing an inode and device in newc and CRC archives so hard-link aliases are visible in listings.
      — claimed: Afloat16, branch feature/r3.12-cpio-hard-link-evidence
- [x] Locate Unity bundle metadata within the declared bundle size.
      — claimed: Afloat16, branch bugfix/r3.12-unity-bundle-declared-size-20261105
- [x] Check that Unity bundle compressed blocks end before the trailing block table, or the declared
      bundle end for front-positioned tables, so bytes outside the data region cannot supply block contents.
      — claimed: Codex, branch bugfix/r3.12-unity-bundle-data-region; PR #1021
- [x] Check Unity bundle member ranges against the complete uncompressed block stream, retain invalid entries as unreadable with explicit listing warnings, and preserve valid sibling members, cross-block ranges and empty entries.
      — claimed: Afloat16, branch bugfix/r3.12-unity-node-ranges-20261110
- [x] Validate Unity serialized metadata offsets and lengths before reading so short inputs cannot panic or borrow object bytes.
      — claimed: Afloat16, branch bugfix/r3.12-unity-metadata-range-20261109
      Source `de9d7444f509`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42633003467/job/129213811357) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642583301/job/129247271153) passed.
- [x] Reject Unity serialized objects outside their declared file.
      — claimed: Afloat16, branch bugfix/r3.12-unity-serialized-object-ranges-20261105
- [x] Count WebAssembly locals across all declaration groups in retained function bodies.
      — claimed: Afloat16, branch bugfix/r3.12-wasm-locals
- [x] Preserve compression method slots in Unreal pak indexes.
      — claimed: Afloat16, branch bugfix/r3.12-unreal-pak-compression-slots-20261105
- [x] Require complete terminators in Unreal FString values.
      — claimed: Afloat16, branch bugfix/r3.12-unreal-fstring-terminators-20261105
- [x] Count every compression block spanned by an IoStore chunk.
      — claimed: Afloat16, branch bugfix/r3.12-iostore-unaligned-chunk-blocks-20261105
- [x] Reject IoStore directory rows that name a missing chunk or name instead of reporting a zero-byte file or fabricated path.
      — claimed: Afloat16, branch bugfix/r3.12-iostore-directory-references
- [x] Reject truncated IoStore fixed headers.
      — claimed: Afloat16, branch bugfix/r3.12-iostore-complete-header-20261105
- [x] Reject CBOR collections that exceed the EVM metadata reader's limits and invalid text strings.
      — claimed: Afloat16, branch bugfix/r3.12-evm-cbor
- [x] Keep ordinary EVM metadata arrays as arrays, formatting three-part versions only for compiler version fields.
      — claimed: Plastic-time, branch bugfix/r3.12-evm-metadata-array-values-1111
- [x] Keep EVM bytecode intact when its CBOR trailer repeats a metadata key or uses a non-text map key.
      — claimed: Afloat16, branch bugfix/r3.12-evm-cbor-map-keys-20261110
- [x] Require the selector division operand order, the Vyper memory load and the four-byte calldata threshold before reporting those dispatch patterns; invalid jump destinations retain unknown payability.
      — claimed: Afloat16, branch bugfix/r3.12-contract-dispatch-evidence
- [x] Require evidence of a rejecting payment branch before labelling EVM functions nonpayable; reading CALLVALUE alone accepts payments.
      — claimed: Afloat16, branch bugfix/r3.12-evm-payment-guards
- [x] Avoid rescanning every retained EVM selector for repeated dispatcher comparisons.
      — claimed: Afloat16, branch bugfix/r3.12-evm-selector-dedup-20261110
- [x] Reject invalid marshal long integer digits.
      — claimed: Afloat16, branch bugfix/r3.12-pyc-long-limb-validation-20261105
- [x] Check expanded RGBA image sizes and enforce output budgets before allocation.
      — claimed: Afloat16, branch bugfix/r3.12-pixel-rgba-output-overflow-20261105
- [x] Reject duplicate keys in safetensors headers before they silently replace tensor descriptions or model metadata.
      — claimed: Afloat16, branch bugfix/r3.12-safetensors-duplicate-keys-20261110
- [x] Report malformed safetensors metadata, element widths and payload coverage.
      — claimed: Afloat16, branch bugfix/r3.12-safetensors-layout
- [x] Disclose incomplete asset catalog tree reads instead of reporting silently empty complete listings.
      — claimed: Afloat16, branch bugfix/r3.12-car-tree-completeness-20261110
- [x] Read Apple BOM block indices and named variables only within the directory byte ranges declared by the container.
      — claimed: Afloat16, branch bugfix/r3.12-bom-directory-bounds
- [x] Count empty tensor shapes consistently across axis orders.
      — claimed: Afloat16, branch bugfix/r3.12-tensor-zero-dimension-product-20261105
- [x] Read certificate choices from the SignedData certificate set only and enforce the certificate limit across the blob.
      — claimed: Afloat16, branch bugfix/r3.12-signeddata-certificate-set
- [x] Find APK signatures through valid ZIP comments and framed central directories.
      — claimed: Afloat16, branch bugfix/r3.12-apk-sign-eocd-comment-boundary-20261105
- [x] Require APK signer SDK ranges and ordered signature-algorithm lists to match their signed copies, with complete required signer fields, before reporting verification.
      — claimed: Afloat16, branch bugfix/r3.12-apk-signed-metadata-20261110
- [x] Include an Android package's type-ID offset in resource identities while retaining its local type-name indices; reject shifted IDs outside the one-byte type field.
      — claimed: Afloat16, branch bugfix/r3.12-arsc-type-id-offset-20261110
- [x] Keep Android resource entry data after the offset table; retain readable identities with a warning when bag headers or records are incomplete, without inventing parent or bag values.
      — claimed: Afloat16, branch bugfix/r3.12-resource-table-identity
- [x] Keep Android resource packages, child chunks and library records inside their enclosing declared ranges.
      — claimed: Afloat16, branch bugfix/r3.12-arsc-parent-chunk-boundaries-20261109
      Source `ea680eeb9e92`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895431) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642583301/job/129247271153) passed.
- [x] Retain carrier zero, grammatical gender, round-screen and color capabilities, and keyboard/navigation availability in resource configuration names.
      — claimed: Afloat16, branch bugfix/r3.12-resource-device-variants
- [x] Keep explicit script and variant subtags when grouping Android resource strings by locale.
      — claimed: Afloat16, branch bugfix/r3.12-android-script-locale
- [x] Bound decoded Android string-pool bytes and disclose partial pools in binary XML and resource tables, so shared offsets cannot multiply a small chunk into unbounded retained strings.
      — claimed: Afloat16, branch bugfix/r3.12-android-pool-retained-bytes-20261110
- [x] Carry incomplete Android manifest decoding into package and component evidence coverage, including the last component page, without substituting ZIP-entry counts for document completeness.
      — claimed: Afloat16, branch bugfix/r3.12-android-manifest-coverage-20261110
- [x] Read 16-bit native pointers without consuming adjacent bytes.
      — claimed: Afloat16, branch bugfix/r3.12-binary-pointer
- [x] Keep same-named imported functions attached to their own library and preserve an explicitly recorded pointer slot.
      — claimed: Afloat16, branch bugfix/r3.12-import-library-slots-20261109
- [x] Preserve a unique versioned ELF import slot when relocation records omit the library name (follow-up to #1365).
      — claimed: Afloat16, branch bugfix/r3.12-elf-versioned-import-slot-fallback-20261109
      — completed: 1179870c55203d946bb8a7412b9b8f1088352fe3
- [x] Keep Mach-O load-command facts within their command.
      — claimed: Afloat16, branch bugfix/r3.12-macho-load-command-framing-20261105
- [x] Resolve Mach-O UNIXTHREAD entry points from a complete state record for the image CPU, including later state records.
      — claimed: Afloat16, branch bugfix/r3.12-macho-thread-state-20261109
- [x] Prevent Mach-O zero-fill sections from exposing unrelated file bytes during section lookup.
      — claimed: Afloat16, branch bugfix/r3.12-macho-zerofill-section-data
- [x] Keep native address translations and reads inside both the mapped range and its file-backed bytes.
      — claimed: Afloat16, branch bugfix/r3.12-binary-padding
- [x] List each Objective-C class and category address once when ordinary and non-lazy registration tables overlap, keeping distinct same-named definitions separate.
      — claimed: Afloat16, branch bugfix/r3.12-objc-nonlazy-identity-20261111
- [x] Require complete Objective-C method record strides before reading absolute or relative method fields.
      — claimed: Afloat16, branch bugfix/r3.12-objc-method-record-stride-20261109
- [x] Keep Mach-O function starts within their declared payload.
      — claimed: Afloat16, branch bugfix/r3.12-macho-function-starts-payload-boundary-20261105
- [x] Apply one indirect-import budget across all Mach-O sections and report when repeated table references exhaust it.
      — claimed: Afloat16, branch bugfix/r3.12-macho-indirect-global-budget-20261109
- [x] A Mach-O executable's entry point comes from the segment whose file bytes hold its
      `LC_MAIN` offset. A segment with no file bytes, such as a `__DATA` that holds only
      zero-fill sections, records file offset 0 like `__TEXT`, and the entry was placed in that
      segment instead of in `__TEXT` (`/bin/cat` reads 0x100008588 for 0x100000588).
      — claimed: Chr0mium24, branch bugfix/r3.12-macho-entry-zero-fill
- [x] Name DriverKit and visionOS simulator Mach-O binaries in the binary view: its platform table
      stopped at 9 and 11, so a `.dext` read "unknown" while the dependency view said "driverkit".
      — claimed: Chr0mium24, branch bugfix/r3.12-macho-platform-names
- [x] Swift symbols with a protocol type decode. In a protocol list (`…_p`) each protocol is a
      context and a name with no `P` kind letter, but the demangler expected a type there, so an
      existential of a protocol named this way was reported invalid ("expected a type"):
      `_$sSS6encode2toys7Encoder_p_tKF` is `Swift.String.encode(to: Swift.Encoder) throws -> ()`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-existentials
- [x] Swift function type attributes decode. Besides `Ya` (async), a function type can carry `Yb`
      (`@Sendable`), `Yc` (a global actor, with its type), `YA` (`@isolated(any)`) and `YK` (the
      error type of `throws(E)`), but the demangler read only `Ya` and reported the rest invalid
      ("unsupported function attribute"): `_$s1x2v1yyYbcvp` is `x.v1 : @Sendable () -> ()`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-function-attributes
- [x] Swift value witness symbols decode. A symbol ending `w` and two letters is one of the
      functions the runtime calls to copy, move, destroy or read the enum tag of a value of a
      type, but the demangler had no `w` operator and reported it invalid ("unsupported operator
      'w'"): `_$ss6ResultOwug` is `getEnumTag value witness for Swift.Result`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-value-witnesses
- [x] Swift lazy witness table cache variables decode. A symbol ending `WL` names a type and
      the conformance it is used with, as `Wl` does, but the demangler read only one operand and
      reported it invalid ("N nodes left after decoding"):
      `_$s7SwiftUI4EdgeO3SetVAESQAAWL` is `lazy protocol witness table cache variable for type
      SwiftUI.Edge.Set and conformance SwiftUI.Edge.Set : Swift.Equatable in SwiftUI`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-witness-cache
- [x] Swift enum case symbols decode. A symbol ending `WC` is the function that builds an enum
      case, but the demangler read `WC` as an unknown witness kind and reported it invalid
      ("unsupported witness kind"): `_$s10FindMyBase3BitO3oneyA2CmFWC` is
      `enum case for FindMyBase.Bit.one(FindMyBase.Bit.Type) -> FindMyBase.Bit`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-enum-cases
- [x] Swift symbols decode opaque result types. A function, variable or subscript declared to
      return `some P` mangles that result as `Qr` (`QR` for a second or later one in a tuple),
      and a use of it elsewhere as `QO` and `Qo`, but the demangler reported all of them invalid
      ("unsupported associated type"): `_$s1a2mkQryF` is `a.mk() -> some`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-opaque-types
- [x] Swift symbols decode private declarations. A private or `fileprivate` declaration's name
      is followed by an identifier for its file and `LL`, and the demangler reported every such
      symbol invalid ("unsupported operator 'L'"):
      `_$s10DVTIconKit12_isDebugging33_8F51EEEBC836B21F9ED44F4C6D38A8CBLLSbvp` is
      `DVTIconKit.(_isDebugging in _8F51EEEBC836B21F9ED44F4C6D38A8CB) : Swift.Bool`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-private-names
- [x] Swift symbols decode outlined value operations. The demangler reported every `WO` and a letter
      (`WOh`, `WOc`, `WOi0_`) as "unsupported witness kind", so `_$s10ACPToolLib11ResetSchemaVWOh`
      should read "outlined destroy of ACPToolLib.ResetSchema", as `swift-demangle` prints it.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-outlined-ops
- [x] Swift symbols name the right type after an optional, an associated type or an extension.
      The demangler's substitution list left out `Sg` optionals and `Qz`/`Qy` associated types
      and added extensions, so a later `A…` reference named the wrong node
      (`[Swift.String : Swift.String]` where the symbol says `Swift.String?`) or fell out of range.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-substitutions
- [x] Charge saved Swift substitution trees and repeated copies against the expansion budget before cloning.
      — claimed: Afloat16, branch bugfix/r3.12-swift-substitution-copy-budget-20261110
- [x] Swift symbols read `SQ` as `Swift.Equatable`. The demangler's table of standard types mapped
      it to the enum `Swift.ImplicitlyUnwrappedOptional`, so `_$sSQMp` read "protocol descriptor for
      Swift.ImplicitlyUnwrappedOptional", a requirement read `A: Swift.ImplicitlyUnwrappedOptional`,
      and `Equatable` conformance descriptors and witness tables (`…SQAAMc`, `…SQsWP`) were
      reported invalid ("expected a protocol").
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-equatable
- [x] Swift symbols spell the concurrency types as `Sc` and a letter. The demangler reported all of
      them invalid ("concurrency standard types are not supported"), so `_$sScAMp` should read
      "protocol descriptor for Swift.Actor" and `ScT`, `ScG`, `ScS` and the rest their own types,
      with an unknown letter after `Sc` still invalid.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-concurrency-types
- [x] Swift symbols name the type metadata singleton initialization cache. The demangler read
      `Ml` with the name of `ML`, so `_$s10RealityKit30__RKEntityTriggerSpecificationOMl` read
      "lazy cache variable for type metadata for RealityKit.__RKEntityTriggerSpecification" where
      `swift-demangle` prints `type metadata singleton initialization cache for …`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-singleton-cache
- [x] Swift one-time initialization symbols name the variable. `WZ` and `Wz` were invalid; they now
      read "one-time initialization function for" and "one-time initialization token for" followed
      by the variable's name, as `swift-demangle` prints them. For example
      `_$s10ACPToolLib11RemoveTokenV13configuration_WZ` reads "one-time initialization function for configuration".
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-once-init
- [x] Require a complete Cargo package version before a registry or vendor path becomes Rust dependency evidence.
      — claimed: Afloat16, branch bugfix/r3.12-rust-dependency-versions
- [x] Swift symbols name the right generic parameter in an associated type. `Qy` numbers its
      parameter the way `q` does (`Qy_` is the second parameter `B`, `Qy0_` the third), but the
      demangler read `Qy_` as `A`, so `Swift.Zip2Sequence.Iterator.next()` returned
      `(A.Element, A.Element)?` where the symbol says `(A.Element, B.Element)?`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-associated-type-index
- [x] Swift metadata symbols name their kind the way `swift-demangle` does. `MN` reads as
      noncanonical specialized generic type metadata (`_$sSRys6UInt16VGMN`), `Mj` as an opaque
      type descriptor accessor key and `Mz` as a flag for loading canonical specialized generic
      type metadata. The demangler printed `type metadata for`, `module descriptor` and `type
      metadata heap reference for` for them.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-metadata-names
- [x] Decode binary plist markers and dictionary keys according to their stored types.
      — claimed: Afloat16, branch bugfix/r3.12-bplist-typed-objects
- [x] Swift initialisers of structs, enums, protocols and extensions read `init`. The demangler
      printed every allocating initialiser (`fC`) as `__allocating_init`, a name `swift-demangle`
      keeps for a class's own initialisers, so `_$sSdyS2dcfC` read
      `Swift.Double.__allocating_init(Swift.Double) -> Swift.Double` for
      `Swift.Double.init(Swift.Double) -> Swift.Double`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-init-names
- [x] Swift operator functions decode. A function named by an operator is written as the
      operator's letters and `oi` (infix), `op` (prefix) or `oP` (postfix), and the demangler
      reported every one invalid (`unsupported operator 'o'`), so `_$s10Foundation4DateV2eeoiySbAC_ACtFZ`
      now reads `static Foundation.Date.== infix(Foundation.Date, Foundation.Date) -> Swift.Bool`.
      Private operators, and the witnesses, thunks and descriptors around an operator function, decode too.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-operator-names
- [x] Swift arguments without a label read `_:` when another argument of the same function,
      initialiser or subscript has one. The demangler printed the bare type, so
      `_$s10Foundation13__DataStorageC6append_6lengthySV_SitF` read
      `Foundation.__DataStorage.append(Swift.UnsafeRawPointer, length: Swift.Int) -> ()` where
      `swift-demangle` prints `append(_: Swift.UnsafeRawPointer, length: Swift.Int) -> ()`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-unlabeled-args
- [x] Swift witness table accessors decode. The demangler read `Wl`, `Wb`, `WT` and `WI` as unknown
      witness kinds and reported them invalid ("unsupported witness kind"):
      `_$s7SwiftUI4EdgeO3SetVAESQAAWl` is `lazy protocol witness table accessor for type
      SwiftUI.Edge.Set and conformance SwiftUI.Edge.Set : Swift.Equatable in SwiftUI`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-witness-accessors
- [x] Apply the extracted-string limit in file-offset order across UTF-16LE and narrow strings, so an early wide string is not hidden by later ASCII.
      — claimed: Afloat16, branch bugfix/r3.12-earliest-extracted-strings
- [x] Keep quoted greater-than signs inside XML plist attributes instead of ending the tag early.
      — claimed: Plastic-time, branch bugfix/r3.12-plist-quoted-tag-delimiters-1112
- [x] Visit XML plist values only inside a complete, correctly encoded document.
      — claimed: Afloat16, branch bugfix/r3.12-xml-plist-document
- [x] Normalize literal XML line endings before decoding plist text, keeping character references intact.
      — claimed: Plastic-time, branch bugfix/r3.12-plist-xml-line-endings-1111
- [x] End JavaScript line comments at every language line terminator, preserving following tokens for the nesting admission guard and lexer fallback.
      — claimed: Afloat16, branch bugfix/r3.12-js-line-comment-boundaries
- [x] Read and edit JSONC registrations whose line comments end at a bare carriage return.
      — claimed: Afloat16, branch bugfix/r3.12-jsonc-carriage-return-comments
- [x] Name code-directory flags with the bits Apple uses: hard 0x100, kill 0x200, expires 0x400,
      restrict 0x800, enforcement 0x1000, library-validation 0x2000. The table used 0x4–0x80 for
      them and named 0x200 and 0x800 as other flags, so a library-validation signature (0x2000)
      showed no flag names and 0x12200 read "allowed-entitlements,runtime".
      — claimed: Chr0mium24, branch bugfix/r3.12-codesign-flag-names
- [x] Keep code-signature children and verification reads within their declared superblob.
      — claimed: Afloat16, branch bugfix/r3.12-codesign-superblob-length-boundary-20261105
- [x] Require complete, single-valued CMS message-digest and content-type signed attributes with the specified tags, and bind the content type to the encapsulated content before reporting authentication.
      — claimed: Afloat16, branch bugfix/r3.12-cms-signed-attributes-20261110
- [x] Reject out-of-range or unterminated library names in the ELF dynamic string table.
      — claimed: Afloat16, branch bugfix/r3.12-elf-dynamic-names
- [x] Recover dynamic symbols through PT_DYNAMIC when an ELF retains other section headers but has no usable dynamic-symbol section.
      — claimed: Afloat16, branch bugfix/r3.12-elf-partial-section-symbols-20261109
- [x] Refuse short ELF64 header and section records and handle section-table offsets at the address limit safely.
      — claimed: Afloat16, branch bugfix/r3.12-sbpf-records
- [x] Keep ELF version definitions, requirements and indices within their declared section byte ranges.
      — claimed: Afloat16, branch bugfix/r3.12-elf-version-section-bounds-20261109
- [x] Keep PE optional-header reads within the declared header.
      — claimed: Afloat16, branch bugfix/r3.12-pe-optional-header-framing-20261105
- [x] Bound import descriptor walks by mapped file bytes and report incomplete metadata. Normal imports keep
      following the terminator when their declared size is short or zero; delay-load and bound metadata
      also respect their directory sizes. Preserve the shared retained-text budget.
      — claimed: Afloat16, branch bugfix/r3.12-pe-import-directory-bounds-20261109
- [x] Require bound PE module and forwarder names to include their NUL terminator inside the declared directory.
      — claimed: Afloat16, branch bugfix/r3.12-bound-import-terminators-20261111
- [x] Use the PE image's declared header extent when an RVA is not mapped by a section.
      — claimed: Afloat16, branch bugfix/r3.12-pe-header-rva-span
- [x] Keep PE entry points, import slots and exception function starts from wrapping image-base arithmetic into unrelated addresses.
      — claimed: Afloat16, branch bugfix/r3.12-pe-virtual-address-overflow-20261109
- [x] Keep relocation symbol indices inside a linked symbol table and require a linked string table before reporting a syscall name.
      — claimed: Afloat16, branch bugfix/r3.12-sbpf-symbols
- [x] Keep PE resource metadata within the resource directory.
      — claimed: Afloat16, branch bugfix/r3.12-pe-resource-directory-boundary-20261105
- [x] Bound PE export-table work and returned aliases even when function slots are empty or one function has many names.
      — claimed: Afloat16, branch bugfix/r3.12-pe-export-work-budget-20261109
- [x] Keep PE version strings whose value is empty (stored with no value at all), such as
      `OleSelfRegister`, whose presence is the whole point.
      — claimed: Plastic-time, branch bugfix/r3.12-pe-empty-version-strings
- [x] Keep fixed PE file and product versions inside a complete declared VS_FIXEDFILEINFO value and its enclosing version node.
      — claimed: Afloat16, branch bugfix/r3.12-pe-fixed-version-bounds-20261110
- [x] Read PE version strings stored as binary (wType 0), which some linkers write; they're
      dropped today (`FileDescription` in `pthreadVC2.dll`). A binary leaf at the string depth is
      decoded as UTF-16LE; a binary node with children stays a container.
      — claimed: AlexJoaquimPereira, branch bugfix/r3.12-pe-binary-version-strings
- [x] Decode inline source-map data URLs without treating their URL fragments as payload; recognize case-insensitive scheme and base64 markers.
      — claimed: Afloat16, branch bugfix/r3.12-source-map-data-url-fragment
- [x] Apply WebAssembly name and producer inventory limits across repeated custom sections and report truncation.
      — claimed: Afloat16, branch bugfix/r3.12-wasm-custom-inventory-budget-20261109
- [x] Bound bytes retained when WebAssembly types and function names are expanded through references, reuse formatted type signatures, and disclose omitted derived metadata.
      — claimed: Afloat16, branch bugfix/r3.12-wasm-reference-bytes-20261110
      Source `232100030ace`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42642340046/job/129246412808) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642340046/job/129246412808) passed.
- [x] Report WebAssembly metadata truncation accurately in summary and complete tool views, including byte-budget exhaustion before entry limits (follow-up to #1540).
      — claimed: Afloat16, branch bugfix/r3.12-wasm-metadata-limits-20261110
- [x] Accept only valid WebAssembly names when reporting custom-section debug and source map URLs.
      — claimed: Afloat16, branch bugfix/r3.12-wasm-debug-names
- [x] Decode the full signed 32-bit source map VLQ range and refuse mapping fields outside it.
      — claimed: Afloat16, branch bugfix/r3.12-sourcemap-vlq-range
- [x] Reject source map mappings with missing source or name entries.
      — claimed: Plastic-time, branch rrfz/bugfix-r3.12-mapping-index-20261110
- [x] Group Go functions by the declaring package when a generic argument names another import path.
      — claimed: Afloat16, branch bugfix/r3.12-go-generic-owner
- [x] Keep Go function tables and names within their section.
      — claimed: Afloat16, branch bugfix/r3.12-go-pclntab-section-boundary-20261105
- [x] Preserve complete Go functions before a damaged later table entry and report partial coverage when the scan stops at structural damage or its unreadable-name limit.
      — claimed: Afloat16, branch bugfix/r3.12-go-pclntab-prefix-20261110
- [x] Bound retained Go function-name bytes when pclntab records share name references, preserving recovered functions and explicitly disclosing omitted names.
      — claimed: Afloat16, branch bugfix/r3.12-go-name-bytes-20261110
- [x] Keep the report's function display limit from marking a valid large WebAssembly module as mismatched.
      — claimed: Afloat16, branch bugfix/r3.12-wasm-function-count
- [x] Retain decoded CLI signatures with trailing data and report the unused bytes as a limitation, while keeping nested function-pointer signatures incremental.
      — claimed: Afloat16, branch bugfix/r3.12-cli-signature-consumption-20261111
- [x] Preserve warnings from every view used by the managed application graph, deduplicating diagnostics before the display limit.
      — claimed: Afloat16, branch bugfix/r3.12-managed-graph-warnings-20261111
- [x] Reject TypeSpec handles where CLI class/value-type and generic signatures require TypeDef or TypeRef, while retaining standalone TypeSpec and custom-modifier decoding.
      — claimed: Afloat16, branch bugfix/r3.12-dotnet-signature-type-handles-20261110
- [x] Keep CLI multidimensional array ranks and bounds distinct from vector signatures.
      — claimed: Afloat16, branch bugfix/r3.12-cli-array-shapes-20261111
- [x] Recognize volatile type notation from required `System.Runtime.CompilerServices.IsVolatile` modifiers across the complete modifier list.
      — claimed: Afloat16, branch bugfix/r3.12-dotnet-volatile-modifiers-20261110
- [x] Preserve complete Unicode characters in shortened .NET strings.
      — claimed: Afloat16, branch bugfix/r3.12-dotnet-user-string-surrogate-limit-20261105
- [x] Reject CIL branch and switch destinations outside the method body without wrapping signed displacements.
      — claimed: Afloat16, branch bugfix/r3.12-cil-branch-target-ranges-20261111
- [x] Validate CIL method exception-section kinds, extents and complete clause records before reading exception handlers.
      — claimed: Afloat16, branch bugfix/r3.12-cil-exception-framing-20261110
- [x] Preserve distinct nonfinite floating-point constants when hashing normalized CIL instructions instead of collapsing them to JSON null, retaining original single-precision NaN bits before widening.
      — claimed: Afloat16, branch bugfix/r3.12-cil-nonfinite-hashes-20261110
- [x] Read Dart version hashes and feature strings only from the bytes belonging to that snapshot.
      — claimed: Afloat16, branch bugfix/r3.12-dart-snapshot-extent
- [x] Keep Dart string-pool duplicate tracking within the retained URI and token limits.
      — claimed: Afloat16, branch bugfix/r3.12-dart-retained-string-budgets-20261110
- [x] Read a Dart ELF build ID only when its name and descriptor fit the declared note.
      — claimed: Afloat16, branch bugfix/r3.12-dart-note
- [x] Locate ELF core mapping names after the complete declared table.
      — claimed: Afloat16, branch bugfix/r3.12-elf-core-file-name-count-cap-20261105
- [x] Advance ELF core NT_FILE path cursors by complete original byte strings, even when a displayed path is lossy or capped.
      — claimed: Afloat16, branch bugfix/r3.12-elf-core-path-cursor-20261109
- [x] Validate index source map section offsets and stop an unresolved section from borrowing mappings from the preceding section.
      — claimed: Afloat16, branch bugfix/r3.12-index-map-boundaries
- [x] Preserve the URL delimiter in source-map roots when resolving a relative source file.
      — claimed: Afloat16, branch bugfix/r3.12-source-map-url-root
- [x] Apply a nonempty source-map root prefix before interpreting a source path, including leading slashes and URL-shaped sources, while preserving empty-root compatibility.
      — claimed: Afloat16, branch bugfix/r3.12-source-root-prefix-20261111
- [x] Preserve embedded source-map contents when safe recovery paths collide, keeping each source identity and index in the manifest.
      — claimed: Afloat16, branch bugfix/r3.12-source-recovery-collisions-20261111
- [x] Bound retained path bytes in source-map recovery manifests before repeating expanded source roots, and report partial recovery.
      — claimed: AlexJoaquimPereira, branch bugfix/r3.12-source-recovery-path-budget-20261111
- [x] Bound the source-map parser's retained `sources`/`sourcesContent` entries before building millions of small strings from a 64 MiB map.
      — claimed: NicolasBohn, branch bugfix/r3.12-sourcemap-entry-bounds
- [x] Bound the number of index-map sections the source-map parser keeps; each section, even an empty external one, still becomes a map of its own.
      — claimed: NicolasBohn, branch bugfix/r3.12-sourcemap-entry-bounds-review
- [x] A malformed source map (`BadJson`, `BadVersion`, bad mappings or sections) and a stand-alone `.map` file without a
      comment pointing at it still leave their sources looking absent in an application graph: say they couldn't be read,
      so a source comparison doesn't confirm those sources missing.
      — claimed: NicolasBohn, branch bugfix/r3.12-unreadable-source-maps
- [x] A source map refused for bad mappings or sections still has a readable `sources` list: keep those sources with a
      warning instead of reporting the whole map unreadable.
      — claimed: NicolasBohn, branch bugfix/r3.12-sources-of-bad-mappings
- [ ] An index-map section that points at an external map (`url`) adds no sources, so an application graph still
      treats the map's list as complete: mark those sources uncertain until the external map is read.
- [x] Recognize source-map annotations with no space, tabs or repeated spaces after the marker, and reject whitespace inside their URLs.
      — claimed: Plastic-time, branch bugfix/r3.12-sourcemap-annotation-spacing-1111
- [x] Keep source map annotation URL suffixes intact.
      — claimed: Plastic-time, branch rrfz/bugfix-r3.12-annotation-20261110
- [x] Keep DOS relocations inside the declared executable header.
      — claimed: Afloat16, branch bugfix/r3.12-dos-relocation-header-boundary-20261105
- [x] Report contradictory DOS executable size declarations as header problems.
      — claimed: Plastic-time, branch rrfz/bugfix-r3.12-mz-header-20261110
- [x] Keep relocation reads within their declared entry stride and never invent an addend for a truncated RELA.
      — claimed: Afloat16, branch bugfix/r3.12-sbpf-rela
- [x] Use the sorted section and segment boundaries during source-map position lookups so repeated symbolication visits only matching positions.
      — claimed: Afloat16, branch bugfix/r3.12-source-map-lookup-work
- [x] Bound generated source-map lines across indexed sections, including empty lines that consume no segment budget.
      — claimed: Afloat16, branch bugfix/r3.12-sourcemap-line-budget-20261110
- [x] Parsers refuse what they used to half-read: binary plists keep objects inside their object
      area and reject unrepresentable widths, base64 (plists, source maps) refuses damaged tails
      and incomplete quanta through one shared decoder, OIDs must be minimally encoded, and string
      offsets past the address space are left out. Tar keeps old GNU timestamps out of names and
      PAX sizes across other metadata records; CRC cpio checks member data; source maps decode
      percent-escaped data URLs and report indexed-map columns in the generated file
      (#606–#615, by Afloat16)
- [x] Use the existing bounded raw LZ4 decoder for SquashFS metadata, regular data blocks and tail fragments while retaining Unity's exact decoded-size contract.
      — claimed: Afloat16, branch feature/r3.12-squashfs-lz4-20261110
- [x] Report truncated SquashFS metadata blocks and retain previously recovered blocks.
      — claimed: Afloat16, branch bugfix/r3.12-squashfs-metadata-warning
- [x] Require each raw or decoded file block to cover exactly its logical range, including the partial final block, so short blocks cannot shift later contents and long blocks cannot be hidden by truncation.
      — claimed: Afloat16, branch bugfix/r3.12-squashfs-block-output-size-20261110
- [x] Recover zero-filled cramfs hole blocks at full and partial page boundaries while retaining the existing recovery budgets.
      — claimed: Afloat16, branch bugfix/r3.12-cramfs-hole-blocks-20261110
- [x] Charge firmware ZIP outputs before decoding the next member, so byte and file limits bound retained member data.
      — claimed: Afloat16, branch bugfix/r3.12-firmware-zip-output-budget-20261110
- [x] Skip inconsistent JFFS2 data nodes and recover files when later valid nodes cover their damaged extents within the final file size.
      — claimed: Afloat16, branch bugfix/r3.12-jffs2-decoded-node-lengths-20261111
- [x] Require complete, structurally consistent extent nodes and child depths before returning a file mapping; missing or invalid metadata leaves the file unrecovered instead of replacing unknown ranges with zeros.
      — claimed: Afloat16, branch bugfix/r3.12-ext4-unavailable-extent-map-20261110
- [x] Bound filesystem and archive recovery by the recorded image, node, directory and member limits.
      — claimed: Afloat16, branch bugfix/r3.12-filesystem-archive-bounds
- [x] Use the version-two cramfs size to bound root, directory and data reads in either byte order while preserving version-one compatibility.
      — claimed: Afloat16, branch bugfix/r3.12-cramfs-v2-image-size-20261110
- [x] Mark cramfs recovery incomplete when a file block-pointer table is unavailable.
      — claimed: Afloat16, branch bugfix/r3.12-cramfs-pointer-completeness-20261111
- [x] Recover cramfs direct block pointers, including shared uncompressed pages and length-prefixed compressed blocks.
      — claimed: Afloat16, branch feature/r3.12-cramfs-direct-block-pointers-20261111
- [x] Recover files and symlinks from cramfs blocks marked uncompressed while preserving mixed compressed runs and recovery limits.
      — claimed: Afloat16, branch bugfix/r3.12-cramfs-uncompressed-blocks-20261110
- [x] Follow the third classic ext block-pointer level, preserving the full logical span of sparse subtrees and refusing unavailable mapping blocks.
      — claimed: Afloat16, branch feature/r3.12-ext-triple-indirect-20261110
- [x] Preserve ext device, socket and FIFO inode kinds without exposing their block fields as regular-file contents.
      — claimed: Afloat16, branch bugfix/r3.12-ext-special-inode-kinds-20261111
- [x] Use ordered visited-node membership in ROMFS, ext, SquashFS and JFFS2 directory walks so wide filesystem trees avoid repeated scans of every previously visited node.
      — claimed: Afloat16, branch bugfix/r3.12-filesystem-page-visits-20261110
- [x] Read JFFS2 node, inode and directory integers in the filesystem byte order, including images with erased leading space, and disclose mixed byte orders.
      — claimed: Afloat16, branch feature/r3.12-jffs2-big-endian-20261110
- [x] Read ROMFS names at the configured name-length limit.
      — claimed: Plastic-time, branch rrfz/bugfix-r3.12-romfs-name-20261110
- [x] Read regular ext4 inline files up to the 60-byte inode payload while preserving byte budgets; unsupported inline continuations and directory layouts remain explicitly unrecovered.
      — claimed: Afloat16, branch feature/r3.12-ext4-small-inline-files-20261110
- [x] Read big-endian cramfs inode fields and block pointers so files and links recover like little-endian images.
      — claimed: Afloat16, branch bugfix/r3.12-cramfs-big-endian-20261110
- [x] Verify gzip FHCRC and accept informational XFL and OS bytes without inventing reserved-value restrictions.
      — claimed: Afloat16, branch bugfix/r3.12-gzip-header-crc-20261109
      Source [`dde4aa02`](https://github.com/Afloat16/craft-anything/commit/dde4aa0233be968f9b55bd3316204b880b8dbf2e) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226891865)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.
- [x] Match firmware ZIP end records to the same archive start so stored nested archives do not shorten the outer region.
      — claimed: Afloat16, branch bugfix/r3.12-firmware-zip-extent-20261110
- [x] Exercise signed PAX lengths, empty keys and equals signs inside a path value.
      — claimed: Afloat16, branch bugfix/r3.12-pax-record-framing
- [x] Reject invalid local PAX sizes before listing the affected TAR member.
      — claimed: Plastic-time, branch bugfix/r3.12-tar-local-pax-size-1112
- [x] Apply global PAX archive attributes to later members, honor local overrides and global deletions, and bound inherited name copies before extraction.
- [x] Pass recovered filesystem file bytes into firmware extraction without retaining a second full copy.
      — claimed: Afloat16, branch refactor/r3.12-firmware-owned-payloads-20261110
- [x] Bound and cancel CRC cpio verification across firmware probes, and reject non-hex checksum
      fields — claimed: Afloat16, branch bugfix/r3.12-bound-cpio-crc-scans-20261103
- [x] Distinguish short block-backed ext symlinks from inline symlinks using inode allocation metadata.
      — claimed: Afloat16, branch bugfix/r3.12-ext-block-backed-short-symlinks-20261111
- [x] More parsers keep to the bytes they declare and refuse impossible values: signed LEB128 past
      64 bits, damaged modified UTF-8 (valid text around it survives), widened Java instructions
      and invocation operands, DEX register lists and array widths, SQLite text in the database's
      encoding with cells kept inside their page and overflow cycles stopped, Android string pools,
      ZIP output limits for stored entries, decoded sizes and ZIP64 fields, ASAR header framing,
      ROMFS dot roots, cramfs, ext and JFFS2 sizes and holes, PyInstaller modules outside their
      PYZ, JSON config edits (comments kept, every duplicate key removed), and UDP, TCP, DNS over
      TCP, pcapng and TLS lengths (#688–#715, by Afloat16)
- [x] Check ClientHello vector lengths and extension values before reporting the handshake as complete.
      — claimed: Afloat16, branch bugfix/r3.12-tls-client-hello-values
- [x] Report SQLite row counts and reads as incomplete when b-tree children are null, cyclic or reused.
      — claimed: Afloat16, branch bugfix/r3.12-sqlite-tree-completeness-20261110
- [x] SQLite tables whose quoted name holds a parenthesis (`"export (1)"`) keep their columns,
      rowid alias and WITHOUT ROWID order: the column list opens at the first parenthesis outside
      quotes.
      — claimed: Plastic-time, branch bugfix/r3.12-sqlite-parenthesized-table-names
- [x] Use ordered page membership for SQLite row and count walks so a wide b-tree does not repeatedly scan every previously visited page.
      — claimed: Afloat16, branch bugfix/r3.12-sqlite-page-visits-20261110
- [x] A comment after a SQLite column list that holds `)` (`... WITHOUT ROWID /* ) */`) still hides
      the WITHOUT ROWID marker: find the list's closing parenthesis by tokens, not `rfind(')')`.
      — claimed: Chr0mium24, branch bugfix/r3.12-sqlite-options-after-comment

- [x] Keep Android binary XML events within their document, chunk and attribute bounds.
      — claimed: Afloat16, branch bugfix/r3.12-android-xml-events
- [x] Reject Java Long and Double constants without their reserved constant-pool slot, including the maximum u16 pool boundary.
      — claimed: Afloat16, branch bugfix/r3.12-java-wide-pool-slot-20261109
      Source `e0dbc3a5b60b`; [full Linux gate](https://github.com/Afloat16/craft-anything/actions/runs/42629757277) passed.
- [x] Borrow Java attribute names from the constant pool so repeated references cannot multiply temporary parser memory.
      — claimed: Afloat16, branch bugfix/r3.12-java-attribute-name-borrowing-20261110
- [x] Unfold JAR manifest attributes as bytes before UTF-8 decoding, preserve value whitespace, and recognize every specified line ending so entry digests use the original filenames.
      — claimed: Afloat16, branch bugfix/r3.12-jar-attribute-bytes-20261110
- [x] Merge digest attributes across repeated JAR manifest sections for the same entry, honoring later attribute values while retaining absent earlier values and checking each file once.
      — claimed: Afloat16, branch bugfix/r3.12-jar-repeated-entry-sections-20261110
- [x] Connect class literals, method types, method handles and dynamic bootstrap arguments to their actual load or call instruction, with a shared linkage-work limit.
      — claimed: Afloat16, branch feature/r3.12-jvm-bootstrap-references
- [x] Find the method parameter terminator outside object type names in JVM descriptors.
      — claimed: Afloat16, branch bugfix/r3.12-java-descriptor-boundary
- [x] Validate complete JVM switch payloads before reserving pairs, and reject duplicate or descending lookup keys instead of emitting invalid control-flow facts.
      — claimed: Afloat16, branch bugfix/r3.12-java-switch-tables-20261110
- [x] Stop JVM bytecode decoding on invalid primitive array type codes or zero-dimensional multidimensional allocations while retaining valid allocations.
      — claimed: Afloat16, branch bugfix/r3.12-java-array-operands-20261110
- [x] Bound aggregate DEX payload copies while attaching shared switch and array data to decoded instructions.
      — claimed: Afloat16, branch bugfix/r3.12-dex-payload-memory-20261110
- [x] Stop capped DEX member and handler lists from interpreting unread records as the following field or a catch-all.
      — claimed: Afloat16, branch bugfix/r3.12-dex-capped-table-alignment-20261109
      Source `149f060b9959`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42633003467/job/129213811327) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642583301/job/129247271153) passed.
- [x] Address the combined parser review: preserve partial Java reference coverage,
      enforce DEX signed 32-bit LEB128 limits, recover legacy AAPT UTF-8 lengths within
      their pool with a shared recovery probe limit, and distinguish missing ext indirect
      metadata from sparse holes.
      — claimed: Afloat16, branch bugfix/r3.12-parser-review-followups-main-20261104
      Shared-budget revision `29c48e03` passed [full Linux CI](https://github.com/storytold/craft-anything/actions/runs/41762780268).
      The merge with main `41d2175e` awaits a new CI run.

- [x] Recognize HTTP extension method tokens in packet captures, including WebDAV requests.
      — claimed: Plastic-time, branch bugfix/r3.12-http-extension-methods-1112
- [x] Expose SVCB/HTTPS target and service parameters, SSHFP/TLSA authentication data, and NAPTR/URI/DNAME discovery values through bounded DNS answer decoding.
      — claimed: Afloat16, branch feature/r3.12-dns-service-bindings
- [x] Keep DNS answer names inside their declared RDATA, including compressed names and MX
      exchanges, and report malformed records as incomplete instead of inventing answer values,
      while preserving legal empty DNS UPDATE RRset prerequisites
      — claimed: Afloat16, branch bugfix/r3.12-dns-rdata-names-review-20261104
- [x] Keep address and service records attached to their owner and validate the record's wire fields.
      — claimed: Afloat16, branch bugfix/r3.12-dns-record-evidence
- [x] Resolve keyed archive references consistently in nested objects and object rows.
      — claimed: Afloat16, branch bugfix/r3.12-keyed-archive-references
- [x] Recognize UTF-16 XML keyed-archive NIBs after either byte-order mark so their objects, classes and references reach the existing plist decoder.
      — claimed: Afloat16, branch bugfix/r3.12-nib-utf16-detection-20261111
- [x] Distinguish the keyed archive's null sentinel at object zero from ordinary stored strings containing the same text.
      — claimed: Afloat16, branch bugfix/r3.12-keyed-archive-null-string-20261110
- [x] Reject NIB variable-length integers whose tenth byte exceeds the unsigned 64-bit range, instead of letting truncated high bits become an unrelated length or index.
      — claimed: Afloat16, branch bugfix/r3.12-nib-integer-overflow-20261110
- [x] Accept binary plist arrays and sets that name one stored value many times; repeated data and
      text spend a byte budget, and containers past the node cap are refused before reserving space.
      — claimed: Plastic-time, branch bugfix/r3.12-plist-shared-array-values
- [x] Bound NIB object copies by their retained data, key and class bytes so shared value ranges cannot amplify a small archive beyond the byte budget.
      — claimed: Afloat16, branch bugfix/r3.12-nib-retained-bytes-20261110
- [x] Separate SQLite column names from their types at carriage returns and form feeds.
      — claimed: Plastic-time, branch bugfix/r3.12-sqlite-column-whitespace-1112
- [x] SQLite column names read whole when quoted: a doubled quote inside one stands for a quote,
      and single-quoted names (which SQLite accepts) lose their quotes. Keys that name such columns
      find them, so rowid aliases and WITHOUT ROWID order hold.
      — claimed: Plastic-time, branch bugfix/r3.12-sqlite-quoted-column-names
- [x] Removing the last member of a JSONC object takes the comma before it too, so a config that
      is plain JSON (Claude Code's, Claude Desktop's) stays plain JSON after an uninstall; an
      object written with trailing commas keeps them, new members included.
      — claimed: Plastic-time, branch bugfix/r3.12-jsonc-no-new-trailing-comma
- [x] Preserve CRLF when creating the first JSONC object in a whitespace-only or comment-only file.
      — claimed: Plastic-time, branch bugfix/r3.12-jsonc-first-object-eol-1111
- [x] SQLite rows keep each value on its own column when a table has VIRTUAL generated columns:
      SQLite computes those when a row is read and stores no field for them, so they show as not
      read (with a note) instead of shifting later values onto the wrong columns, in rowid and
      WITHOUT ROWID tables alike.
      — claimed: Chr0mium24, branch bugfix/r3.12-sqlite-virtual-generated-columns
- [x] A SQLite CREATE TABLE longer than 4 KiB is read whole. The schema walk cut it at the 4 KiB
      text cap, so the columns after the cut weren't listed, a rowid alias read NULL, a WITHOUT
      ROWID table showed no rows, and the defaults of columns added later weren't filled.
      — claimed: Chr0mium24, branch bugfix/r3.12-sqlite-long-create-table
- [x] Comments in a SQLite CREATE TABLE stay out of its columns: a comma or quote in a comment
      doesn't split or swallow definitions, a comment before or right after a name isn't part of
      it, and a comment inside a definition isn't part of its type, so INTEGER PRIMARY KEY still
      marks the rowid alias.
      — claimed: Plastic-time, branch bugfix/r3.12-sqlite-create-table-comments
- [x] Keep the comma inside a SQLite type's parentheses: `DECIMAL(10, 2)` is listed as
      `DECIMAL(10 2)`. The type keeps the comma inside its parentheses, and the schema overview
      shows it unchanged. — claimed: AlexJoaquimPereira, branch bugfix/r3.12-sqlite-type-comma
- [x] Keep shortened IL2CPP literals within their UTF-8 output budget without inventing a partial character.
      — claimed: Afloat16, branch bugfix/r3.12-il2cpp-literal-byte-budget-20261109
      Source `d81b7d570546`; [format checks](https://github.com/Afloat16/claude-mem/actions/runs/42634877637/job/129220281761) and [compatibility gates](https://github.com/Afloat16/claude-mem/actions/runs/42642583301/job/129247271153) passed.
- [x] Check that IL2CPP method ranges agree with each method's declaring type before reporting names and ownership.
      — claimed: Afloat16, branch bugfix/r3.12-il2cpp-method-owners
- [x] Asset catalogs name each rendition key attribute as CoreUI reads it. When the deployment
      target predates an attribute (display gamut before macOS 10.12), actool writes a stand-in
      (the placeholder attribute) in KEYFORMAT and the real one in KEYFORMATWORKAROUND, which
      isn't read: the key format lists `placeholder` and a Display P3 variant's key shows
      `placeholder: 1` instead of `display-gamut: 1`.
      — claimed: Chr0mium24, branch bugfix/r3.12-car-key-format-workaround
- [x] Swift optionals of function types keep their parentheses. The demangler printed the type
      inside `?` or `!` as is, so an optional closure read as a closure that returns an optional:
      `WeatherKit.WDSEndpoint.portOverrideProvider` read `() -> Swift.Int?` where `swift-demangle`
      prints `(() -> Swift.Int)?`. An optional protocol composition read `main.P & main.Q?` for
      `(main.P & main.Q)?`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-optional-functions
- [x] Swift special types decode. A type written with `X` and a kind letter (a non-escaping
      closure `XE`, an `AnyObject` or class-and-protocol existential, a protocol metatype, a
      `@convention` or `@autoclosure` function, `Self`, a `weak` or `unowned` type) was reported
      invalid ("unsupported special type"): `_$s13IDEConsoleKit0A5ThemeC22infoTypeThumbnailImageSo7NSImageCvgAFyXEfU_`
      is `closure #1 () -> __C.NSImage in IDEConsoleKit.IDEConsoleTheme.infoTypeThumbnailImage.getter : __C.NSImage`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-special-types
- [x] Python 3.5.0, 3.5.1 and 3.5.2 wrote magic number 3350 into their `.pyc` files, and 3.5.3
      moved to 3351. A file with 3350 is reported as a 3.5 pre-release (`is_release: false`), with
      a note that its code layout may differ. Count 3350 and 3351 as releases of 3.5.
      — claimed: Chr0mium24, branch bugfix/r3.12-pyc-3-5-release-magic
- [x] Report undefined Python bytecode header flag bits while preserving the code inventory and the defined invalidation mode.
      — claimed: Afloat16, branch bugfix/r3.12-pyc-flag-validation-20261110
- [x] Wrap extracted PYZ code with the header length required by its Python magic number;
      preserve the marshal payload and apply the same version-specific size to extraction limits.
      — claimed: Afloat16, branch bugfix/r3.12-pyz-version-headers-20261110
- [x] Swift addressors decode. After `a` (mutable) or `l`, an addressor has a letter for its kind
      (`u` for unsafe), but the demangler read only the `a` and took the kind letter for another
      operator, so every addressor was reported invalid: `_$s5Books19PAGETURNLOG_ENABLEDSbvau` is
      `Books.PAGETURNLOG_ENABLED.unsafeMutableAddressor : Swift.Bool`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-addressors
- [x] Swift outlined variables decode. `Tv` with an optional index names the outlined variable of the
      symbol before it (a function, a protocol witness, or a global), but the demangler reported every
      one as an unsupported thunk, so `_$s013CompilerSwiftA21PluginMessageHandling0C7FeatureO8rawValueACSgSS_tcfCTv_`
      is `outlined variable #0 of CompilerSwiftCompilerPluginMessageHandling.PluginFeature.init(rawValue: Swift.String) -> CompilerSwiftCompilerPluginMessageHandling.PluginFeature?`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-outlined-variable
- [x] Swift async thunks decode. `Tu` is the async function pointer of the function before it, and
      `TY` and `TQ` with an index are its suspend and await resume functions, but the demangler
      reported all three as an unsupported thunk, so `_$s10XCTestCore5QueueC11synchronizeyyYaFTY0_`
      is `(1) suspend resume partial function for XCTestCore.Queue.synchronize() async -> ()`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-thunks
- [x] Swift subscript accessors print the accessor after `subscript` and the type after a colon,
      as `swift-demangle` does. The demangler put the accessor last, so
      `_$s10Foundation4DataV06InlineB0Vys5UInt8VSiciM` read
      `Foundation.Data.InlineData.subscript(Swift.Int) -> Swift.UInt8.modify` where `swift-demangle`
      prints `Foundation.Data.InlineData.subscript.modify : (Swift.Int) -> Swift.UInt8`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-subscript-accessors
- [x] Swift key path accessors decode. The getter and setter of a key path (`TK` and `Tk`) after a
      property descriptor and the type the path starts from were reported as an unsupported thunk, so
      `_$s10DVTIconKit18GeneratedIconImageC14drawingContextAA0cd7DrawingG0CSgvpACTk` is
      `key path setter for DVTIconKit.GeneratedIconImage.drawingContext : DVTIconKit.GeneratedIconDrawingContext? : DVTIconKit.GeneratedIconImage`
      in `swift-demangle`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-keypath-thunks
- [x] Swift reflection, metadata-cache and Objective-C class symbols decode. The demangler stopped
      at `M` followed by `K`, `D`, `F`, `A`, `P`, `B`, `U`, `t` or `s` and reported the symbol
      invalid ("unsupported metadata kind"); about 206,000 symbols in macOS 15.7 and Xcode 26 system
      binaries were affected. `_$s7Testing3TagVSeAAMcMK` is `metadata instantiation cache for
      protocol conformance descriptor for Testing.Tag : Swift.Decodable in Testing`.
      — claimed: Chr0mium24, branch bugfix/r3.12-swift-demangle-metadata-kinds
- [x] Name the glyph weight (26) and glyph size (27) key attributes of asset catalogs, which hold
      SF Symbol style images; they're listed as `attribute-26` and `attribute-27` today.
      — claimed: Chr0mium24, branch bugfix/r3.12-car-glyph-attributes
- [x] Reject malformed ASAR directory, link and file nodes instead of listing fabricated empty files.
      — claimed: Afloat16, branch bugfix/r3.12-asar-node-shapes-20261109
      Source [`351002ea`](https://github.com/Afloat16/craft-anything/commit/351002eab7e69791a9eb6cd3764c35e1c3cbc130) passed [formats tests, fmt and clippy](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226891843)
      and [layers, locks, secret scanning, wasm checks and Cargo.lock stability](https://github.com/Afloat16/claude-mem/actions/runs/42640688213/job/129240669663).
      The [unchanged-code baseline](https://github.com/Afloat16/claude-mem/actions/runs/42636776430/job/129226895582) reproduces the new regression cases.

- [x] Reject duplicate GGUF metadata keys and tensor names before they overwrite metadata or inflate parameter totals.
      — claimed: Afloat16, branch bugfix/r3.12-gguf-duplicate-identities-20261111
- [x] Calculate GGUF quantized tensor byte ranges from their block layout and diagnose invalid row widths.
      — claimed: Afloat16, branch bugfix/r3.12-gguf-quantized-extents-20261110

- [x] Require complete sBPF symbol records and their declared string-table links before reporting symbol identities.
      — claimed: Afloat16, branch bugfix/r3.12-sbpf-symbol-framing-20261110
- [x] Keep the sBPF symbol and relocation output limits global across repeated ELF tables.
      — claimed: Afloat16, branch bugfix/r3.12-sbpf-table-records
- [x] Bound retained PE import/export name and forwarder bytes across repeated references; preserve bounded prefixes and report byte-budget truncation.
- [x] Refuse padded and unterminated subidentifiers and cut-off arc lists in code-signature requirement OIDs instead of showing a different OID, and split a long first subidentifier.
      — claimed: NicolasBohn, branch bugfix/r3.12-codesign-oid-subidentifiers


### R3.13 Web inspection correctness
Owner: open · Status: in progress
- [x] Decode character references in HTML page titles before returning their displayed text.
      — claimed: Plastic-time, branch rrfz/bugfix-r3.13-html-title-entities
- [x] Report received HAR response body sizes separately from decoded content sizes and identify entries whose sizes are unknown.
      — claimed: Afloat16, branch bugfix/r3.13-har-transfer-size
- [x] Read HAR captures with the permitted UTF-8 byte order mark.
      — claimed: Afloat16, branch bugfix/r3.13-har-bom-bounds-20261107
- [x] The HTML scanner finds script preloads in `rel` token lists, decodes asset URLs once (numeric
      and named references), keeps script and title text through lookalike closing tags, and skips
      raw-text elements, markup inside attribute values and custom tag names such as
      `<script:widget>` (#591–#596, by Afloat16)
- [x] HAR reports redact credentials whose query names are percent-encoded (and the stand-in
      backend matches older recordings the same way), redact declared secrets in every kept field
      (names, methods, hosts, versions, timings), count body bytes before text conversion, and
      template only real calendar dates in API paths (#602–#605, by Afloat16)
- [x] Preserve empty query pairs when matching older stand-in recordings, without treating raw
      query separators as pairs — claimed: Afloat16, branch `bugfix/r3.13-preserve-empty-query-pairs-20261103`

---

## M4 Release

### R4.1 A macOS CLI that opens
Owner: maintainers · Status: todo
- [ ] **decision** A standalone macOS CLI that can carry its ticket, such as a stapled `.pkg`; that
      needs a Developer ID Installer certificate in the release secrets

### R4.2 Managed tier
Owner: maintainers · Status: todo
- [ ] **decision** Managed tier: account backend, usage metering (Claude API under our own key only)
- [ ] Before the managed tier or any paid offering: confirm the Claude Code orchestration question
      with Anthropic (claude-code.md)

---

## Parked

Looked at and not committed to; none of this is open work. Say in the issue if you want one.

- From the tool survey: firmware image kinds (ESP32, U-Boot, device trees, Intel HEX), Erlang BEAM,
  GPU code objects, kernel modules, GameCube DOL, Mac crash reports, SAZ and qlog captures, protobuf
  bodies, R8 mappings, JNI links, MCP over HTTP
- Ideas from issues: resume by itself after a usage limit (#476, the one tied to a real problem), a
  source folder as a target (#480), actions after a build ends (#495), WebAssembly disassembly
  (#175), an HTTPS intercepting proxy (#164), VM and Windows Sandbox detonation (#144, #163), an
  adapter for another disassembler (#147)
- More providers and agents (#14, #63, #97, #162, #170, #186, #206, #470): R2.2's table comes first,
  so each new one is a small addition rather than another half-wired agent
