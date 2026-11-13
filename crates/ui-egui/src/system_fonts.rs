//! A local Windows CJK fallback for source builds that have no optional font bundle.
//! Read installed fonts on a worker; never redistribute them or block a UI frame.

use egui::{FontData, FontDefinitions};
use read_fonts::TableProvider;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, OnceLock};

const FONT_NAME: &str = "windows-cjk";
const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;
const CANDIDATES: &[&str] = &["msyh.ttc", "msjh.ttc", "simhei.ttf", "simsun.ttc"];

/// Preserve the initial font stack until the worker has a usable fallback ready.
pub(crate) fn install(ctx: &egui::Context, fonts: FontDefinitions) {
    let ctx = ctx.clone();
    let result = std::thread::Builder::new().name("windows-fonts".into()).spawn(move || {
        static FONT: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
        let font = FONT.get_or_init(|| {
            let root = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"))?;
            from_directory(&Path::new(&root).join("Fonts"))
        });
        if let Some(font) = font {
            ctx.set_fonts(with_fallback(fonts, Arc::clone(font)));
            ctx.request_repaint();
        } else {
            log::warn!("No usable Windows CJK font was found; set CRAFT_FONTS_DIR to include additional fonts");
        }
    });
    if let Err(error) = result {
        log::warn!("Unable to start the Windows font loader: {error}");
    }
}

fn from_directory(directory: &Path) -> Option<Arc<FontData>> {
    CANDIDATES.iter().find_map(|file| read_font(&directory.join(file), MAX_FONT_BYTES).map(Arc::new))
}

fn read_font(path: &Path, limit: u64) -> Option<FontData> {
    let cap = limit.checked_add(1)?;
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes).ok()?;
    if u64::try_from(bytes.len()).ok()? > limit {
        return None;
    }
    let face = read_fonts::FontRef::from_index(&bytes, 0).ok()?;
    face.head().ok()?;
    face.maxp().ok()?;
    face.cmap().ok()?;
    Some(FontData::from_owned(bytes))
}

fn with_fallback(mut fonts: FontDefinitions, font: Arc<FontData>) -> FontDefinitions {
    fonts.font_data.insert(FONT_NAME.to_owned(), font);
    for family in fonts.families.values_mut() {
        if !family.iter().any(|name| name == FONT_NAME) {
            family.push(FONT_NAME.to_owned());
        }
    }
    fonts
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use egui::{FontFamily, FontId};

    const FIXTURE: &[u8] = include_bytes!("../../../assets/fonts/DMSans-Regular.ttf");

    #[test]
    fn font_reads_are_bounded_and_reject_invalid_files() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("font.ttf");
        assert!(read_font(&path, MAX_FONT_BYTES).is_none());
        assert!(read_font(dir.path(), MAX_FONT_BYTES).is_none());
        std::fs::write(&path, b"not a font")?;
        assert!(read_font(&path, MAX_FONT_BYTES).is_none());
        std::fs::write(&path, FIXTURE)?;
        let length = u64::try_from(FIXTURE.len()).expect("fixture fits u64");
        assert!(read_font(&path, length - 1).is_none());
        assert!(read_font(&path, 0).is_none());
        assert!(read_font(&path, u64::MAX).is_none());
        let font = read_font(&path, length).expect("valid font at the exact limit");
        assert_eq!(font.font.as_ref(), FIXTURE);
        assert_eq!(font.index, 0);
        Ok(())
    }

    #[test]
    fn an_invalid_candidate_does_not_hide_a_usable_font() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(from_directory(dir.path()).is_none());
        std::fs::write(dir.path().join("msyh.ttc"), b"invalid collection")?;
        std::fs::write(dir.path().join("msjh.ttc"), FIXTURE)?;
        let font = from_directory(dir.path()).expect("the next valid candidate is used");
        assert_eq!(font.font.as_ref(), FIXTURE);
        Ok(())
    }

    #[test]
    fn every_family_keeps_its_primary_and_gets_one_fallback() {
        let mut fonts = FontDefinitions::default();
        let body = fonts.families.get(&FontFamily::Proportional).expect("default body family").clone();
        for name in ["medium", "semibold"] {
            fonts.families.insert(FontFamily::Name(name.into()), body.clone());
        }
        let original = fonts.families.clone();
        let font = Arc::new(FontData::from_static(FIXTURE));
        let fonts = with_fallback(with_fallback(fonts, Arc::clone(&font)), font);
        assert!(fonts.font_data.contains_key(FONT_NAME));
        for (family, primary) in original {
            let stack = fonts.families.get(&family).expect("family retained");
            assert!(stack.starts_with(&primary));
            assert_eq!(stack.iter().filter(|name| name.as_str() == FONT_NAME).count(), 1);
        }
    }

    /// A manual check on the actual Windows installation, with no window or network.
    #[test]
    #[ignore = "requires installed Windows CJK fonts"]
    fn installed_windows_fonts_render_chinese_in_every_family() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let families = [FontId::proportional(14.0), FontId::monospace(14.0), crate::theme::medium(14.0), crate::theme::semibold(14.0)];
        let text = "简体中文 繁體中文 项目 设置 工作台 你好，世界";
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
            if families.iter().all(|font| ctx.fonts_mut(|fonts| fonts.has_glyphs(font, text))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for font in families {
            assert!(ctx.fonts_mut(|fonts| fonts.has_glyphs(&font, text)), "Chinese glyphs are missing in {font:?}");
        }
    }
}
