//! **A render world**: the picture drawn from a copy of the game world, kept by extraction.
//!
//! [`RenderWorld`] owns a second [`World`] and the [`Extractor`] that keeps it a copy of what the
//! draw reads ([`render_extractor`]). A frame is the game path's three steps with the copy in the
//! middle: [`prepare_frame`](super::prepare_frame) on the game world — everything a frame writes
//! that the game can see —, the extractor bringing the copy up to date, [`draw_frame`](super::draw_frame)
//! on the copy, and the draw's cost published back to the game world. The draw is handed only the
//! copy, so whatever it does, the game world is not in reach — which is what lets it later run while
//! the game goes on.
//!
//! What the copy holds is decided in one place, [`render_extractor`], and held to the draw by a test
//! that reads the draw path's source: every component and resource the draw names is copied, or is
//! listed with the reason it need not be.

use super::{draw_frame, draw_viewports, prepare_frame, publish_render_stats};
use crate::core::World;
use crate::renderer::Renderer;
use gizmo_core::extract::Extractor;

/// A copy of the game world that the picture is drawn from.
///
/// It owns a second [`World`] and the [`Extractor`] that keeps it a copy of what the draw reads
/// ([`render_extractor`]). [`frame`](Self::frame) is the game path's three steps with the copy in
/// the middle: [`prepare_frame`](super::prepare_frame) on the game world, the extractor bringing
/// the copy up to date, [`draw_frame`](super::draw_frame) on the copy, and the draw's cost published
/// back to the game world. The draw is handed only the copy, so the game world is out of its reach
/// — which is what lets it later run while the game goes on.
///
/// ```no_run
/// use gizmo::prelude::*;
/// use gizmo::systems::render::RenderWorld;
///
/// let mut copy = RenderWorld::new();
/// App::<()>::new("Demo", 1280, 720)
///     .set_render(move |world, _state, encoder, view, renderer, _light_time| {
///         copy.frame(world, encoder, view, renderer);
///     })
///     .run()
///     .expect("the application failed to run");
/// ```
pub struct RenderWorld {
    world: World,
    extractor: Extractor,
}

impl Default for RenderWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for RenderWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderWorld").field("extractor", &self.extractor).finish_non_exhaustive()
    }
}

impl RenderWorld {
    /// An empty copy, kept by [`render_extractor`].
    pub fn new() -> Self {
        Self { world: World::new(), extractor: render_extractor() }
    }

    /// The extractor, to copy a type of the game's own that a pass of its own reads — a component
    /// a custom render hook queries in the copy.
    pub fn extractor_mut(&mut self) -> &mut Extractor {
        &mut self.extractor
    }

    /// The copy as the last frame left it.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Brings the copy up to date with `game` — after [`prepare_frame`](super::prepare_frame), for
    /// a host that draws the copy itself.
    pub fn extract(&mut self, game: &mut World) {
        self.extractor.run(game, &mut self.world);
    }

    /// One frame: prepare `game`, copy it, draw the copy into `view`, publish what the draw cost
    /// to `game`. The copy's counterpart of [`default_render_pass`](super::default_render_pass);
    /// split screen included, as there.
    pub fn frame(
        &mut self,
        game: &mut World,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        renderer: &mut Renderer,
    ) {
        prepare_frame(game, renderer);
        self.extract(game);
        draw_frame(&self.world, encoder, view, renderer);
        publish_render_stats(game, renderer);
    }

    /// [`frame`](Self::frame) through [`draw_viewports`](super::draw_viewports) — the copy's
    /// counterpart of [`render_viewports`](super::render_viewports).
    pub fn frame_viewports(
        &mut self,
        game: &mut World,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        renderer: &mut Renderer,
    ) {
        prepare_frame(game, renderer);
        self.extract(game);
        draw_viewports(&self.world, encoder, view, renderer);
        publish_render_stats(game, renderer);
    }
}

