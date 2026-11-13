//! The prompt box used everywhere the person types to the builder or the assistant: it grows
//! with the text (Enter sends, Shift+Enter adds a line), shows attached pictures as chips, and
//! has the attach, stop and send buttons in its bottom row; where it takes pictures, Ctrl+V pastes
//! one ([`crate::paste`]). Up at the start of the box takes back
//! the last message still waiting, to edit it; with none waiting, Up at the very start and Down at
//! the very end go through earlier prompts, and Down past the newest brings back what was being
//! typed. Anywhere else they move the cursor as usual.

use crate::i18n::{tr, trf};
use crate::theme::{self, Tokens};
use crate::{icons, widgets};
use egui::{Margin, RichText, Ui};
use std::path::PathBuf;

/// An earlier prompt: what was typed, and the pictures attached to it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prompt {
    pub text: String,
    pub pictures: Vec<PathBuf>,
}

/// How one prompt box looks and behaves.
pub struct PromptBox<'a> {
    /// The accessible name, and the id of the text field.
    pub name: &'a str,
    pub hint: String,
    /// Visible text rows: (at least, at most before it scrolls).
    pub rows: (usize, usize),
    /// The paperclip's tooltip; `None` hides it (no attachments here).
    pub attach: Option<&'a str>,
    /// A small picture of each attachment, in order, once it's decoded (`CraftApp::previews`).
    pub previews: &'a [Option<egui::TextureHandle>],
    /// A run or answer is in progress: a stop button beside Send, named this ("Stop", "Pause").
    pub stop_button: Option<&'a str>,
    /// The stop was asked for and hasn't finished: the button says so and ignores clicks (#551).
    pub stopping: bool,
    /// Show the send button (the Make screen has its own big Start button).
    pub send: bool,
    /// A sent message is still waiting and can be taken back: Up at the start of the box asks for it.
    pub recall: bool,
    /// Earlier prompts, oldest first, for Up and Down; read only when a key asks for them.
    pub history: Option<&'a dyn Fn() -> Vec<Prompt>>,
    pub font_size: f32,
}

/// Going through earlier prompts: how far back (0 is what was being typed), and what the box
/// holds for it to still count.
#[derive(Debug, Clone, Default, PartialEq)]
struct Browsing {
    back: usize,
    shown: String,
    /// What was being typed, for Down past the newest.
    draft: String,
    /// The pictures attached to what was being typed.
    draft_pictures: Vec<PathBuf>,
    /// The pictures of the prompt showing.
    shown_pictures: Vec<PathBuf>,
}

/// The prompt before the one showing; none when there's no earlier one.
fn older(history: &[Prompt], now: Option<&Browsing>, text: &str, pictures: &[PathBuf]) -> Option<Browsing> {
    let back = now.map_or(1, |b| b.back + 1);
    let p = history.get(history.len().checked_sub(back)?)?;
    Some(Browsing {
        back,
        shown: p.text.clone(),
        draft: now.map_or_else(|| text.to_string(), |b| b.draft.clone()),
        draft_pictures: now.map_or_else(|| pictures.to_vec(), |b| b.draft_pictures.clone()),
        shown_pictures: p.pictures.clone(),
    })
}

/// The prompt after the one showing, or what was being typed once past the newest.
fn newer(history: &[Prompt], now: &Browsing) -> Browsing {
    let back = now.back.saturating_sub(1);
    match history.len().checked_sub(back).and_then(|i| history.get(i)) {
        Some(p) if back > 0 => Browsing { back, shown: p.text.clone(), shown_pictures: p.pictures.clone(), ..now.clone() },
        _ => Browsing { back: 0, shown: now.draft.clone(), shown_pictures: now.draft_pictures.clone(), ..now.clone() },
    }
}

