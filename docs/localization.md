# UI localisation

The desktop app offers **Čeština** (`cs`), **Deutsch** (`de`), **Eesti** (`et`), **Español** (`es`), **Italiano** (`it`), **ქართული** (`ka`),
**한국어** (`ko`), **Русский** (`ru`), **Tiếng Việt** (`vi`), **Português (Brasil)** (`pt-br`), **Türkçe** (`tr`), **ไทย** (`th`), **简体中文** (`zh-hans`) and **繁體中文** (`zh-hant`), alongside
English, French and Japanese, in **Settings → General → Language**. The choice is saved in the
existing `language` setting. **System** uses the operating system's preferred
supported language; if none matches, it uses English. An explicit choice takes
precedence over the system language.

Chinese locales without a script, including `zh`, `zh-CN` and `zh-SG`, select
Simplified Chinese. `zh-TW`, `zh-HK` and `zh-MO` select Traditional Chinese.
An explicit `Hans` or `Hant` script takes precedence over the region. Locale
matching accepts underscores, case differences and Unix encoding suffixes.

## German

**Deutsch** (`de`) uses the informal "du". `de`, `de-DE`, `de-AT` and `de-CH` select it, as does the
system language when it is German. `crates/ui-egui/src/i18n/de.tsv` has the same keys as the Chinese
catalogs (a unit test compares them), written from the English source strings, not from the Chinese.
It was drafted with an AI and not yet reviewed by a native speaker; corrections are welcome.
Product and technology names, tool ids and shortcuts keep their spelling. The kind detection that reads
a request ("a game", "a website") also knows the German words *Spiel*, *Webseite* and *Internetseite*,
so a translated suggestion keeps its project kind.

## Estonian

**Eesti** (`et`) is selected by `et` and `et-EE`, and by the system language when it is Estonian.
`crates/ui-egui/src/i18n/et.tsv` has the same keys as the other catalogs, written from the English
source. It was drafted with an AI from the English source, so corrections from native speakers are
welcome. Product and technology names, tool ids and shortcuts keep their spelling. The glossary:
*Puhas ruum* (Clean room), *Sügav* (Deep), *Uuri / Ehita / Kontrolli / Valmis* (Study / Build / Check /
Ready), *Tõendid* (Evidence), *leid* (finding), *projekt* (Project), *sihtmärk* (target), *tööriist*
(tool), *töölaud* (workbench), *ehitaja* (builder), *Seaded* (Settings) and *taasehitus* (rebuild).

The kind detection that reads a request also knows *mäng* for a game and *veebileht*, *koduleht* and
*veebisait* for a website, so a translated suggestion keeps its project kind. If a request names
both, the head of the main clause wins: a complement case (*mängust*, *veebilehega*) marks the
source or topic, not the thing to build, so *Tee sellest mängust veebileht* is a website while
*Loo mäng nimega Veebileht* is a game.

## Georgian

**ქართული** (`ka`) is selected by `ka` and `ka-GE`, and by the system language when it is Georgian.
`crates/ui-egui/src/i18n/ka.tsv` has the same keys as the other catalogs. Written with Claude Code from the
meaning of each English string; terminology and doubtful strings decided by a native Georgian speaker (@devius).
Glyphs come from Noto Sans Georgian in craft-fonts, which the pinned craft-fonts already has; a test checks it.
## Italian

**Italiano** (`it`) uses the informal "tu". `it` and `it-IT` select it, as does the system language when it is
Italian. `crates/ui-egui/src/i18n/it.tsv` has the same keys as the other catalogs, written from the English
source strings. It was drafted with an AI and not yet reviewed by a native speaker; corrections are welcome.
Product and technology names, tool ids and shortcuts keep their spelling. The kind detection that reads a
request also knows *gioco* and *sito*, so a translated suggestion keeps its project kind. Clean room is
*Camera bianca* and Deep mode is *modalità Approfondita*.

## Vietnamese

