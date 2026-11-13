# Pitfalls

Mistakes made while building this project, what they cost, and the rule that prevents each one.
Read this before your first change; add to it when you make a new mistake worth not repeating.
Keep entries short: what happened, then the rule.

## Git and pull requests

**Work began before its roadmap claim, on a branch without the required prefix.** Renaming the
branch and recording the claim during PR preparation fixes coordination going forward, not the
missed order. Create and publish the claim on a correctly named branch before implementation;
never describe a late claim as compliance with the claim-before-work rule.

**A commit landed on `main` without a PR.** `git switch main && git pull` was run with uncommitted
changes: the switch succeeded, the pull refused, and the failure was read as "still on my branch".
The next commit and `git push --force-with-lease` went to `main`.
- Check `git branch --show-current` before every commit and push.
- Push by name (`git push origin <branch>`), never a bare `git push` after switching around.
- Commit or stash before switching branches; read every line of a failed compound command.
- Maintainers: protect `main` (PRs required, squash only) so this can't happen.

**A PR merged with only its claim.** One shell command did commit, push, `gh pr create` and
`gh pr merge`; the pre-commit hook failed (it ran `cargo xtask secrets` on an older default Rust),
the `&&` chain stopped, but the next lines still ran, so #138 merged the claim commit under the
work's title. Run commit, push, create and merge as separate steps and read each result; run the
hook with the project's toolchain (`RUSTUP_TOOLCHAIN=1.99.0 git commit …`).

**A merge of commits no check ran on.** A script waited for one CI run of a PR, then merged
whatever the PR's head was by then; its author had pushed two more commits meanwhile, so #141
reached `main` with code neither CI nor review had seen. Merge with `--match-head-commit` set to
the commit the green run tested (`gh run view <id> --json headSha`), never to a head read later.

**Squash messages that lost their co-authors.** A batch of replays was merged from a script that
kept each message on one line and turned `\n` back into line breaks. A shell heredoc had collapsed
that step into one that did nothing, and `printf` broke other lines early, so a dozen squash
commits reached `main` with `\n\nCo-authored-by:` as plain text or with the line cut off, and
GitHub credited nobody. Write a squash message to a file with real line breaks, and read the first
commit a new merge script makes (`git log -1 --format=%B`) before it does a second.

**A squash commit names the wrong PR number.** The subject was written as "(#12)" before the PR
existed; it became #11. Take the number from `gh pr create`'s output (`${URL##*/}`) and use it.

**Two PRs pass alone and break `main` together.** One tightened `cargo xtask layers`, the other
added an app the new rule rejected; both were green on their own branch.
- Before merging a PR that was rebased before another merge, build and test the merged result:
  `git merge-tree --write-tree origin/main origin/<branch>` for conflicts, then a temporary branch
  with the merge, `cargo test --workspace`, clippy and `cargo xtask layers`.
- After merging several PRs in a row, run the full checks on `main` once more.
- It happened again with translations: #117 added UI strings and #115, merged right after it,
  added a check that every string has a Chinese translation, so `cargo xtask ci` failed on
  `main`. A PR that adds a check, or adds what a check covers, is tested on top of the other.
  And again with a new catalog: #555 added "Pausing…" to every catalog there was, and #486 (Russian)
  merged after it without it. A new language and a new UI string merged together need the string
  in the new catalog too.
  And with a layout change: #687 moved coding agents onto their own tab of what Detect found, and
  #664, merged next, tested a button on that list without picking the tab. Checking that two PRs
  merge cleanly isn't checking that their tests pass together: run the UI tests on the merge.

**A table row kept twice by a merge.** Two PRs edited the same STATUS.md row; resolving the
conflict kept both versions, so five rows reached `main` twice, each copy with a different
update, and the three OpenCode rows disagreed on whether SSH runs get the tools. When a
conflict is in a table row, merge the edits into one row, and after merging look for a row
name that appears twice.

**A dependency change merged without its lockfile.** Changes were staged with
`git add -A crates apps docs`, which leaves out the root `Cargo.lock`, so a new dependency reached
`main` without its lock entry. Stage by `git add -A` (then review `git status`), or list `Cargo.lock`
whenever a `Cargo.toml` changed.

**A superseded PR on someone else's branch.** Rebasing it means force-pushing their branch. Replay it
on a new branch, open a new PR that says it supersedes the old one, and comment on the old one.
Don't close or rewrite other people's work without asking.

**Review fixes pushed to a PR that had already merged.** #686 was merged hours before its review
comments were acted on; the fixes went to its branch and the threads were answered "Fixed", while
`main` had the code without them. Read `gh pr view --json state,headRefOid` before pushing to a PR's
branch, and land anything late in a new PR from `main`.
It happened again with the check in place: `gh pr view 1022 --json state -q .state && … git push`
printed `MERGED` and went on, because printing a state isn't failing. The check has to stop the
command: `test "$(gh pr view <n> --json state -q .state)" = OPEN && …`.