/// The extractor that copies everything the draw reads: the components and resources
/// [`draw_frame`](super::draw_frame) and [`draw_viewports`](super::draw_viewports) query, and the
/// mesh and material maps their handles resolve through.
///
/// Held to the draw by `render_world_tests::the_copy_holds_every_type_the_draw_reads`, which reads
/// the draw path's source for the types it names. A type of the game's own that a pass of its own
/// reads is added with [`RenderWorld::extractor_mut`].
pub fn render_extractor() -> Extractor {
    use crate::core::component::{Children, EditorOnly, IsHidden, Parent};
    use gizmo_physics_core::components::{GlobalTransform, Transform};
    use crate::renderer::components::{
        Camera, CameraShake, Decal, DirectionalLight, LodGroup, Material, Mesh, MeshRenderer, NoWireframe, Occluder,
        PlanarReflection, PointLight, PostProcess, RectLight, ReflectionProbe, Skeleton, SpotLight, Sprite, Text,
        Tilemap, Wireframe, WireframeColor,
    };
    use gizmo_core::asset::Handle;

    let mut extractor = Extractor::new();
    extractor
        // Where things are, and which are drawn at all.
        .component::<Transform>()
        .component::<GlobalTransform>()
        .component::<Children>()
        .component::<Parent>()
        .component::<IsHidden>()
        .component::<EditorOnly>()
        .component::<crate::core::pool::Pooled>()
        // Cameras and what they see through.
        .component::<Camera>()
        .component::<CameraShake>()
        .component::<PostProcess>()
        .component::<ReflectionProbe>()
        .component::<PlanarReflection>()
        // Geometry.
        .component::<Mesh>()
        .component::<Material>()
        .component::<MeshRenderer>()
        .component::<Handle<Mesh>>()
        .component::<Handle<Material>>()
        .assets::<Mesh>()
        .assets::<Material>()
        .component::<Skeleton>()
        .component::<LodGroup>()
        .component::<Occluder>()
        .component::<Wireframe>()
        .component::<NoWireframe>()
        .component::<WireframeColor>()
        .component::<Decal>()
        // Light.
        .component::<DirectionalLight>()
        .component::<PointLight>()
        .component::<SpotLight>()
        .component::<RectLight>()
        .component::<crate::renderer::irradiance::IrradianceVolume>()
        // 2D and text.
        .component::<Sprite>()
        .component::<Tilemap>()
        .component::<Text>()
        .component::<super::Typewriter>()
        .component::<super::LocalizedLabel>()
        // What the frame's preparation handed over, and settings the draw reads.
        .resource::<crate::core::time::Time>()
        .resource::<super::ParticleSpawns>()
        .resource::<crate::renderer::GizmoFrame>()
        .resource::<super::WireframeConfig>()
        .resource::<super::visibility_index::VisibilityIndex>()
        .resource::<crate::renderer::screen_fade::ScreenFade>()
        .resource::<crate::core::window::DisplayScale>();
    #[cfg(feature = "physics")]
    extractor.resource::<super::WaterVolumes>();
    #[cfg(feature = "ui")]
    extractor
        .component::<crate::ui::Node>()
        .component::<crate::ui::UiTransform>()
        .component::<crate::ui::ZIndex>()
        .component::<crate::ui::BackgroundColor>()
        .component::<crate::ui::ClipContent>()
        .component::<crate::ui::Scroll>()
        .component::<crate::ui::Slider>()
        .component::<crate::ui::VirtualStick>()
        .component::<crate::ui::ProgressBar>()
        .component::<crate::ui::Toggle>()
        .component::<crate::ui::TextInput>()
        .component::<crate::ui::Dropdown>()
        .component::<crate::ui::Tooltip>()
        .resource::<crate::ui::UiScale>();
    extractor
}

#[cfg(test)]
mod render_world_tests {
    use super::*;
    use gizmo_core::extract::Copied;
    use std::collections::BTreeSet;

    /// The calls through which code reads the world, whose type arguments name what it reads.
    const READS: &[&str] =
        &["query", "query_filtered", "get", "get_resource", "try_get_resource", "get_resource_mut", "try_get_resource_mut"];

    /// Words inside a read's type arguments that are not things in the world.
    const NOT_DATA: &[&str] = &["Entity", "With", "Without", "Changed", "Added", "Option", "Vec"];

    /// Read in these files by the frame's **preparation** only, which hands the draw what it needs
    /// of them in another form — each with that form.
    const PREPARED: &[(&str, &str)] = &[
        ("Gizmos", "taken into the `GizmoFrame` resource by `end_debug_line_frame`"),
        ("ParticleEmitter", "stepped by `step_particle_emitters`; the draw reads `ParticleSpawns`"),
        ("PhysicsWorld", "its fluid zones handed over as `WaterVolumes`"),
        ("LocalizedText", "resolved into `LocalizedLabel` by `resolve_localized_texts`"),
        ("Locale", "read by `resolve_localized_texts`, which records missing keys in the game's locale"),
    ];