**Tiếng Việt** (`vi`) addresses the person as *bạn*. `vi` and `vi-VN` select it, as does the system
language when it is Vietnamese. `crates/ui-egui/src/i18n/vi.tsv` has the same keys as the other
catalogs, written from the English source strings in NFC (precomposed) Unicode. It was drafted with
an AI and not yet reviewed by a native speaker; corrections are welcome. Product, technology and
engine names, tool ids and shortcuts keep their spelling. The glossary: *Phòng sạch* (Clean room),
*Chuyên sâu* (Deep), *Tìm hiểu / Xây dựng / Kiểm tra / Tinh chỉnh / Sẵn sàng*
(Study / Build / Check / Refine / Ready), *Bằng chứng* (Evidence), *Phát hiện* (finding),
*Dự án* (Project), *Mục tiêu* (target), *Công cụ* (tool), *Bàn làm việc* (workbench),
*Cài đặt* (Settings) and *Dựng lại* (rebuild).

The kind detection that reads a request also knows *trò chơi* for a game and *trang web* for a
website, so a translated suggestion keeps its project kind.

## Turkish and Thai

**Türkçe** (`tr`) is selected by `tr` and `tr-TR`, and **ไทย** (`th`) by `th` and
`th-TH`. Both have the same keys as the other catalogs. A request's project kind also knows *jogo*,
and *oyun* with its endings as whole words (so *oyuncak*, a toy, stays an app), and *web sitesi* and
*internet sitesi*; the Thai catalog adds *เกม* for a game and *เว็บไซต์* and *หน้าเว็บ* for a website,
so a translated suggestion keeps its project kind.

The Thai catalog addresses the person as *คุณ* and keeps **Clean room** and **Deep** in English, as
the title bar does. The glossary: *หลักฐาน* (Evidence), *สิ่งที่พบ* (finding), *โปรเจกต์* (Project),
*เป้าหมาย* (target), *เครื่องมือ* (tool), *เวิร์กเบนช์* (workbench), *ตัวสร้าง* (builder),
*การตั้งค่า* (Settings) and *ศึกษา / สร้าง / ตรวจ / พร้อม* for Study / Build / Check / Ready.
It was drafted with an AI and not yet reviewed by a native speaker; corrections are welcome.
Thai glyphs come from Noto Sans Thai in craft-fonts, which the script test in `theme.rs` checks.

## Scope and format

`crates/ui-egui/src/i18n/cs.tsv`, `de.tsv`, `es.tsv`, `et.tsv`, `it.tsv`, `ka.tsv`, `ko.tsv`, `pt-br.tsv`, `ru.tsv`, `th.tsv`, `tr.tsv`, `vi.tsv`, `zh-hans.tsv` and
`zh-hant.tsv` contain original
translations under the repository's MIT OR Apache-2.0 licence. Each UTF-8 line
has two columns: `English source<TAB>translation`. Comments begin with `#`;
`\n` represents a newline. The language registry in `i18n.rs` embeds the catalogs.

The catalogs cover first run, new projects, project controls, Settings, the
workbench, tool titles, tool groups, target descriptions, tooltips and
accessibility labels. The command palette searches translated tool titles as
well as English titles and stable tool IDs. Missing translations fall back to
the original English text. Product and technology names retain their spelling.

User text, AI responses, evidence and raw tool results, tool summaries and schema
descriptions, logs and exported reports retain their original text. The CLI,
MCP, tool IDs, stored project data and the separate browser viewer are unchanged.

Keep placeholders such as `{name}` and `{n}` intact, along with command IDs,
shortcuts, escapes and trailing `…`. Update every catalog (`cs`, `de`, `es`, `et`, `it`, `ka`, `ko`, `pt-br`, `ru`, `th`, `tr`, `vi`, `zh-hans`, `zh-hant`)
when English source keys change; the tests require matching key sets. Common terms include 净室 / 淨室 (Clean room), 深度 (Deep),
构建 / 建置 (Build), 证据 / 證據 (Evidence), and 项目 / 專案 (Project).
The Korean catalog `ko.tsv` uses 클린룸 (Clean room), 심층 (Deep), 분석 / 만들기 / 검사 / 완성
(Study / Build / Check / Ready), 근거 (Evidence) and 프로젝트 (Project); short labels are nouns and
sentences use the formal 합니다 style.

## Spanish

**Español** (`es`) uses informal "tú", Mexican vocabulary (*agregar*, *computadora*, *presiona*) and
`es`, `es-MX`, `es-419` and the other `es-` regions select it. The register, the Mexican vocabulary
and the glossary below were chosen by a native speaker; the catalog itself was drafted with AI
assistance from the English source, so corrections from other speakers are welcome.

