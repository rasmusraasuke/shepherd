use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{LazyLock, Mutex};

use eframe::egui::FontData;
use fontique::{
    Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, GenericFamily,
    QueryFamily, QueryStatus, SourceCache, SourceCacheOptions,
};

type Fonts = Mutex<(Collection, SourceCache)>;

pub fn font(families: &[GenericFamily], bold: bool, italic: bool) -> Option<FontData> {
    static FONTS: LazyLock<Option<Fonts>> = LazyLock::new(|| {
        catch_unwind(AssertUnwindSafe(|| {
            Mutex::new((
                Collection::new(CollectionOptions {
                    shared: false,
                    system_fonts: true,
                }),
                SourceCache::new(SourceCacheOptions::default()),
            ))
        }))
        .ok()
    });

    let mut fonts = FONTS.as_ref()?.lock().ok()?;
    let (collection, cache) = &mut *fonts;
    let mut query = collection.query(cache);
    query.set_families(families.iter().map(|family| QueryFamily::Generic(*family)));
    query.set_attributes(Attributes::new(
        FontWidth::NORMAL,
        if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
    ));
    let mut found = None;
    query.matches_with(|font| {
        let (blob, _) = font.blob.clone().into_raw_parts();
        found = Some(FontData::from_blob(blob, font.index));
        QueryStatus::Stop
    });
    found
}