/// Up at the start or Down at the end went through earlier prompts: consumed before the field
/// sees them. Up leaves the cursor at the start and Down at the end, so pressing it again goes on.
/// Returns the pictures the box should hold now, when it moved to another prompt.
fn browse(ui: &mut Ui, id: egui::Id, text: &mut String, attachments: &[PathBuf], history: &dyn Fn() -> Vec<Prompt>) -> Option<Vec<PathBuf>> {
    let key = id.with("history");
    // Editing what came back, or sending it, ends going through them.
    let now = ui.data_mut(|d| d.get_temp::<Browsing>(key)).filter(|b| b.shown == *text);
    let list = || {
        let mut list = history();
        list.dedup();
        list
    };
    let plain = egui::Modifiers::NONE;
    let up = at_start(ui.ctx(), id, text) && ui.input_mut(|i| i.consume_key(plain, egui::Key::ArrowUp));
    let next = if up {
        older(&list(), now.as_ref(), text, attachments)
    } else {
        now.as_ref().filter(|_| at_end(ui.ctx(), id, text) && ui.input_mut(|i| i.consume_key(plain, egui::Key::ArrowDown))).map(|b| newer(&list(), b))
    };
    let b = next?;
    text.clone_from(&b.shown);
    if b.back == 0 {
        ui.data_mut(|d| d.remove_temp::<Browsing>(key));
    } else {
        ui.data_mut(|d| d.insert_temp(key, b.clone()));
    }
    if up {
        cursor_to(ui.ctx(), id, 0);
    } else {
        cursor_to_end(ui.ctx(), id, text);
    }
    Some(b.shown_pictures)
}

/// What the person did with it this frame.
#[derive(Default)]
pub struct PromptOut {
    /// Send was pressed (button or Enter) with something to send.
    pub send: bool,
    pub attach: bool,
    /// Ctrl+V with something other than text on the clipboard, where the box takes pictures.
    pub paste: bool,
    pub stop: bool,
    /// Up was pressed at the start of the box while a message could be taken back.
    pub recall: bool,
    /// The attachment to remove.
    pub remove: Option<usize>,
    /// Up or Down moved to another prompt: the pictures the box should hold now (the draft's own
    /// when it came back to what was being typed). `None` leaves them as they are.
    pub restore: Option<Vec<PathBuf>>,
}