    /// Every `.rs` file under `dir`, the files that are test modules as a whole left out and each
    /// other one cut at its `#[cfg(test)]`.
    fn draw_path_source(dir: &std::path::Path) -> String {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(dir, &mut files);
        let sources: Vec<_> = files.iter().map(|f| (f, std::fs::read_to_string(f).unwrap_or_default())).collect();
        let test_files: BTreeSet<String> = sources
            .iter()
            .flat_map(|(_, src)| {
                src.match_indices("#[cfg(test)]")
                    .filter_map(|(i, a)| {
                        let name = src[i + a.len()..].trim_start().strip_prefix("mod ")?.split(';').next()?.trim();
                        name.chars().all(|c| c.is_alphanumeric() || c == '_').then(|| name.to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        sources
            .iter()
            .filter(|(f, _)| f.file_stem().and_then(|s| s.to_str()).is_none_or(|s| !test_files.contains(s)))
            .map(|(_, src)| src.find("#[cfg(test)]").map_or(src.as_str(), |i| &src[..i]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The capitalised names in the type arguments of every read in `src`.
    fn names_read(src: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for (at, _) in src.match_indices("::<") {
            let call: String =
                src[..at].chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>().chars().rev().collect();
            if !READS.contains(&call.as_str()) {
                continue;
            }
            let mut depth = 1;
            let args: String = src[at + 3..]
                .chars()
                .take_while(|c| {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        _ => {}
                    }
                    depth > 0
                })
                .collect();
            for word in args.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                if word.starts_with(|c: char| c.is_ascii_uppercase()) && !NOT_DATA.contains(&word) {
                    names.insert(word.to_string());
                }
            }
        }
        names
    }

    fn last_segments(type_name: &str) -> impl Iterator<Item = &str> {
        type_name
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .filter_map(|path| path.rsplit("::").next())
            .filter(|w| w.starts_with(|c: char| c.is_ascii_uppercase()))
    }

    /// What a render thread will carry across and back: the copy, the renderer that draws it and
    /// the game side's GPU handle. A static assertion — a field that stops one being `Send` fails
    /// to compile here, not in the threaded loop that would first need it.
    #[test]
    fn the_copy_and_the_renderer_can_go_to_another_thread() {
        fn send<T: Send>() {}
        send::<World>();
        send::<RenderWorld>();
        send::<Renderer>();
        send::<crate::renderer::GpuContext>();
    }

    /// **The copy holds every type the draw reads.** The draw path's source (`systems/render`,
    /// tests left out) is read for the types named in its `query` / `get` / `get_resource` calls;
    /// each must be copied by [`render_extractor`] or be one the preparation hands over in another
    /// form ([`PREPARED`]). A type the draw starts reading and nobody adds here fails this test
    /// rather than vanishing from every picture drawn from a copy.
    #[test]
    fn the_copy_holds_every_type_the_draw_reads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/systems/render");
        let read = names_read(&draw_path_source(&dir));
        assert!(read.len() > 30, "the scan found only {read:?} — it is reading the wrong thing");

        let mut copied = BTreeSet::new();
        for c in render_extractor().copies() {
            let name = match c {
                Copied::Component(n) | Copied::Resource(n) => n,
                Copied::Assets(n) => {
                    copied.insert("Assets".to_string());
                    n
                }
                _ => continue,
            };
            copied.extend(last_segments(name).map(str::to_string));
        }
        let prepared: BTreeSet<String> = PREPARED.iter().map(|(n, _)| n.to_string()).collect();

        let missing: Vec<&String> = read.iter().filter(|n| !copied.contains(*n) && !prepared.contains(*n)).collect();
        assert!(missing.is_empty(), "the draw reads these and the render world does not copy them: {missing:?}");
        let stale: Vec<&String> =
            prepared.iter().filter(|n| !read.contains(*n) && !copied.contains(*n)).collect();
        assert!(stale.is_empty(), "PREPARED lists types the draw path no longer names: {stale:?}");
    }
}
