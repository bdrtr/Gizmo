//! A `Text`'s face, kept in step with its name so a scene can hold it.
//!
//! A [`FontId`] is an index into this run's font library and means nothing in a file; the face's
//! name ([`Renderer::font_name`](crate::Renderer::font_name): `"default"`, a path, or a name it was
//! loaded under) does. [`sync_text_fonts`] keeps [`Text::font_name`] and each span's name naming the
//! face it draws in, and gives a text read from a scene file — whose ids are
//! [`FontId::UNRESOLVED`] — its faces back by name. Whichever side changed since it last looked is
//! the one followed: a game that sets a new face renames the text, a name changed in a file or an
//! editor finds its face.

use gizmo_core::{Entity, World};

use crate::components::Text;
use crate::text::FontId;

/// The faces [`sync_text_fonts`] last left a text with: the text's, then each span's.
#[derive(Clone)]
struct SyncedFonts {
    font: FontId,
    spans: Vec<Option<FontId>>,
}
impl gizmo_core::component::Component for SyncedFonts {}

/// One face and its name brought in line. `synced` is the face this pass last left there, `None` the
/// first time it looks. Returns the face and the name to keep.
fn sync_one(
    renderer: &mut crate::Renderer,
    font: Option<FontId>,
    name: Option<String>,
    synced: Option<Option<FontId>>,
) -> (Option<FontId>, Option<String>) {
    let unresolved = font == Some(FontId::UNRESOLVED) || (font.is_none() && name.is_some());
    let face_changed = synced.is_none_or(|s| s != font);
    // Compared without allocating: this runs for every text, every frame.
    let name_matches = || {
        let current = font.and_then(|f| renderer.text.as_ref().and_then(|t| t.library().name_of(f)));
        current == name.as_deref()
    };
    if unresolved || (!face_changed && name.is_some() && !name_matches()) {
        // Read from a file, or a name set by hand: the name decides the face.
        let Some(n) = name.as_deref() else { return (font, name) };
        return match renderer.font_by_name(n) {
            Ok(id) => (Some(id), name),
            Err(error) => {
                tracing::warn!(font = n, %error, "[Text] the face a scene names would not load; the text draws nothing until it does");
                (font, name)
            }
        };
    }
    if face_changed {
        // A face set by the game: the name follows it.
        return (font, font.and_then(|f| renderer.font_name(f)));
    }
    (font, name)
}

/// Keeps every `Text`'s face and its name in step — see the [module docs](self).
pub fn sync_text_fonts(world: &mut World, renderer: &mut crate::Renderer) {
    let texts: Vec<(Entity, Text, Option<SyncedFonts>)> = {
        let synced = world.query::<&SyncedFonts>();
        world.query::<(Entity, &Text)>().iter().map(|(e, t)| (e, t.clone(), synced.get(e).cloned())).collect()
    };
    for (entity, mut text, synced) in texts {
        let (font, name) = sync_one(renderer, Some(text.font), text.font_name.clone(), synced.as_ref().map(|s| Some(s.font)));
        let mut changed = font != Some(text.font) || name != text.font_name;
        text.font = font.unwrap_or(text.font);
        text.font_name = name;
        let mut span_fonts = Vec::with_capacity(text.spans.len());
        for (i, span) in text.spans.iter_mut().enumerate() {
            let before = synced.as_ref().map(|s| s.spans.get(i).copied().flatten());
            let (f, n) = sync_one(renderer, span.font, span.font_name.clone(), before);
            changed |= f != span.font || n != span.font_name;
            span.font = f;
            span.font_name = n;
            span_fonts.push(span.font);
        }
        let now = SyncedFonts { font: text.font, spans: span_fonts };
        let resync = synced.as_ref().is_none_or(|s| s.font != now.font || s.spans != now.spans);
        if changed {
            if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(entity) {
                *t = text;
            }
        }
        if resync {
            world.add_component(entity, now);
        }
    }
}