pub fn show(ui: &mut Ui, text: &mut String, attachments: &[PathBuf], b: PromptBox) -> PromptOut {
    let t = Tokens::get(ui.ctx());
    let mut out = PromptOut::default();
    let id = ui.make_persistent_id(("prompt-box", b.name));
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = egui::Stroke::new(1.0, if focused { t.text_faint } else { t.field_border });
    let frame = egui::Frame::new()
        .fill(t.card)
        .stroke(stroke)
        .corner_radius(t.r_lg())
        .shadow(egui::Shadow { offset: [0, 2], blur: 12, spread: 0, color: t.shadow })
        .inner_margin(Margin { left: 16, right: 10, top: 14, bottom: 10 });
    // Stacked top to bottom whatever the caller's layout (it's often placed in a row).
    frame.show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(ui.available_width());
            if !attachments.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (i, p) in attachments.iter().enumerate() {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let remove = trf("Remove {name}", &[("name", &name)]);
                        match b.previews.get(i).cloned().flatten() {
                            // A picture: a square tile filled with its middle, the whole of it on
                            // hover, and the x in its corner.
                            Some(tex) => {
                                let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(TILE), egui::Sense::hover());
                                egui::Image::new((tex.id(), rect.size())).uv(middle(tex.size_vec2())).corner_radius(t.r_lg()).paint_at(ui, rect);
                                ui.painter().rect_stroke(rect, t.r_lg(), egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
                                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &name));
                                resp.on_hover_ui(|ui| {
                                    ui.add(egui::Image::new((tex.id(), fit(tex.size_vec2(), PREVIEW_HOVER))));
                                    ui.label(RichText::new(p.display().to_string()).size(theme::CAPTION));
                                });
                                let x = egui::Rect::from_min_size(rect.right_top() + egui::vec2(-26.0, 4.0), egui::Vec2::splat(22.0));
                                ui.painter().circle_filled(x.center(), 10.0, t.card);
                                // A child of its own, so the row's next tile isn't placed after it.
                                if icons::button(&mut ui.new_child(egui::UiBuilder::new().max_rect(x)), "x", 22.0, false, &remove).clicked() {
                                    out.remove = Some(i);
                                }
                            }
                            // Not decoded yet, or not readable: its name.
                            None => {
                                egui::Frame::new()
                                    .fill(t.raised)
                                    .stroke(egui::Stroke::new(1.0, t.card_border))
                                    .corner_radius(t.r_sm())
                                    .inner_margin(Margin { left: 8, right: 2, top: 2, bottom: 2 })
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            icons::show(ui, "paperclip", 13.0, t.icon);
                                            ui.add(egui::Label::new(RichText::new(&name).size(theme::SMALL)).truncate()).on_hover_text(p.display().to_string());
                                            if icons::button(ui, "x", 24.0, false, &remove).clicked() {
                                                out.remove = Some(i);
                                            }
                                        });
                                    });
                            }
                        }
                    }
                });
                ui.add_space(6.0);
            }
            let line = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(b.font_size)));
            // Up at the very start (or in an empty box) takes back the last waiting message; it's
            // consumed before the field sees it, and anywhere else Up moves the cursor as usual.
            // The caller fills the text in after this frame; the cursor goes to its end on the next.
            let to_end = id.with("cursor-to-end");
            if ui.data_mut(|d| d.remove_temp::<bool>(to_end)).is_some() {
                cursor_to_end(ui.ctx(), id, text);
            }
            if b.recall && focused && at_start(ui.ctx(), id, text) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                out.recall = true;
                ui.data_mut(|d| d.insert_temp(to_end, true));
            } else if let Some(history) = b.history.filter(|_| focused && !b.recall) {
                out.restore = browse(ui, id, text, attachments, history);
            }
            // What Tab would complete to, when the box starts with a partial command name. egui
            // decides keyboard navigation at the start of the pass, so the field keeps Tab only
            // while there is something to complete; anywhere else Tab moves focus as usual.
            let completing = completed_command(text);
            if focused
                && let Some(completed) = &completing
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
            {
                text.clone_from(completed);
                cursor_to_end(ui.ctx(), id, text);
            }
            // No "more below" fade: the frame after each keystroke the scroll area briefly
            // measures a few points of overflow, and the fade blinked over the line being typed.
            ui.spacing_mut().scroll.fade.strength = 0.0;
            let tallest = tallest_text(line, b.rows.1, ui.ctx().content_rect().height());
            egui::ScrollArea::vertical().id_salt(id.with("scroll")).max_height(tallest + 4.0).stick_to_bottom(true).show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(text)
                        .id(id)
                        .hint_text(&b.hint)
                        .frame(egui::Frame::NONE)
                        .desired_rows(b.rows.0)
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::proportional(b.font_size))
                        // Enter sends; Shift+Enter starts a new line.
                        .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)))
                        .event_filter(egui::EventFilter {
                            tab: completing.is_some(),
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            ..egui::EventFilter::default()
                        }),
                );
                widgets::named(r.clone(), b.name);
                if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) && !text.trim().is_empty() {
                    out.send = true;
                }
            });
            // A paste of a picture never reaches the text field: it's attached instead, when the box
            // has the focus or nothing else does.
            let pasted = b.attach.is_some() && crate::paste::picture_wanted(ui.ctx());
            if pasted && (focused || ui.memory(|m| m.focused().is_none())) {
                out.paste = true;
            }
            // Hint: available slash commands when typing one.
            if text.trim_start().starts_with('/') && !text.trim().contains(' ') {
                ui.label(RichText::new(format!("{} {}", tr("Commands:"), crate::slash::names())).size(theme::CAPTION).color(t.text_faint));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if let Some(tip) = b.attach
                    && icons::button(ui, "paperclip", 30.0, false, tip).clicked()
                {
                    out.attach = true;
                }
                ui.label(RichText::new(tr("Enter to send · Shift+Enter for a new line")).size(theme::CAPTION).color(t.text_faint));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if b.send {
                        let ready = !text.trim().is_empty();
                        if send_button(ui, ready).clicked() && ready {
                            out.send = true;
                        }
                    }
                    // Named, not a bare icon: the hint says "press Pause", so the button says Pause.
                    if let Some(name) = b.stop_button {
                        let label = if !b.stopping {
                            name
                        } else if name == "Pause" {
                            "Pausing…"
                        } else {
                            "Stopping…"
                        };
                        if widgets::secondary(ui, Some("circle-stop"), label).clicked() && !b.stopping {
                            out.stop = true;
                        }
                    }
                });
            });
        })
    });
    out
}