Product, technology and mode names keep their spelling: **Clean room** and **Deep** stay in English
because they name the two modes in the title bar, as do tool and engine names (Ghidra, Unity, Unreal,
Godot, Claude Code). The terms the app itself translates follow one glossary: *construir* /
*construcción* (Build), *evidencia* (Evidence), *proyecto* (Project), *hallazgo* (finding),
*objetivo* (target), *herramienta* (tool), *banco de trabajo* (workbench), *constructor* (builder),
*Configuración* (Settings) and *Estudiar / Construir / Verificar / Listo* for
Study / Build / Check / Ready. *Plugin* stays as it is, in all 16 strings that use it, rather than the
suite's *complemento*: it is the word the people using the interface say in Mexican Spanish, and a
catalogue that changes term halfway reads worse than one that keeps a loanword.

The kind detection that reads a request also knows the Spanish words *juego*, *juegos*, *jugabilidad*
and *videojuego* for a game, and *sitio web* and *página web* for a website, so a translated
suggestion keeps its project kind. The bare word *web* is deliberately not a signal: *una aplicación
web* is an app, not a website.

## Validation

Run:

```sh
cargo test -p craft-anything-ui-egui
cargo test -p xtask
cargo clippy -p craft-anything-ui-egui -p xtask --all-targets -- -D warnings
cargo xtask i18n-coverage
```

Catalog tests check duplicate and empty entries, matching key sets, placeholders
and ellipses. The coverage command checks literal translation calls, common
widget labels and registered engine tool titles and groups; it also runs in
`cargo xtask ci`. It does not prove coverage of arbitrary runtime text or labels
held in tables, so review those when adding or changing UI code.

A UI change can land with its new strings in English while native speakers review the
translations in a PR of their own. List each new string in `crates/ui-egui/src/i18n/awaiting.txt`
(one per line, as in a catalog's first column). The coverage command lets the strings listed
there be missing, and the app shows them in English until a catalog has them. The command fails
once a listed string is in every catalog, or is no longer shown, so the translation PR takes it
out of the list. The catalog tests require every catalog to have the same keys, so that PR adds
each string to all of them at once. Only strings the command finds can be listed: a literal in
`tr`, `trf` or a widget's label, not one passed through a constant (`tr(LABEL)`), which the
command can't see and would report as not shown.

The existing snapshot example accepts any catalog code with `--lang`, such as
`--lang zh-hant` or `--lang ko`, for example:

```sh
cargo run -p craft-anything-ui-egui --example snapshot -- --lang zh-hant --only settings --out-dir target/snapshots/zh-hant
```

Chinese, Korean and Georgian glyphs use the existing craft-fonts fallback, which release and CI builds
Spanish and Estonian are Latin script and need no extra glyphs. Chinese and Korean glyphs use the existing craft-fonts fallback, which release and CI builds
Vietnamese needs craft-fonts too: the bundled DM Sans and egui's default fonts lack the letters with
stacked marks (ạ, ả, ấ, ế, ơ, ư, ữ…), so a build without craft-fonts shows them as boxes. With
craft-fonts, Noto Sans CJK SC supplies those letters (a different face from the DM Sans around them);
the script test in `theme.rs` checks every one of them. Chinese and Korean glyphs use the existing craft-fonts fallback, which release and CI builds
already include. Local builds must set `CRAFT_FONTS_DIR` to a craft-fonts checkout
before compiling to get the same glyph coverage; with `CRAFT_FONTS_REQUIRED=1` set as well, a
build that would leave the fonts out (no `CRAFT_FONTS_DIR`, or a bad one) fails instead of
quietly showing boxes. The catalogs add no font assets.
On Windows, a source build without craft-fonts also tries installed CJK fonts, starting with
Microsoft YaHei. A worker reads at most 32 MiB per candidate and checks the font tables before
adding a fallback to body, code, medium and semibold text. These system fonts stay on the
computer; they are not embedded or redistributed. Missing or invalid files are skipped.
This fixes missing Chinese glyphs without changing the UTF-8 catalogs. Other scripts still
need the craft-fonts bundle for the coverage described above.
Check each script, clipping, narrow windows and native file dialogs visually
on each platform; catalog coverage alone does not verify layout or fonts.