**A report checked against main, not the release it ran.** #736 (a build looping on an empty `app/`)
was read against main, where the stop after two unchanged turns already existed, so it looked fixed.
The reporter ran v0.3.0, which had neither that stop (#557) nor the empty-folder note (#749); both
merged the day after it shipped. Before calling a report fixed or still open, find the release it
names (`git merge-base --is-ancestor <fix> <release tag>`), and check the message text it quotes.

## Tests

**A heartbeat sampled before shutdown raced a successful stop.** A child could write between
the baseline read and `stop()`, failing the descendant cleanup test even when cleanup worked.
Probe for exit instead of timing writes: the child holds a file lock the system releases only
when it exits, and the test takes it after the stop. Ignored child fixtures also need
an explicit launch guard so running ignored tests directly cannot write in the working directory.
The replacement's quoted `cmd /C` script still failed on Windows, and startup assertions skipped
cleanup. Use a guarded Rust parent instead of shell syntax, and stop before asserting on startup.
Its child must inherit the watched group/job: another `Process` or `Launch` creates a separate
Unix group and would no longer test the ancestor's shutdown.

**An isolated PATH still assumed system utilities lived in `/bin`.** The remote setup fixture
linked `mkdir`, `chmod` and `mv` there, so a host with utilities elsewhere got dangling links.
Resolve fixture utilities on the host, make their paths absolute, then populate the private PATH.

**Python compilation reused stale bytecode after a fast edit.** The new-project toolchain probe
changed valid Python to a syntax error within the same mtime second. `compileall` skipped it and
exited zero; the screenshot run then failed instead. Use `compileall -f` for a build check, and
hold the valid and broken fixture revisions at the same timestamp in the real-Python regression.

**A test started a real download on a contributor's machine.** A test called `engines_install` for
Unity; a later change made that download the vendor installer and open it on Windows. It also
created folders in the real config directory (`Session::default()` uses it).
- Tests never touch the network, never start installers or GUI programs, and never write outside a
  temp folder. Use ids that are refused before any I/O, or local stand-in servers.
- When you widen what a tool does (a new download, a launch), grep the tests that call it.
- Tests build their session with `craft_anything_engine::testing::session()` (config, plugin
  caches, engine store and projects in a temp folder); `cargo xtask layers` rejects
  `Session::default()`. CLI tests set `CRAFT_ANYTHING_CONFIG_DIR` to a temp folder.

**A demo app wired to the person's settings.** The headless app the UI tests use built its AI hub
with `AiHub::new`, which saves to the real `config.toml` and the system keychain; no test had
added or removed an AI yet, so nothing noticed. Everything a test app can save goes to a temp
folder (`AiHub::with_stores`), not only the engine's session.

**A test that passes only where a program is installed.** A tool's `Requires::Engine` is checked
before it runs, so a test of its input checks passed on a machine with ilspycmd and failed on one
without. Point the test session's `engines.tools` path at a stand-in file when the tool refuses
before running anything, and run new tests on a machine without the program.
It happened again when Clean room began refusing tools before their program is looked for: a
"missing adb" test returned early on a machine with adb and failed in CI without it. A test of
what happens without a program builds its session in the mode the tool runs in.
It happened a third time with Ghidra: the test session's engine store was empty, but the lookup
still searched PATH, `~/.local/bin` and `/opt/ghidra`, so two Deep-mode tests got Ghidra's answer
on a machine that had it. `testing::session()` now sets `engines.only_configured`, so it finds only
the engines its settings or store name.

**A failing crate hid the next crate's tests.** `cargo test -p a -p b` stopped after a failure in
`a`, so `b`'s tests never ran; the rerun was filtered to the failing test, and a broken engine id
reached `main`. Run with `--no-fail-fast`, and after a fix rerun the whole crates, not a filter.

**A stopped server's port was no longer its identity.** A shutdown test connected to the old
port and treated any listener there as the stopped server, although another fixture could reuse it.
Observe the actual tool's registered server and a weak reference to its shared answerer; cancellation
must release that resource and empty its registry. Do not reserve and release a port before startup.

**A test that changes the whole process.** A UI test raised the process-wide stuck-lock banner; in
CI the banner moved the layout under another test running beside it, and its click missed. It passed
25 runs on a Mac. Anything a test sets for the whole process goes in a test program of its own.

**A free port handed to another test.** For a negative TCP probe, a bound port stopped being
ours as soon as its listener was dropped. Another fixture could acquire it before the probe.
Use the unassigned loopback port zero to exercise a real connection refusal without borrowing
and releasing a shared ephemeral port.

**Fixtures that leak into each other.** A "nothing here" path sat beside a fake game folder in the
same temp directory, and game detection walked up and found the game. Give each case its own
`tempfile::tempdir()`.

**Tests that share a process-wide value.** The start guard keeps its mark's lock in a `static`;
one test's `opened()` emptied it while another test was between `begin` and its check, so
`main` failed now and then although the PR's own run had passed. Tests that touch the same
`static` take a shared test lock, even when each uses its own temp folder.

**A mock server closed on a partial request.** A loopback fixture read a request only once, then
replied and closed. TCP can split even a short header; unread bytes made that close reset the
connection. Consume the complete header before replying, with a size cap and an absolute deadline.
Negative probes use loopback port zero so another fixture cannot acquire a released port.
A controlled fragmented request reproduced the fixture defect; the old CI log did not preserve
the probe's underlying error, so it does not establish that failure's cause.

**A socket option set after the peer reset it.** A preview test set its clients' read timeout
after stopping the server. macOS answers `setsockopt` with `Invalid argument` on a socket closed in
both directions (a reset, or the peer's FIN after our own `shutdown(Write)`); Linux doesn't check,
so the test failed only on Macs. Set socket options right after connecting, before anything can
close the connection.

**A script written and run at once.** Tests that write a stand-in program and start it failed now
and then on Linux with "Text file busy": another test thread was starting a child at that moment,
and the child still held the new file open for writing. Starting a program goes through
`process::spawn_retrying`, which tries a busy file again for a moment; don't add a sleep to a test
instead.

**Proving a refactor changed nothing.** Unit tests that check substrings can miss changed prompts.
For a refactor that must keep output identical, dump the outputs for every combination before and
after and compare them byte for byte (done for every brief across 162 combinations in R1.1).

**A guard phrase that matched the wording it protects.** A test that no blind builder is told to
"look at the picture" failed on the new "you can't look at the pictures in reference/". Make a
forbidden phrase the instruction's exact words, check each one still appears in what the other
case is sent (or the guard goes stale without failing), and break the code once to see it fail.

**A Windows-only test that no check ran.** #748 added a second Antigravity folder to the search
path; the Windows test that counts those folders wasn't updated, and `main` stayed green because CI
runs on Ubuntu, where `cfg(windows)` tests don't compile. When a change touches a list a
`cfg(windows)` (or macOS) test checks, grep for it and run it on that system, or say in the PR that
you couldn't.

**A step moved to a worker, its test still counting frames.** #720 moved the picture size check
off the UI thread; a UI test that added pictures, ran three frames and then looked kept passing on
fast machines and failed in CI on unrelated PRs. When work moves to a worker, grep the tests that
call it and make them `wait` for the result, not `run_steps(n)`.

## Building and running

**A documentation edit rebuilt dependencies in a separate cache.** A sequential review follow-up
used another target directory for its commit hook, triggering an unnecessary build. Separate
build caches are for concurrent work; reuse the current checkout's cache for sequential work.

**A runner's PID isn't the window owner's.** `go run` starts a compiled child, so picturing the
runner missed the app (or a name fallback found another app). Search only the launched process
and its descendants, and read the UI tree from the same PID that supplied the picture.

**Runtime capture passed relative paths to a tool that needs absolute ones.** With a relative
project placement, the selected virtualenv interpreter and artifact paths stayed relative;
PATH lookup can return a relative path too. Resolve workspace paths against the process's
current directory, look up only bare program names, and make the result absolute without
dereferencing the virtualenv symlink. Test the generated capture request without opening a GUI.
The same rule covers every child started in app/: engine build arguments, option files, console
run checks, Play and pins must get absolute program, working-directory and path arguments; a
relative one is resolved against app/ again.

**A screenshot helper aborted inside CoreGraphics.** The first macOS screenshot probe used
ScreenCaptureKit from a command-line Swift helper without initializing AppKit's WindowServer
connection. An owned-window capture aborted in `CGS_REQUIRE_INIT`, while no-window regressions
passed. Initialize `NSApplication.shared` on the main actor before capture, and verify an actual
owned-window PNG as well as missing-window errors.

**A model catalogue queried in the wrong project.** OpenCode V2's raw `api get /api/model` does
not reliably select the invoking CLI's directory. Use the `model.list` operation with an explicit
`location[directory]` parameter, and test that the returned Location matches it. Over SSH,
expand the remote shell's working directory, never send a local project path.

**Rebuilding a binary that is running fails on Windows** ("Access is denied"), and the next command
silently used the old binary. Long-running binaries (live rebuilds, servers) get their own
`CARGO_TARGET_DIR`.

**A program started from a screen outlived the tests.** The Ghidra memory setting first asked
PowerShell for the computer's memory on a background thread while the screen drew; in UI tests
the test process ended first, the orphaned PowerShell kept the test runner's output pipe open,
and `cargo test | grep` never returned. Screens never start programs, not even "in the
background"; read a setting or a value a worker already has.

**Asking a process to stop isn't stopping it.** `RunningProcess::kill()` only raises a flag for the
supervisor thread; the window-capture tool returned and the CLI exited before the thread acted, so
the program it opened stayed open (and held the shell's pipe, so the command never returned). When
the caller may exit next, use `kill_and_wait`, and check after a live run that nothing was left
running.

**A turn's own process outlived its answer.** `run_cli` only sent `AgentEvent::TurnDone` once the
whole coding-agent subprocess exited, never once its JSON stream said the turn was done (the
`result` line). Claude Code can print that line and then linger (its own shutdown hook, logging)
for as long as the idle timeout allows — hours, for a builder turn — so a finished build stayed on
"Building"/"Pausing" long after the real work and even a final report were done, with no crash and
no error to find. Once the stream's own terminal event is parsed, stop waiting on the process
itself: fold a `result_seen` flag into `cancel_when` with a short grace period (`RESULT_GRACE`),
and don't report that as "Stopped." A terminal stream event, not the OS exit, is what ends a turn.

**A deadline that ended in a wait with none.** Every `Process` run has a time limit and a cancel,
but when either fired the supervisor called the library's `kill()`, which stops the program and
then waits for it with no limit (on Windows also for a job message the system doesn't promise).
A program that outlives its stop held the run, and Pause, for good (#1438, #981). A deadline covers
its own cleanup too: after a stop, poll for a moment and move on (`process::end`), and when a
library call says "kill", read whether it also waits. Not reproduced on the reporters' machines.

**A console held open by a program started next door.** On Windows Server 2022 opening a
pseudoconsole briefly makes inheritable handles (a Unix terminal too, until it's made close-on-exec),
so a program another thread started at that moment kept the console open until it ended, and closing
a console with programs still attached hung its host. Program starts and console or terminal opens
take `process::STARTING`; end a console's process tree before closing it.

**The newest program isn't the app.** Play started the newest executable in `target/`; a
workspace that builds an app and its CLI started the CLI, which printed help and quit with no
console, so the button did nothing, three times in a row. Pick what to start from what the project
declares (Cargo manifests: `default-run`, the package using a window library), never from file
dates, and make a start that exits at once say so.

**A launcher's exit read as the program's.** On some Windows machines `chrome.exe` hands the
browser to a second process and exits; the supervisor took that exit as the run ending and stopped
the whole group, so websites never opened ("closed after 0.3 s"). Where it didn't hand off, the
browser outlived the CLI (the job isn't killed when closed) and held the shell's output pipe, so
the command hung and old browsers piled up. Start programs that hand off with `.hands_off()`, and
after a live run check that the command returned and nothing it started is still running.

**A separate config folder hides the agent's own settings.** To keep a live run from touching the
real `config.toml`, `XDG_CONFIG_HOME` was pointed at a scratch folder; the agent CLI we start reads
the same variable, lost its own provider settings, and failed with "unsupported provider". Back up
`config.toml` and restore it after the run instead, and leave the agent's environment as it is.

**A renamed CLI can't start the builder's tools.** To keep a live run's binary from being locked, it
was copied as `cli-r25.exe`; the CLI finds itself by name (`craft-anything-cli`) to start the
builder's MCP server, so Claude Code saw no tools and stopped. Copy it under its own name into a
folder of its own.
It happened again in a release: the macOS app bundle had only `craft-anything`, with the CLI in
a separate zip, so every Claude Code build from the 0.2.0 DMG failed the same way (#91). Every
package that ships the app ships the CLI beside its executable, and the packaging script runs
the CLI from the finished package before it's published.
Its symptom misled people too: Claude Code's MCP log says `Executable not found in $PATH: "stdio"`
for any command that won't start, so #64 went looking for a bad config. Reproduce with the real
program before believing what a log's wording suggests.

**A Windows path with forward slashes read as nothing.** rtl_433 decoded a recording in the shell
but found no messages, and exited 0, when the test passed `C:/…/x.cu8`; it only reads Windows
paths with backslashes. Hand external programs native paths (`\` on Windows), and give each
external tool a live test that checks it really produced output, not just that it exited 0.
It happened again with Explorer: project folders are kept as `D:/Projects/…\name`, and "Open
folder" opened Documents. Paths for the system's programs go through `platform::native_path`.

**A message on the command line.** Claude Code was handed the whole task as `-p <message>`; a
34,641-character brief passed Windows' 32,767-character command-line limit, so the build never
started ("The filename or extension is too long", os error 206, #402). Give a program anything
that can grow (messages, briefs, lists of files) on stdin or in a file, never in argv.

**Console windows popping up over the app.** Only the detached launcher hid the console; the
supervised runner didn't, so Ghidra's `.bat` and PowerShell each opened a console window during a
real Deep-mode run (the CLI never shows it: it already has a console). Every child started from
the windowed app gets `CREATE_NO_WINDOW`; check the desktop app, not only the CLI, after adding a
program.

**A graphics backend that crashed before it was chosen.** We ranked GL last when picking an
adapter, but eframe starts every backend first, and starting GL loaded an old Intel OpenGL driver
that crashed the app before any window or error box (#196). Ranking can't protect you from a
backend that's started at all: start only the backends the app may use.
It happened again with Vulkan on Windows (#394): starting Vulkan loaded an Intel Vulkan driver
(`igvk64.dll`) that crashed with `0xc0000005` on startup. Windows now starts DX12 only by default
(which includes WARP); `WGPU_BACKEND` still allows choosing other backends.

**A shader compiler picked by a default that looks around the computer.** wgpu's DX12 `Auto`
compiler takes a `dxcompiler.dll` it finds on the PATH; one without `dxil.dll` makes unsigned
shaders, and on an RTX 5060 Ti the device was lost as the window opened ("Parent device is
lost", #522). Where a library chooses a program or DLL by searching, name the one that ships
with the system (FXC), and keep the environment variable (`WGPU_DX12_COMPILER`) as the way to
another. A window that can't start now says how to draw without the graphics card
(`CRAFT_ANYTHING_RENDERER=software`). Not run on the reporter's machine or on Windows here.

**"Not installed" for a program that was there.** The system answers "not found" both for a missing
program and for a file whose interpreter is missing (an npm script with no `node`), and we turned
every one into "isn't installed or isn't on your PATH": #95, #130 and #152 reinstalled what they
had. When a spawn says not found, look the program up yourself; if it's there, name the file and
what it needs (`spawn_error_for`). Not run on Windows here.

**An answer with two parts, read as one.** An approval card has "Allow once" and "Always for this
project": the answer carries `allow` and `always`. The consent card our MCP server shows for a tool
that runs code read only `allow`, so "Always" was dropped and the same tool (a scripted web session)
was asked about every time (#563). When a card offers a choice, whoever reads the answer handles
every field of it, and a test answers it each way and asks again.

**A loop that only ends at its step cap.** A builder on an API backend may take 400 steps in one
step of the run, and each step sends the whole conversation again: one stage spent 9.4M tokens on
code that never compiled (#399). A failed build command is an `Ok` tool result (`succeeded: false`),
so counting tool errors misses it. Cap a loop on what it is stuck on (build commands failing in a
row, read from the typed `succeeded` field), not only on its total steps.

**A context overflow that looked like a broken request.** Ollama fits a prompt that is too long
for its context by dropping the oldest messages, keeping the system prompt; the person's request
went first, and a Qwen 3.8 model refused the rest with 500 "no user query found in messages"
(#1245). It read as a fault in our request, and as a server error it was retried three times. The
error came from Ollama's source (`server/prompt.go`, `model/renderers/qwen35.go`), not from the
words: when a local server's error makes no sense for the request sent, read what the server does
to it first, and mark a refusal that can't change on retry with a typed field (`context_overflow`).

**A limit left unset is the largest one.** We sent OpenRouter no `max_tokens`, so it held the
model's whole output limit (131,072 tokens for Kimi K3.1 Pro) against the key's credit, and a key
with $35 left of its daily limit was refused with 402 before the model ran (#1213). When a service
bills or reserves by a request's limits, find out what it assumes when one is missing; when it says
what it can afford, ask again within that.

**A router that picks the provider for you.** OpenRouter sends each request to one of several
upstream providers, and a provider without tool calling answered 400 "Provider returned error"
(#369), which said nothing the person could act on. For a service that routes, say in the request
what the provider must support (`provider.require_parameters` for tools), and show the provider's
own reason when it still fails (#542).

**A refusal that names a person nobody asks.** Auto mode refused `git push` with "never …
without the person", but never showed a card, so the person's "yes, push it" in the prompt box
reached only the builder, which retried and was refused again, five times. Its rule matched
`gh repo` anywhere, so `gh repo view` was "publishing" too, and "always" on `cd app && npm i` was
kept as `bash:cd`, which would have let `cd app && git push` through. A refusal says how the
person can allow it, and a way to allow it exists; read commands word by word; remember a
dangerous kind whole, never by a command's first word.

**A crash that no code can report.** Three Windows tickets (#33, #104, #74) said the app "does
nothing": a graphics driver crashed natively as the window opened, so no dialog, panic hook or log
line ever ran, and each fix only covered the driver already seen. When a failure can't be caught,
leave a mark before the risky step and clear it after (`start_guard.rs`); a mark found at the next
start is the report, and that start takes the safer path (WARP). Not run on Windows here.

**A spinner that kept the window drawing at full speed.** egui's `Spinner` asks to be redrawn at
once, every frame, while it's on screen. A build or a page capture can run for an hour, so the
window was redrawn at the display's rate the whole time (a core busy, or every core when drawing in
software), and a person reported "my CPU is fully used" (#579). Animations ask for their next frame
at their own pace (`widgets::animate`, `widgets::Spinner`); check a library widget for
`request_repaint` before using it for something long. Not measured on Windows here.
It happened again with egui's `ProgressBar::animate(true)` in the jobs list, which the spinner fix
didn't look at: replacing one widget isn't checking the others. Search the library's widgets for
`request_repaint` (`grep -rn request_repaint ~/.cargo/registry/src/*/egui-*/src/widgets`), and the
test that refuses egui's spinner on screens refuses its progress bar too.

**Every tool offered to a model that can only use a few.** The builder on an API backend was sent
all 33-41 Core and Build tools, about 23,000-30,000 characters of descriptions. A local 14B model
(#417) called the study tools (`core_session_show`, `core_journal_read`) 35 times and never
`build_file_write`, and Ollama's default context is smaller than the list. Offer a model the tools
the job needs (`LOCAL_BUILDER_CORE`), measure the list's size with the real registry, and run a
small local model before saying it works. Not run live here.

**A debugger failure reported as an observation.** `apple_calls_observe` built a Python callback
with `\n` between statements in a Rust string, so lldb rejected it and exited 1 before launching
anything; the tool ignored the exit status and returned "Observed 0 call(s)" with confidence
observed (#251). A run that never started the thing it observes is a failure, not a count of zero:
check the exit status before building evidence, and compile generated code (here, with Python)
before trusting it. lldb itself was not run here.

**A graphics default that drew on the other card.** egui-wgpu picks the discrete GPU unless told
otherwise; on a Linux laptop whose integrated GPU drives the panel, every frame was copied across
to it, the window took 20 s to open and the whole desktop lagged (#571). Forcing the low-power GPU
instead breaks desktops whose monitors are on the discrete card. Draw on the GPU that drives the
screen, read from the connected displays, and keep wgpu's environment variables as the override;
when a window is slow to open, check which adapter `desktop.log` says it drew with.

**A Metal version taken from the OS, not from the compiler.** wgpu-hal picks the Metal shading
language from the macOS version alone; on a 2012 iMac under OpenCore Legacy Patcher, macOS 15 asked
for `metal3.2`, the compiler stopped at 3.1, every shader failed and the window never opened (#742,
upstream #10687). Our `[patch]` of wgpu-hal probes the compiler (`vendor/wgpu-hal/CRAFT_ANYTHING.md`).
The advice on a lost device named `CRAFT_ANYTHING_RENDERER`, which only Windows reads: check that
advice names only settings the platform actually reads. Not run on the reporter's Mac.

**A Mac app that couldn't find Claude Code, and offered a GitHub key instead.** An app opened from
the Dock gets the system's short `PATH`, not the shell's, so `claude` in `~/.local/bin` wasn't
found; "Built by" then fell back to "Claude Code on github.com-work", an `~/.ssh/config` alias
for a second GitHub key, read as a computer to build on. Look programs up and start them with
`which::search_path` (`Process` and `Launch` do), and leave git servers out of SSH hosts.

**Found on Windows, but "isn't on your PATH" when run.** npm installs Gemini CLI, Codex and often
Claude Code as `gemini.cmd`-style shims; `which` finds them through `PATHEXT`, but Rust starts
only `.exe` files by bare name, so every build failed (#18). Through `cmd.exe` a prompt with line
breaks or over 8191 characters wouldn't arrive anyway. `Process` runs an npm shim's script with
Node directly (`which::launch_target`); test a new program on Windows with a multi-line argument.
npm also writes an extensionless POSIX script beside each shim, and `which` returned that first,
so OpenCode was offered and then every run said it wasn't installed (#41): look up the kinds
Windows can start (`PATHEXT`) before the bare name.
And when the shim's Node couldn't be found (nvm for Windows with no version in use, so its
`C:\Program Files\nodejs` link was gone), we fell back to the `.cmd`, and Rust refused the
multi-line prompt: "batch file arguments are invalid", which named neither Node nor nvm. Search
nvm's version folders too, and when a shim's interpreter is missing, say so (`interpreter_missing`).
The desktop DeepSeek Harness has a custom batch launcher, so npm-shim handling does not unwrap
it either. Send its task on stdin, with no task argument. A literal `-` is a stdin marker in newer
Harness versions but is the task itself in older ones; omitting the task makes those older versions
fail before a model runs. They need an update to a headless profile that supports stdin.

**A first analysis that waited for a lock it could never take.** Ghidra's analysis lock is created
with `create_new` in `ghidra-cache/`, which only existed on machines that had used it before; on a
fresh store the create failed and the loop read that as "someone else holds it" until the
deadline. Create a lock's folder first, and treat any error but "already exists" as a failure.

**A bound read from an analysis that never saw the code.** Ghidra's unguarded jump tables were
first ended at the next address something refers to; a hand-made ELF passed, but in a real Rust
program the code holding the next table's `lea` is reached only through the table being bounded,
so it was never disassembled, nothing referred to the next table and the bound took all 256
entries. When a fix leans on what an analysis found, ask whether the thing being fixed is why it
found nothing, and run it on a real program, not only on one built for the test.

**A bridge that didn't compile.** A helper in the Ghidra bridge was named like a public
`GhidraScript` method, so Ghidra refused the script and the run only said the bridge stopped.
Compile the `.java` against Ghidra's jars (`javac -proc:none -cp <Ghidra/**/lib/*.jar>`) before a
live run. Ghidra also rejects project paths with a folder starting with a dot, as
`tempfile::tempdir()`'s do: give live-test temp folders a plain prefix.

**A check that read another worktree.** A PR was tested in a worktree that shared the main
`target/`; `xtask` finds the repository through `CARGO_MANIFEST_DIR`, fixed when it's built, so
afterwards `cargo xtask` in the main tree reused that binary and read the worktree's files. Once the
worktree was removed it said "can't read docs/ROADMAP.md"; while it existed, every check that
reads files itself (`pending`, `secrets`, `layers`, `locks`, `tools --check`…) would have looked
at the wrong tree (the cargo commands `ci` runs use the folder they're started in). Give every
worktree its own `CARGO_TARGET_DIR`, and rebuild `xtask` (or delete it from `target/`) after
sharing one.

**A helper stopped only by a destructor.** The keep-awake helper was stopped when its guard was
dropped; closing the desktop app during a build ended the process without dropping it, and the
PowerShell loop kept the computer awake (its time limit was a week). A helper that outlives a
crash is a leak: have it watch the process it serves and end by itself (`caffeinate -w`, a loop
that checks the PID), and test by killing the owner, not by dropping the guard.

**Stopping a long run leaves children behind** unless every program goes through
`craft_anything_platform::process`. After stopping one, check that its child processes are gone.

**Separate target folders fill the disk.** Agents given their own `CARGO_TARGET_DIR` under
`target/` (to avoid locking a running exe) left them behind; one reached 79 GB and the disk filled
mid-build ("not enough space", os error 112). Delete an agent's target folder when its work is
merged, and check `du -sh target/*` when a build fails to write.
It happened again from a commit: the pre-commit hook builds `cargo xtask secrets`, and in a new
worktree with no `CARGO_TARGET_DIR` set that is a whole fresh build in its `target/`, which filled
the disk under a person's running build. Commit with the same `CARGO_TARGET_DIR` you built with,
and check free space (`df -h`) before a build next to someone else's work.
And again from the main `target/`: its incremental cache alone had reached 46 GB (81 GB in
`target/debug`) and the drive hit 0 bytes mid-test. Delete `target/debug/incremental` when it
grows, and build checks with `CARGO_INCREMENTAL=0`.

**A green PR that ran nothing.** `gh pr view` said `CLEAN`/`MERGEABLE`, but while the repository is
private every CI job is skipped, so nothing had been checked. Read the checks' conclusions, not the
merge state. Without CI, run its steps locally with `--locked`; a Mac can compile for the other
systems with `cargo check --target`, with zig as the C compiler for `ring` (a wrapper that drops the
`--target=` flag `cc` adds), but that only proves it compiles.

**Chinese boxes mistaken for broken encoding.** A valid UTF-8 checkout can still show boxes
when a source build embeds no CJK fonts. Check the file bytes and font table before transcoding
anything. Verify glyph coverage in every text family with the package's font test and snapshot
example; Cargo preserves the native-library and LTO settings that ad-hoc linking can miss.

**A "required" switch read only on the failure path.** The build script looked at
`CRAFT_FONTS_REQUIRED` only when `CRAFT_FONTS_DIR` was set and bad; with the folder variable
missing, a build told the fonts were required finished with none embedded, and Korean showed as
boxes. A requirement is checked on every path that ends without the thing, the missing input first;
keep the decision in a function a test can call (`plan` in `crates/ui-egui/build.rs`).

**Unformatted code reached `main`.** A test was edited by script after `cargo fmt --all` had run,
so #53 merged with a long line rustfmt rewraps. Run `cargo fmt --all --check` last, after every
edit, right before committing.

**Clippy passed on an older Rust.** #1480 ran clippy on Rust 1.97 (the workspace's
`rust-version`), clean; CI's 1.99 has `clippy::chunks_exact_to_as_chunks` and stopped the PR on
it. A clippy that predates a lint can't report it. Run fmt and clippy with CI's toolchain
(`RUSTUP_TOOLCHAIN=1.99.0`), and if a tool says it isn't installed for a toolchain, install that
toolchain's component rather than running another copy.

**A process list read nothing on FreeBSD.** `ps -A -o pid=,ppid=` lists both columns on Linux,
but POSIX makes everything after `=` the header, so FreeBSD's `ps` printed one `pid` column headed
",ppid=" and every parent lookup came back empty. Give each column its own `-o` (`-o pid= -o ppid=`)
when a header is renamed or blanked.

## Design

**A source inventory limit came after expansion.** Taking five thousand names after collecting
every source path copied a large `sourceRoot` into every entry first. Count retained entries and
expanded UTF-8 bytes before allocating each path, share the budget across indexed sections and
application maps, and report when only a prefix could be kept. A source identity stays complete;
shortening one to fit the budget would describe a different source.
The source-to-bundle comparison then treated omitted paths as confirmed missing because it read
only the retained names. Carry inventory truncation through the diff and its evidence: preserve
positive matches, report the other sources as uncertain, and keep confirmed unmatched sources
for complete inventories. Test the consumer with a real exhausted budget, not only the flag.

**A recovery manifest repeated expanded source roots.** The source-map inventory budget did not
cover the recovery tool, which copied every expanded path into its manifest: a 75 KiB map with a
64 KiB root and 1,200 sources retained about 78 MiB. Apply the inventory budget to recovery paths
too, stop retaining originals when it is exhausted, and report partial recovery with a typed flag.

**A floating-point conversion changed a fingerprint.** Widening a single-precision NaN to a double
can change its sign, payload or signaling bit. Keep the original encoded bits for identities, and
test signaling/quiet pairs as well as different payloads; retain numeric conversion for display.

**A canonical-length check hid readable signature facts.** Rejecting a redundant ASN.1 length
made its certificate or signature disappear from the report. Keep a bounded, readable element,
mark its non-minimal encoding, and carry that limitation into every signature view. Test retained
metadata and the warning together; encoding conformance and cryptographic verification are separate facts.

**An unknown header bit hid readable code.** Rejecting undefined `.pyc` flag bits made the whole
file unreadable, dropping its code, names and constants from reports. A reader collecting evidence
must preserve fields and payload it can still understand: report the unknown bits as a warning,
derive known modes from their defined bits, and test that the warning and recovered data coexist.

**A round of checks that nothing could stop.** The builder's own checks ran one after another on
the build's thread with no time limit, Pause was looked at only between tool calls, and the feed
said only "checking its work against the original" for the whole round, so a check that stalled
named no check and could hold the build for hours. Give a builder's checks a round time limit,
look at Pause before each check, and say each check as its own step. The limit must reach the
checks' own program runs too: a build step (a game engine's import, a cold compile) makes no tool
call, so a limit that only applies "at the next tool call" leaves exactly the step that hangs.
A check that's cut off is a failed check with its reason, never a silent wait. And every way a
run can end — including a caught bug — records the project's status: leaving the last status on
disk ("Checking" while nothing runs) reads as a stuck build that can't be retried.

**An early error without a terminal event.** Model validation returned after reporting an error,
but headless chat waits for `TurnDone`, not just an idle worker. Preflight failures and cancellation
must finish the event sequence too; regression tests check both the error/status and the terminal event.
Worker-thread creation failures need the same terminal event even though no worker ever started.

**A conditional call erased the skipped path's value.** The ARM decoder discarded conditions on
calls, and value flow replaced the earlier register definitions as if every call executed. Keep
the call's condition and merge its possible writes with the skipped path in both block transfer
and edge construction. A known non-returning target stops only an unconditional call; after a
conditional one, only the skipped path contributes values. Test both kinds within one block and
across blocks, with unconditional calls as controls.

**A candidate cap discarded a real dependency.** The esbuild module reader kept the first 4096
resolved zero-argument calls, then filtered for known wrappers; ordinary local calls could fill
the buffer before a real dependency appeared. Discover wrapper bindings first and count only
recognised dependencies against their limit. Keep the buffer bounded, and test unrelated calls,
forward declarations and more actual dependencies than the limit permits.

**An occurrence label was also a binding name.** The second `obj.x` window and the first
`obj["x[2]"]` window both appeared as `obj.x[2]`. Reserve literal labels and use a separate
disambiguator for collisions; never change the occurrence index to make a label unique. Include
unchanged windows when reserving labels so a different set of changes cannot rename a window.

**Reading literals and skipping the rest.** The Electron window reader skipped a spread inside
`webPreferences`, so a window whose preferences came partly from another object was reported as
"contextIsolation is not set; the default applies", which nobody had checked until a live run on
an installed app. When a reader takes literal values, anything that may add or replace them
(spreads, computed keys, later writes) makes the result unknown, never the default; and check new
readers on real, minified inputs, not only on the shapes the tests were written for.

**A naming scheme that clients can't accept.** Dotted tool ids (`web.page.capture`) were decided
before checking that MCP clients and model APIs allow only `[A-Za-z0-9_-]` in tool names. Check
external constraints (protocols, APIs, file systems) before fixing a public contract.

**A tool planned in a layer that can't reach what it needs.** Rebuild control as engine tools (L4)
can't call `RebuildRun` (L5). Before planning a tool, check which crate owns what it calls; if a
higher layer owns it, it needs a way for that layer to register tools (R1.7).

**An event queue drained in the middle of a run.** A scripted browser session's `goto` step
cleared the CDP event queue to "start fresh", so every network request before and during the
navigation vanished from the run's network capture. Only the code that owns a run's results
drains its event queue; steps leave events where they are, and the run keeps them all, tagged
with the step they arrived in.

**Exclusive creation left a permission window.** Opening a private temporary file with default
permissions could let another process retain a readable descriptor before a later chmod. Set
mode 0600 in the first Unix open for private files. Temporary names must also stay short and
independent of the destination name: adding a prefix and suffix can exceed the component limit
even when the destination name is valid. Keep the temporary file in the destination directory,
but exclude the destination component before creating a candidate, including case and trailing
dot/space aliases. Probe a supported long name in tests instead of assuming a fixed filename limit.

**Two writers to one file.** The app and the rebuild run both wrote `feed.json`. Give each persisted
file one owner; when another writer is unavoidable (an answer given from the CLI while a run is
going), merge what's on disk before saving instead of overwriting it.
The same happened in `craft.json`: a run saved from the copy it started with and undid a budget
the person raised meanwhile (and any "always allow" answer). Fields the person owns change only
through `Workspace::set_person`; `save` keeps them as they are on disk.
And again inside one process: the engine had its own `write_atomic` whose temp file was named
by process only, so two threads saving `craft.json` wrote the same temp file and one rename found
it gone (#273). There is one atomic writer, `craft_anything_platform::fs::write_atomic`; don't
write another.
The UI settings worker also released the session guard after cloning settings, letting that stale
snapshot overwrite a later model-selection save. Keep the guard through the worker's disk write
so session-based writers cannot overtake each other between reading and saving.
The exit-time save must merge the session's latest AI table before writing the app's pending settings;
the UI's AI snapshot may predate a selection already committed by the hub. But waiting for that guard
can hold shutdown behind a tool for hours. Try it once; if busy, durably defer only non-AI settings
and recover them at startup against the latest committed AI table, rather than racing a live writer.
After a successful full exit save, discard an older deferred snapshot so startup cannot replay it
over the newer app settings.

**Preflight checked a different transport.** An explicit OpenCode selection bypassed ACP, but
builder preflight still checked ACP's helper and the native adapter never supplied the MCP server.
Share transport selection between preflight and execution; reject unsupported native options
before running a process rather than starting an agent without the tools its task requires.

**A config route the installed version ignores.** OpenCode V2 (2.2.5) ignores
`OPENCODE_CONFIG_CONTENT`, and `opencode run` hands the turn to a background service that never sees the
run's environment, so the builder ran without our tools and fell back to calling the CLI from a shell.
V2 also names a tool call `id` (V1: `callID`), so every tool event was dropped and the feed stayed empty
for the whole turn. Use a file in `OPENCODE_CONFIG` with `run --standalone`, and confirm a live run
reached the server (`mcp connected server=craft-anything` in OpenCode's log), not just that it started.

**A side door to a service the main path avoided.** Runs used `--standalone` so OpenCode's shared
service never saw them, and #1185 deleted the sessions a stopped turn left. But the model check
before each turn ran plain `api model.list`, which starts that service, and the service resumed a
session the person had left unfinished days before and went on editing another project for half
an hour. When a program has a mode that must be avoided, give every call to it the same flag, not
only the main one: list each command we run (`run`, `api`, `session delete`) and test each.

**Keywords that only some code paths honour.** "…in Rust" picked Rust for apps but not for games, so
a headless game rebuild stopped at once for want of Unity. It was found only by a live run. Run the
real flow end to end after changing how requests are read, and add the case as a test.

**A default given as an order.** "A notes app in Electron" was built in Rust: the brief quoted the
request, then said "Use Rust with eframe/egui… create a cargo project" with nothing saying which
wins, and the builder followed the later, more specific line; the app, the build check and Play
followed the stack picked at the start too. Say that the person's words come first and phrase a
default as one ("unless their request says otherwise"); let what acts on the result (checks,
Play, the recorded stack) look at what was built, not at what was planned.

**More than one way to build the engine.** The CLI's `commands` listing built its own engine
from default settings, so it missed the plugins (and the person's settings) every other command
had. Build the engine through one function per app (`make_engine` in the CLI) and add new layers'
tools and plugins there.

**A best-effort step that hides its errors.** Capturing a site's extra pages used `.is_ok()`, so a
missing parent folder made every page silently drop out; only a live run showed "Photographing more
pages" followed by nothing. When a step may fail without stopping the run, still say why (the feed,
a note, a log line), and test the success path end to end, not just the picking.
The fix then made the check create `checks/round-N/routes/` first, which made `round-N` exist, and
`web_site_capture` (which refuses an existing folder) stopped the run. Create a tool's output parent
right before that tool runs, never a folder that another tool must create itself.

**A size measured after drawing, used a frame late.** The prompt box's height was measured each
frame and used to size the feed on the next, so every time it grew or shrank one frame showed it
100 px too low, and typing looked like flicker. When a layout uses a size from the last frame,
call `ctx.request_discard` when it changed, so egui lays the frame out again before showing it.
The same layout then came out 17 points taller than the space it was given: it left out the item
spacing after the feed, and `ui.horizontal` centred the 85-point box 12 points down in its row.
The agent console's panel grows to fit its content, so it grew 17 points every frame until its
maximum, and then the top of its tabs was cut off. When a part is sized to fill what's left, count
the item spacing and use `horizontal_top` for tall rows, and test that the panel's size holds still
over several frames.
Its first frame had no measure yet and guessed 48 points for a box of 84 or more, so the panel grew
on that frame, and egui saved the larger size (#942). Where a panel grows to fit, guess too tall.

**A library default that blinked over a text field.** egui's scroll areas fade their edge when
there's more below; the prompt box's one measured a few points of overflow on the frame after each
keystroke, so a dark gradient blinked over the line being typed. Unit tests look at text, not at
what's painted over it. When something flickers, render the frames (`Harness::render` at a
fractional scale) and diff them, then list the shapes painted over the spot.

**A tool that locked the session it was called with.** `rebuild_question_ask` read the settings
with `hub.engine.session()`, but a tool runs while its call holds the session, so the CLI waited
forever before starting anything (found by the first live run, not by tests, which stopped
earlier with no AI set up). Inside a tool, read what you need from `ctx.session` and hand it on;
test a tool through `Engine::call` with a time limit on a thread.

**A forked session's cost is the session's.** Claude Code's `total_cost_usd` for a
`--fork-session` run is the whole session's running total ($22.67 for a question that cost cents).
Don't show a cost you can't attribute to the thing it's shown beside.
The same holds for `--resume`: a build turn's total is the session's running total, so summing
each turn's total counted the session again every turn (#1187). Keep each session's last total and
add only the increase; a per-turn `usage` block is per turn, the cost fields are not.

**An option that offered nothing new.** Asked for "a shallow live copy", the pin dialog got a
Live choice that copied nothing and opened the project's app, which is exactly Play, under a
name that suggested something was kept. It was merged, tested and installed before anyone asked
what it was for. Before adding a choice, write one sentence on what it gives that the existing
path doesn't; if there's no such sentence, ask what was meant.
Its replacement nearly repeated it: hard-linking dependencies saved almost nothing on real
projects, because 85% of a Rust pin was a `.pdb` nothing needs. Measure on real data before
designing around where you assume the size is.

**Long content above a fixed prompt box.** Answers to questions were cards between the feed and
the prompt box, sized to their text; the feed got what was left, so a long answer squeezed it and
pushed the box off the window. Content of any length belongs inside the scroll area. And an X on
something that can't be brought back is a delete: offer folding instead, and keep the data.

**A library default that hid the scroll bars.** egui's thin scroll bars paint the thumb in the
button fill and the track in the field colour; our theme made those the same, and the thin
preset narrows the bar to 2 px at rest, so the thumb showed only on hover. When a theme sets a
library's colours, check every widget that borrows them, in both themes, and test contrast with
numbers (3:1 for parts you interact with), not by eye.

**Files read while drawing.** The Project screen parsed `package.json` 2–4 times a frame, listed
the reference pictures and checked screenshot files on every repaint, and read the side panel's
files on every frame a build event arrived; on a slow or network drive the window would freeze.
Drawing reads only memory: anything from disk is read on a worker when something changes, and
the screen shows the last result.

**A non-blocking read that falls back to defaults.** The app read the person's settings with
`try_session().unwrap_or_default()` at startup, so a busy engine meant Clean room and no AI, and
the next settings change saved those defaults over theirs. Seen once in a live run. A fallback
must never stand in for data that gets saved back: wait (it's startup), or keep what's on disk.
The model picker later captured a busy session as "no project", queried in the wrong directory,
and rejected the save as a settings change. Capture settings and project under one guard; report
contention, preserve the draft and matching background results, and check again when available.
It happened again in the loader itself: `Settings::load_from` took every read error for "no file",
so a config.toml that wasn't UTF-8, or that another program held for a moment, loaded as defaults
and the next save replaced it; a file that couldn't be parsed was renamed aside with the rename's
result ignored. Only "not found" means there's nothing to lose. Defaults that stand in for a file
that is there carry that fact (`Settings::not_loaded`) and refuse to be saved, also through a
serde round trip that would drop it (`with_setting`); a backup keeps every byte under a name
nothing else has, and one that fails stops the save.

**A save that changed memory first.** Adding an AI in the app put it in the session's settings,
then the file write failed (on Windows, likely a virus scan holding config.toml), so the app showed
the AI and its role while nothing was saved, and the error sat in the message bar behind the Settings
window. Save to disk first and change memory only after it worked; replace files with a short retry
on Windows (`write_atomic`); show errors where the person is looking.

**A limit on what is kept, taken for a limit on the work.** `walk_dir` stopped keeping files at
`max_files` but went on through the whole tree, read each folder's entries into one list before
looking at any limit, and kept every folder and link it met: a tree of empty folders or links, or
one folder with millions of names, had no limit at all. Count what is examined, not only what is
kept; stop at the first thing over a limit; bound what is held while collecting (the first names in
order, so a cut result is the same every time); and test the shapes that never reach the limit on
the thing counted (no files, only links, one huge folder).

**Showing settings shows their secrets.** `settings_show` returned the whole settings, plugins'
`env` values included, which can hold API keys; a tool's output reaches the journal and MCP
clients. The journal only redacts by field name. When a setting can hold a secret, hide its value
in every tool that prints settings, and test that it doesn't appear.

**A secret check that matched a word inside a name.** `cargo xtask secrets` took any variable
with `AUTH` in its name as a secret; git sets `GIT_AUTHOR_NAME` while the hook runs, so a contributor
whose git name is their handle couldn't commit their own roadmap claim. Name the exceptions
(`NOT_SECRET`) rather than loosening the word, and test both sides with a list of variables.

**Secret files kept out only by one person's gitignore.** `.env` files and
`.claude/settings.local.json` were ignored by a contributor's global gitignore, not the repo's, so
anyone else could have committed them. Ignore secret and local files in the repo's `.gitignore`,
and run `cargo xtask secrets` before every commit (the `.githooks/pre-commit` hook does it).

**An empty float sum is `-0.0`.** `iter().map(..).sum::<f64>()` over nothing gives `-0.0`, which
`{:.2}` shows as "$-0.00". Sum money with `fold(0.0, ..)`.

**A per-run budget that a retry resets.** The build's refinement cap (`max_rounds`) counted from zero on every
run, while the round numbers carried over, so "Try again" bought six more checks each press and a build that never
worked reached check 40 and 49 (#736). A limit that must hold across runs is a count saved with the project, and
it's reset only by a new request. Test it with several runs in a row, not one.

**One field, two meanings.** `TurnDone`'s `input_tokens` included cached tokens from every coding agent but not
from the built-in loop, which keeps them apart as Anthropic reports them. Once Ollama 0.40 reported its cache, a
local build's `usage.json` said 5,323 input tokens for 75,014. When several producers fill one field, say on the
field what it counts, and test each producer with the case where they differ (here, a cached prompt).

**Docs that drift.** `STATUS.md` described a branch 25 commits old and the tool count was stale.
Update STATUS, the roadmap and generated docs (`cargo xtask tools`, `cargo xtask schemas`) in the
same PR as the change.

**A tool's title is a UI string.** A PR that added an engine tool and no UI passed every check run
for it locally, and CI's `cargo xtask i18n-coverage` failed: the app shows tool titles, so each
needs a translation in every catalog. Run `cargo xtask i18n-coverage` for any PR that adds a tool
or changes a title, not only for UI changes.

**A second lock in one statement.** `CliSpec { deep: !engine.session().clean_room(), logs: engine.session().config_dir(), … }`
looked like two quick reads, but an unbound guard lives to the end of the statement, so the second
waited on the first forever. Every build froze after studying its target while the card calmly
said "This is a big one", and no test ran that line. Read the session once; `cargo xtask locks`
and `platform::sync` now catch this. A wait message must not reassure about what it can't see:
say how long it's been since anything happened. Find a hang with `sample <pid>`: it names the lock.

**An error that points at nothing.** "Check the output below; run Claude Code by hand" came with no
output below (the event carried only message and remediation) and no command, and the program
that failed was really ssh, talking to a git alias from `~/.ssh/config` that had been offered as a
computer. Say what ran and what it printed, write the rest to a log the person can open (a failed
agent run does: `crate::failure_log`), and never name a next step you don't spell out. Exit 255
from ssh is ssh's own failure: don't blame the remote program. Test a message by causing the failure.

**A retry that took "no message" for "brand new".** A run with no message and no `continuing`
flag was read as a first build, and Try again starts exactly that run. A browser that died during
the final check failed a finished build; Try again then studied the original again and gave the
builder the first brief. A run decides what's left from what the project records, not from which
button started it, and a check that couldn't look says nothing about the build.

**A stopped tool call broke the next request.** Retaining a stopped assistant reply kept its
tool calls without matching results. The next model request then carried unpaired calls.
Every retained call needs a matching result, including an explicit non-execution result.
Test the next serialized request as well as whether a tool ran.

**A deferred request reset lost on failure.** Moving consumed-note bookkeeping out of the live
builder loop saved the reset only after a successful turn. A failed or cancelled turn had already
removed the note, so reopening kept the old unchanged-turn count. Persist deferred request state
on terminal errors too, outside provider retries; keep the original outcome if that save also fails.

**Context added to the message but not the note.** "Just ask" answers were prepended to the
builder's next message, but a note can reach a builder through its tool hook before any message,
so the note carried the request without what it referred to. Put shared context where every path
reads it (the note's hand-over file and the after-turn prompt), not in one delivery path, and test
the fast path (hook) and the fallback (after the turn) separately. Mark it told only when the note
is actually read or sent, never when it is written; and keep a note's raw text so a later run can
filter answers by its own mode.

**Lower-casing a command, then comparing its paths.** Auto mode lower-cased a shell command to
match keywords, then checked its paths against the project folder with that same text; Windows
compares paths without case, so only a Mac showed it: deleting `/Users/…/app/Build` in the
person's own project was refused. Match keywords on a lower-cased copy and compare paths as
written; test a path rule with capitals and a folder outside the temp directory.
The same check split the command at every space, quotes included, so `"…/MyGame Backup/x"`
began with the project's path and went through; it had been noted as "only refuses too much"
without trying a folder named like the project. Read arguments as the shell does, and test a
path rule with the folder beside the target, not only the target.

**A filter written for one kind of text, run on another.** The stand-in's secret scan was written for
API answers, then also ran on recorded scripts: `this.token=t` became `[dropped]` and the scripts
broke, and "Basic Membership" lost a word. It also tested whole tokens, so a secret inside a URL or
after `ref=` passed. Test a scrubber on what must survive as well as what must go, and on each
kind of content it can receive.

**Response metadata that fills a replay limit.** Comparing stored HTTP headers literally kept
equivalent polls apart: names ignore case, and only values for the same name retain order.
Compare that way without rewriting the saved headers. Unavailable 304 samples still prove which
request lacked a cached body. At capacity, replace one only with a usable response for the same
path, query and request key, then append it in recording order. Keep other requests' evidence,
count the displaced sample, and do not replace anything with a body refused by a size or secret
check. Compare complete samples before charging retained bytes, so repeats need no refund. Test an
exact total-body limit and one with room remaining, preserving the unavailable 304 and remaining budget.

**A strict tool that sent an AI off to write its own.** `game_pak_extract` refused an output
whose folder didn't exist and took one file per call; the extractor model hit that twice, then
wrote its own pak reader in Python for a large archive. Tools an AI drives make what they
can (folders), take whole sets in one call, and every brief says to fix the call rather than
replace the tool. Read an AI run's transcript for tools it gave up on.

**Two engines, two address spaces.** Ghidra loads a position-independent ELF (lowest segment at
0) at 0x100000, while the built-in engine reads it where its headers say. Handing Ghidra our
address found nothing there, and Deep mode quietly fell back to the built-in decompiler for every
PIE program and shared library. Whenever an address crosses between engines, move it by their
image bases (`ghidra::Rebase`, from the base the bridge reports) and say in the evidence which
space each address is in.

**An id that only means something on one computer, kept in a folder that travels.** The project
saved Claude Code's session id, but the session itself lives in `~/.claude/projects/` on the
computer (and under the folder) that started it; a project synced to another computer stopped at
"No conversation found" (#452), and the retry for a cut-off turn asked for the same missing session
three more times. Anything a project keeps must still work where the folder goes: keep what the id
points to as well (`agent-session.jsonl`), tell "not here" apart from a failure by a typed signal
(`Conversation::session_lost`), and never retry what can't succeed.

**An installer that downloaded what was already there.** Ghidra was found only on PATH and at two
Linux folders, so an unzipped copy in Downloads, Program Files or a Homebrew cask was missed, and
the install then downloaded the same release again (#740). Install folders are a search, not a
list: scan the folders people keep such things in (`KnownRoot`, by folder name, a set number of
levels down), and make an install use a copy already found outside our store. Test each layout
(unzipped, cask, capitalised name) with temp folders, not the machine the test runs on.

**A per-run id that a provider expects to be stable.** OpenCode Go refuses a request without
`x-opencode-session` (#1225). The header was sent, but only for backends saved with an id, so
backends from older versions or found from a key got a new id on every run. A fallback id for one
run is a workaround, not a fix: give each saved provider backend its id once, at startup, before
any session lock is held (`AiHub::ensure_session_id`), and test a restart keeps the same id.

## Agent tooling

**Shared CLI efforts reached Claude unchanged.** Keep CLI-only levels available for agents, but
normalize at the provider boundary; test serialized requests for known and unknown model families.

**A sender could exceed its MCP receiver's target limit.** Bound the handoff to the receiver's
accepted count and retain the active target; test the boundary, truncation order and remote mode.

**A finding correction reset omitted confidence to inferred.** Edit defaults must preserve existing
values unless explicitly changed; test omitted fields and every supplied confidence level.

**Atomic replacements still lost concurrent investigation updates.** A process-local mutex and
rename protect neither a cross-process revision check nor the whole read/modify/write sequence.
Hold one OS file lock from load through every rendered write, use it for all writers, bound and
cancel the wait, and test with separate processes. Never treat malformed state as an empty update.

**An optional view refresh could fail the builder's turn.** Log cache-refresh errors and continue
relaying agent events; only failures of the actual build belong on its fatal path.

**Generic ACP errors named Copilot for every adapter.** Use the current agent's display name and
the adapter's sign-in guidance; test an ACP-only adapter with and without that guidance.

**An agent's findings existed only in its own session.** A separate MCP process started without
the desktop's targets or a saved project, so its `fnd_*` records never reached the app. Local
agent chats now attach a shared investigation, hand over target metadata and refresh snapshots
after tool results. Test the child process's targets and the UI's finding ids, not only tool calls.

**A headless egui check reached rendering and then failed in teardown.** The app's named fonts
must be installed, and a test without a renderer must explicitly clear `output.textures_delta`.
Use the repository's `run_ui` API and its font setup instead of assuming an older egui harness.

**An ACP handshake passed while the tools were missing, and an empty tool list enabled them all.**
Copilot CLI 1.0.105 initialized successfully but did not expose a server supplied only through
ACP's `session/new`; explicit `--additional-mcp-config` made it available. An empty
`--available-tools=` restored native tools instead of disabling them. Verify the tools actually
sent to a fake local model, including the no-tools case, before trusting startup or argument tests.

**Shell quoting breaks generated edits.** Heredocs carrying Python with quotes, `'''` or `\n` were
mangled by the shell several times. Write multi-line edit scripts to a file and run the file.

**A Python edit that turned every line into CRLF.** `Path.write_text` on Windows writes CRLF
line endings, so a scripted size change rewrote 19 LF files with CRLF (git only warned). Write
with `write_bytes(s.encode())` and check `git ls-files --eol -m` after scripted edits.

**A process search that found itself.** Looking for a helper by a word in its command line, with
that word in the search's own command, listed the search: "a helper is left behind" after every
run, with a new pid each time. The new pid was the clue. Build the pattern from parts
(`'Set' + 'Thread'`), and don't stop processes by pattern on a machine where real builds may run.

**A command that waits for a terminal.** `git stash show -p` opened a pager and an empty
`python - <<'EOF'` heredoc opened the interactive interpreter; both sat waiting for input until
the tool timed out, with work left in a stash. Never run anything that can page or prompt: pass
`--no-pager` (or `GIT_PAGER=cat`), never feed an interpreter an empty script, and check
`git stash list` and `git status` after a timeout before carrying on.
It happened again from a leftover first line (`python -I - <<'X'` with nothing before `X`) in a
command that then wrote and ran a script: 10 minutes lost. Write the script with the editor
tool and run only `python -I <file>`.
And twice more in one night, each time a `python -I - <<'X'` with an empty body typed as a
throwaway step in a longer command. There is no safe use of `python -` here: never type it.
And again as the first step of a `sed` edit (`python -I - 2>/dev/null; sed …`): the command sat
for two minutes and the edit after it never ran. Make an edit with the editor tool, not a shell
line. The same session typed it once more, with `</dev/null` added, and that hung too: the habit
is the problem, not the input. Every Python step names a script file.

**A shell script tried in bash, run by dash.** A watcher script written for `/bin/sh` used
`kill -0 -- -<group>` and passed in Git Bash; on Ubuntu `/bin/sh` is dash, whose `kill` takes `--`
as a process id, so the script quit at once and both of its tests failed in CI. Try a `/bin/sh`
script with `dash` (or `checkbashisms`) as well as bash, and keep to what both accept.

**A search that ran no search.** `timeout 40 find … 2>/dev/null` on a Mac: there's no `timeout` there,
the error went to `/dev/null`, and the empty result read as "no such files on this computer". Don't
hide a command's errors when an empty result means something; set the time limit on the tool call.

**A file list in a variable that zsh didn't split.** `F=$(grep -l …); sed -i '' … $F` handed sed all the
names as one path; it failed, the tests that followed passed on the old code, and it read as done.
Pipe lists into `xargs`, and check the edit landed (`git status`) before reading the test result.

**A scripted edit that stops half-way.** One script deleted a file, then failed an assertion on a
later file, leaving the tree half-edited. Make edit scripts check every anchor before writing
anything, and run `git status` after any failure before trying again.

**A scripted edit that deletes more than it meant to.** A roadmap update found the end of an item
by searching for the next `- [` line; the item was the last in its section, so the search ran into
the next section and deleted its heading. Read `git diff` of every scripted edit before committing,
and bound searches to the section (stop at the next heading).

**A time limit that also covers the build.** A live browser test was run under `timeout`; the
first build of the test binary used most of the time, the limit killed the test mid-run and its
private browser was left running. Build first (`cargo test --no-run`), then run the test under the
limit, and don't kill test processes by hand to clean up after it.

**A UI tree that showed a password.** The Windows UI tree skipped values of password fields, but
.NET's UI Automation client hadn't loaded its Win32 proxies, so every WinForms control was a plain
pane, nothing was flagged as a password, and a password box's text came back as its title. Register
the client-side providers, never read a password field's title or value, treat an unrecognised edit
box as one, and test against a real window with a secret in it.

**A list of tool names that drifted.** The MCP server marked results as untrusted by a list of
tool names; two of its names no longer existed, so nothing failed and the gap went unseen. Put
such facts on the tool itself (`Effects::CARRIES_TARGET_CONTENT`), compare `Registry::canonical`
ids (which cover old ids and `mcp__…__` names) instead of raw names, and keep tool ids that other
code depends on in one constant next to the tool.

**A new tool named the old way.** Two new tools got ids outside the scheme although new tools use
`<area>_<object>_<action>` from the start, and had to be renamed. Read docs/tool-ids.md before
naming a tool.

**A stash that left a new file behind.** Work in progress was stashed to fix something on another
branch; `git stash` keeps only tracked files, so a new file stayed in the tree and `git add -A`
put it into the other PR. Use `git stash -u` (or commit the work in progress on its own branch),
and read `git status` before `git add -A`.

**Fence checks that disagreed about tool calls.** Suffix checks accepted malformed closing text,
while requiring exactly three closing backticks rejected valid longer fences. Share one boundary
parser between JSON recovery and ReAct: count the opener, require a standalone closing line at
least that long, and reject an earlier matching close followed by more text. Shorter backtick
lines inside a longer fence are content. Test both paths and the next HTTP request, including
malformed suffixes and non-call JSON examples whose strings contain protocol markers.

**A call the server couldn't read, taken for the answer.** A llama.cpp server turns a Qwen
model's `<tool_call><function=…>` markup into a real call only when it names an offered tool;
otherwise it hands the markup back as text. The loop read text with no call as the final
answer, so a 512k-window build that called `web_search` stopped its turn after 3 minutes and
failed its check. Text recovery that only knows the formats seen so far ends a turn silently on
the next one: read a model's own call format, and when it names a tool it doesn't have, tell it
so and name the ones it has, rather than ending.

**A view that asked to be scrolled on every frame.** The command palette called `scroll_to_me` on
the selected row each frame, so the wheel's scroll was pulled back to it at once and the list
kept jumping to the top (#584). Scroll something into view on the frame it changed (a key, a new
query), never as a standing request.

**A test wrote into the real config folder.** A helper that keeps one id per installation created
opencode-session under settings::config_dir() when the first request was made, so running the
tests left a file on the developer's machine and two runs could collide. Anything we persist goes
through the hub's stores (AiHub::with_stores, Session::isolated), which tests point at a temp
folder; production code never writes under the real config folder by hand.

**A per-probe walk repeated expensive work on the same bytes.** CRC cpio verification was bounded
inside one archive, but a firmware scan could rediscover overlapping, trailerless archives and
sum the same payload repeatedly. Count checksum byte visits across the whole scan, poll its
cancellation callback inside long sums, and distinguish skipped verification from a mismatch.
Test overlapping candidates with correct checksums and a complete nested archive so a structural
trailer cannot turn unchecked data into a high-confidence result.

**A line cap that cut a JSON-RPC answer in pieces.** MCP over stdio answers one message per line,
and a server with many richly described tools answers `tools/list` as several megabytes on that
one line. The process reader capped a partial line at 1 MiB and delivered it anyway, so the answer
arrived as fragments that never parsed and the plugin failed as a `tools/list` timeout — while the
same server answered a direct probe in under a second. A cap protects memory, but a JSON-RPC line
must arrive whole to parse at all: keep the cap at the size the MCP client already allows, and test
with a multi-megabyte line, not a short one.

**A capture budget checked before deduplication changed the replay.** Redaction expanded a repeated
body, so its second copy was rejected before it could be discarded as a duplicate. Apply aggregate
budgets only to distinct bodies actually retained. When a budget removes a body, preserve unavailable
cached 304 responses and deduplicate identical omitted samples.

**Two worktrees shared build artifacts during conflict checks.** Concurrent Cargo checks of
different branches reused the same package output path and one saw the other branch's engine.
Give every worktree its own `CARGO_TARGET_DIR`, even when one agent manages both. Cleaning
the shared engine cache then removed 11.6 GiB of compiled artifacts: avoid shared-cache
cleanup as a recovery shortcut; use an isolated target directory for further verification.

**A partial archive listing counted as a complete install.** A GNU-name limit correctly stopped
an archive parser, but the installer wrote earlier listed files and returned success. Preserve the
partial inventory for inspection, expose the stop as typed data, and make extraction check it before
writing any member. Missing end markers need their own compatibility control; `complete: false`
alone does not identify a refused metadata record.

**Safe recovery paths erased distinct source identities.** Source-map names such as `src//main.js`
and `src/main.js` became the same safe path, so exclusive creation kept the first file and dropped
the second. A file can also occupy a later source's parent directory, or arrive after its own
child. Preserve ordinary output paths and put collisions in an independent input/source-index
directory; keep complete source identities in the manifest. Check both prefix orders and
same-named input scripts through the real recovery tool, and retain the writer's symlink and
exclusive-creation checks after selecting a name.