/// An attachment's tile (points, square), and the box its whole picture fits in on hover.
const TILE: f32 = 96.0;
const PREVIEW_HOVER: egui::Vec2 = egui::vec2(360.0, 360.0);

/// The middle square of a picture of `size`, as texture coordinates: a tile is filled, not stretched.
fn middle(size: egui::Vec2) -> egui::Rect {
    let full = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    if size.x <= 0.0 || size.y <= 0.0 {
        return full;
    }
    let (w, h) = ((size.y / size.x).min(1.0), (size.x / size.y).min(1.0));
    egui::Rect::from_center_size(full.center(), egui::vec2(w, h))
}

/// `size` scaled to fit in `max`, keeping its shape.
fn fit(size: egui::Vec2, max: egui::Vec2) -> egui::Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return max;
    }
    size * (max.x / size.x).min(max.y / size.y)
}

/// How tall the text may grow before it scrolls: the box's own limit of `rows` lines, or
/// two-fifths of a taller window, so a long prompt shows more of itself there (#473).
fn tallest_text(line: f32, rows: usize, window: f32) -> f32 {
    (line * rows as f32).max(window * 0.4)
}

/// What Tab would complete the box to, when it starts with a partial command name. Only the
/// name is replaced, so a question after it is kept; `None` when there is nothing to complete.
fn completed_command(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('/')?;
    let name = rest.split_whitespace().next().unwrap_or(rest);
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let full = crate::slash::complete(name)?;
    if name.eq_ignore_ascii_case(full) {
        return None;
    }
    let leading = text.len() - trimmed.len();
    Some(format!("{}/{full} {}", &text[..leading], rest[name.len()..].trim_start()))
}

/// The cursor is at this character, selecting nothing (or there's no text, or no cursor yet).
fn cursor_at(ctx: &egui::Context, id: egui::Id, text: &str, index: usize) -> bool {
    text.is_empty()
        || egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range()).is_none_or(|r| r.primary.index.0 == index && r.secondary.index.0 == index)
}

fn at_start(ctx: &egui::Context, id: egui::Id, text: &str) -> bool {
    cursor_at(ctx, id, text, 0)
}

fn at_end(ctx: &egui::Context, id: egui::Id, text: &str) -> bool {
    cursor_at(ctx, id, text, text.chars().count())
}

fn cursor_to(ctx: &egui::Context, id: egui::Id, index: usize) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(index))));
    egui::TextEdit::store_state(ctx, id, state);
}

/// Put the cursor at the end of the text.
fn cursor_to_end(ctx: &egui::Context, id: egui::Id, text: &str) {
    cursor_to(ctx, id, text.chars().count());
}

/// Send: an app-colour square once there's something to send, a quiet one before.
fn send_button(ui: &mut Ui, ready: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::click());
    let fill = if ready { crate::theme::mix(t.accent, t.accent_hover, widgets::hover_t(ui, &resp)) } else { t.raised };
    ui.painter().rect_filled(rect, t.radius, fill);
    icons::paint(ui, rect, "send", 16.0, if ready { t.on_accent } else { t.text_faint });
    widgets::focus_ring(ui, &resp, rect);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("Send (Enter)")));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr("Send (Enter)"))
}

/// "2 pictures attached" for the feed, or nothing.
pub fn attached_line(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => tr("(1 picture attached)"),
        n => trf("({n} pictures attached)", &[("n", &n.to_string())]),
    }
}

/// A sent message as it was typed: what the feed shows, without its [`attached_line`].
pub fn as_typed(shown: &str) -> &str {
    let Some((text, last)) = shown.rsplit_once('\n') else { return shown };
    let n: Option<usize> = last.chars().filter(char::is_ascii_digit).collect::<String>().parse().ok();
    if n.is_some_and(|n| attached_line(n) == last) { text } else { shown }
}

/// The text the agent gets: the message, then where its pictures are so it opens them.
pub fn with_attachments(text: &str, files: &[PathBuf], root: &std::path::Path) -> String {
    if files.is_empty() {
        return text.to_string();
    }
    let list = files.iter().map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string().replace('\\', "/")).collect::<Vec<_>>().join(", ");
    format!("{text}\n\nAttached pictures: {list}. Open each one with the Read tool; they show what the person means.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #473: eight lines at most, whatever the window, left most of a detailed prompt out of view.
    #[test]
    fn a_long_prompt_shows_more_of_itself_in_a_tall_window() {
        assert_eq!(tallest_text(20.0, 8, 300.0), 160.0, "a small window keeps the box's own limit");
        assert_eq!(tallest_text(20.0, 8, 1400.0), 560.0, "a tall one shows up to two-fifths of it");
        assert_eq!(tallest_text(20.0, 30, 1000.0), 600.0, "a box allowed more keeps it");
    }

    #[test]
    fn up_goes_back_through_prompts_and_down_returns_to_what_was_being_typed() {
        let list = [Prompt { text: "one".into(), ..Default::default() }, Prompt { text: "two".into(), ..Default::default() }];
        let first = older(&list, None, "half a thou", &[]).unwrap();
        assert_eq!((first.shown.as_str(), first.draft.as_str()), ("two", "half a thou"));
        let second = older(&list, Some(&first), &first.shown, &[]).unwrap();
        assert_eq!((second.shown.as_str(), second.draft.as_str()), ("one", "half a thou"));
        assert_eq!(older(&list, Some(&second), &second.shown, &[]), None, "the oldest stays");
        assert_eq!(newer(&list, &second).shown, "two");
        let draft = newer(&list, &first);
        assert_eq!((draft.back, draft.shown.as_str()), (0, "half a thou"), "past the newest is the draft");
        assert_eq!(older(&[], None, "", &[]), None);
    }

    #[test]
    fn each_prompt_brings_back_its_own_pictures_and_the_draft_its_own() {
        let one = Prompt { text: "one".into(), pictures: vec![PathBuf::from("a.png")] };
        let two = Prompt { text: "two".into(), pictures: vec![PathBuf::from("b.png"), PathBuf::from("c.png")] };
        let list = [one.clone(), two.clone()];
        let draft_pictures = vec![PathBuf::from("draft.png")];
        let first = older(&list, None, "half a thou", &draft_pictures).unwrap();
        assert_eq!(first.shown_pictures, two.pictures, "the newest prompt's pictures");
        assert_eq!(first.draft_pictures, draft_pictures, "the draft's are kept for Down");
        let second = older(&list, Some(&first), &first.shown, &draft_pictures).unwrap();
        assert_eq!(second.shown_pictures, one.pictures, "the earlier prompt's own pictures");
        let back = newer(&list, &second);
        assert_eq!((back.shown.as_str(), back.shown_pictures.clone()), ("two", two.pictures.clone()));
        let draft = newer(&list, &back);
        assert_eq!((draft.back, draft.shown.as_str()), (0, "half a thou"));
        assert_eq!(draft.shown_pictures, draft_pictures, "the draft's own pictures come back");
    }

    #[test]
    fn picture_tiles_sit_side_by_side_in_one_row() {
        use egui_kittest::kittest::Queryable as _;
        let files: Vec<PathBuf> = ["a.png", "b.png", "c.png"].map(PathBuf::from).to_vec();
        let mut h = egui_kittest::Harness::new_ui_state(
            move |ui, text: &mut String| {
                let tex = ui.ctx().load_texture("tile", egui::ColorImage::filled([4, 2], egui::Color32::RED), egui::TextureOptions::LINEAR);
                let previews = vec![Some(tex); 3];
                let b = PromptBox {
                    name: "Box",
                    hint: String::new(),
                    rows: (1, 4),
                    attach: Some("Attach"),
                    previews: &previews,
                    stop_button: None,
                    stopping: false,
                    send: true,
                    recall: false,
                    history: None,
                    font_size: 14.0,
                };
                show(ui, text, &files, b);
            },
            String::new(),
        );
        h.run();
        let tiles: Vec<egui::Rect> = ["a.png", "b.png", "c.png"].iter().map(|n| h.get_by_label(n).rect()).collect();
        assert!(tiles.iter().all(|r| (r.top() - tiles[0].top()).abs() < 0.5 && (r.width() - TILE).abs() < 0.5), "{tiles:?}");
        assert!(tiles.windows(2).all(|w| w[1].left() > w[0].right()), "side by side, not overlapping: {tiles:?}");
        let x = h.get_by_label("Remove b.png").rect();
        assert!(tiles[1].contains_rect(x), "the x sits inside its tile: {x:?} in {:?}", tiles[1]);
    }

    #[test]
    fn a_tile_shows_the_middle_square_of_its_picture() {
        let r = middle(egui::vec2(400.0, 200.0));
        assert_eq!((r.min.x, r.max.x, r.min.y, r.max.y), (0.25, 0.75, 0.0, 1.0));
        let r = middle(egui::vec2(100.0, 400.0));
        assert_eq!((r.min.x, r.max.x, r.min.y, r.max.y), (0.0, 1.0, 0.375, 0.625));
        assert_eq!(middle(egui::Vec2::ZERO).max, egui::pos2(1.0, 1.0));
        assert_eq!(fit(egui::vec2(720.0, 360.0), PREVIEW_HOVER), egui::vec2(360.0, 180.0));
    }

    #[test]
    fn a_prompt_comes_back_without_its_pictures_line() {
        assert_eq!(as_typed("Add tags\n(2 pictures attached)"), "Add tags");
        assert_eq!(as_typed("Add tags\n(1 picture attached)"), "Add tags");
        assert_eq!(as_typed("First\nsecond line"), "First\nsecond line");
        assert_eq!(as_typed("Use 3\ncolours"), "Use 3\ncolours");
    }

    /// #551: the hint says "press Pause", so a labelled Pause button is there; after it's pressed
    /// the button says Pausing… and a second click does nothing.
    #[test]
    fn the_pause_button_is_named_and_says_when_it_was_pressed() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable as _;
        #[derive(Default)]
        struct Screen {
            stops: u32,
            stopping: bool,
        }
        impl eframe::App for Screen {
            fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
                egui::CentralPanel::default().show(ui, |ui| {
                    let b = PromptBox {
                        name: "Ask for a change",
                        hint: String::new(),
                        rows: (1, 8),
                        attach: None,
                        previews: &[],
                        stop_button: Some("Pause"),
                        stopping: self.stopping,
                        send: true,
                        recall: false,
                        history: None,
                        font_size: 14.0,
                    };
                    if show(ui, &mut String::new(), &[], b).stop {
                        self.stops += 1;
                    }
                });
            }
        }
        let mut h = Harness::builder().with_size(egui::vec2(800.0, 400.0)).build_eframe(|cc| {
            crate::CraftApp::setup_context(&cc.egui_ctx);
            Screen::default()
        });
        h.run();
        h.get_by_label("Pause").click();
        h.run();
        assert_eq!(h.state().stops, 1);
        h.state_mut().stopping = true;
        h.run();
        assert!(h.query_by_label("Pause").is_none());
        h.get_by_label("Pausing…").click();
        h.run();
        assert_eq!(h.state().stops, 1, "pressing it again changes nothing");
    }
}
