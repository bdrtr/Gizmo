//! A pass's draws recorded as a plan first, then replayed into the pass with every run of draws
//! that share what is bound merged into one indirect multi-draw.
//!
//! wgpu validates and tracks each command a render pass records, on the CPU, when the frame's
//! commands are finished — per `set_pipeline`, per `set_bind_group`, per `set_vertex_buffer`, per
//! draw. A city merged into thousands of meshes draws thousands of times a pass, and that
//! bookkeeping was as large as recording the frame itself (`nfsu2-gizmo` MOTOR-NOTLARI #29).
//!
//! A [`PassPlan`] takes the same calls a pass would, drops the ones that set again what is already
//! set, and on [`replay`](PassPlan::replay) turns each run of two or more draws with nothing set
//! between them into one `multi_draw_indirect` / `multi_draw_indexed_indirect`, reading their
//! arguments from a buffer made for the pass. Meshes in a shared buffer
//! ([`MeshArena`](crate::renderer::mesh_arena::MeshArena)), drawn in the order the batcher puts the
//! opaque world in (by pipeline, then material, then buffer), make such runs: a run per material.
//!
//! An indirect draw that starts past instance 0 needs `INDIRECT_FIRST_INSTANCE`; without it — or
//! without indirect execution at all — the plan replays every draw as it came, which is the frame
//! the pass recorded before plans existed.

use std::ops::Range;

enum Cmd<'a> {
    Pipeline(&'a wgpu::RenderPipeline),
    BindGroup(u32, &'a wgpu::BindGroup),
    Vertex(&'a wgpu::Buffer),
    Index(&'a wgpu::Buffer, wgpu::IndexFormat),
    Draw { vertices: Range<u32>, instances: Range<u32> },
    DrawIndexed { indices: Range<u32>, base_vertex: i32, instances: Range<u32> },
}

/// Bind group slots a plan keeps track of.
const GROUPS: usize = 8;

/// A pass's draws, waiting to be [replayed](Self::replay) — see the [module](self).
#[derive(Default)]
pub(crate) struct PassPlan<'a> {
    cmds: Vec<Cmd<'a>>,
    pipeline: Option<*const wgpu::RenderPipeline>,
    groups: [Option<*const wgpu::BindGroup>; GROUPS],
    vertex: Option<*const wgpu::Buffer>,
    index: Option<(*const wgpu::Buffer, wgpu::IndexFormat)>,
}

/// What [`PassPlan::replay`] did, for tests and for the frame's statistics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Replayed {
    /// Draws the plan was given.
    pub(crate) draws: u32,
    /// Draw commands the pass recorded for them: one per multi-draw, one per draw left alone.
    pub(crate) commands: u32,
}

impl<'a> PassPlan<'a> {
    pub(crate) fn set_pipeline(&mut self, pipeline: &'a wgpu::RenderPipeline) {
        let at = pipeline as *const _;
        if self.pipeline != Some(at) {
            self.pipeline = Some(at);
            self.cmds.push(Cmd::Pipeline(pipeline));
        }
    }

    pub(crate) fn set_bind_group(&mut self, index: u32, group: &'a wgpu::BindGroup) {
        let at = group as *const _;
        let slot = &mut self.groups[index as usize];
        if *slot != Some(at) {
            *slot = Some(at);
            self.cmds.push(Cmd::BindGroup(index, group));
        }
    }

    pub(crate) fn set_vertex_buffer(&mut self, buffer: &'a wgpu::Buffer) {
        let at = buffer as *const _;
        if self.vertex != Some(at) {
            self.vertex = Some(at);
            self.cmds.push(Cmd::Vertex(buffer));
        }
    }

    pub(crate) fn set_index_buffer(&mut self, buffer: &'a wgpu::Buffer, format: wgpu::IndexFormat) {
        let at = (buffer as *const _, format);
        if self.index != Some(at) {
            self.index = Some(at);
            self.cmds.push(Cmd::Index(buffer, format));
        }
    }

    pub(crate) fn draw(&mut self, vertices: Range<u32>, instances: Range<u32>) {
        if !vertices.is_empty() && !instances.is_empty() {
            self.cmds.push(Cmd::Draw { vertices, instances });
        }
    }

    pub(crate) fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        if !indices.is_empty() && !instances.is_empty() {
            self.cmds.push(Cmd::DrawIndexed { indices, base_vertex, instances });
        }
    }

    /// Records the plan into `pass`: what it set, in order, and its draws — each run of two or
    /// more with nothing set between them as one multi-draw when `multi` (the device has indirect
    /// execution and `INDIRECT_FIRST_INSTANCE`), every one on its own otherwise.
    /// The runs `replay` merges: two or more draws of one kind (indexed or not) with nothing set
    /// between them. Their arguments, packed as the indirect draw structs, and per command where a
    /// run starts — `Some((byte offset into the arguments, draws))`. Nothing when `!multi`.
    fn runs(&self, multi: bool) -> (Vec<u32>, Vec<Option<(u64, u32)>>) {
        let mut args: Vec<u32> = Vec::new();
        let mut runs: Vec<Option<(u64, u32)>> = Vec::new();
        if !multi {
            return (args, runs);
        }
        runs.resize(self.cmds.len(), None);
        let mut i = 0;
        while i < self.cmds.len() {
            let indexed = match self.cmds[i] {
                Cmd::Draw { .. } => false,
                Cmd::DrawIndexed { .. } => true,
                _ => {
                    i += 1;
                    continue;
                }
            };
            let mut end = i + 1;
            while end < self.cmds.len()
                && matches!((&self.cmds[end], indexed), (Cmd::Draw { .. }, false) | (Cmd::DrawIndexed { .. }, true))
            {
                end += 1;
            }
            if end - i >= 2 {
                let offset = (args.len() * 4) as u64;
                for cmd in &self.cmds[i..end] {
                    match cmd {
                        Cmd::Draw { vertices, instances } => args.extend([
                            vertices.end - vertices.start,
                            instances.end - instances.start,
                            vertices.start,
                            instances.start,
                        ]),
                        Cmd::DrawIndexed { indices, base_vertex, instances } => args.extend([
                            indices.end - indices.start,
                            instances.end - instances.start,
                            indices.start,
                            *base_vertex as u32,
                            instances.start,
                        ]),
                        _ => unreachable!("a run is draws only"),
                    }
                }
                runs[i] = Some((offset, (end - i) as u32));
            }
            i = end;
        }
        (args, runs)
    }

    /// Records the plan into `pass`: what it set, in order, and its draws — each run of two or
    /// more with nothing set between them as one multi-draw when `multi` (the device has indirect
    /// execution and `INDIRECT_FIRST_INSTANCE`), every one on its own otherwise.
    pub(crate) fn replay(self, pass: &mut wgpu::RenderPass<'_>, device: &wgpu::Device, multi: bool) -> Replayed {
        use wgpu::util::DeviceExt;
        // The runs, found first: their arguments go into one buffer, which has to exist before
        // the first of them is recorded.
        let (args, runs) = self.runs(multi);
        let indirect = (!args.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pass plan indirect"),
                contents: bytemuck::cast_slice(&args),
                usage: wgpu::BufferUsages::INDIRECT,
            })
        });

        let mut replayed = Replayed::default();
        let mut i = 0;
        while i < self.cmds.len() {
            if let (Some(Some((offset, count))), Some(buffer)) = (runs.get(i), indirect.as_ref()) {
                match self.cmds[i] {
                    Cmd::DrawIndexed { .. } => pass.multi_draw_indexed_indirect(buffer, *offset, *count),
                    _ => pass.multi_draw_indirect(buffer, *offset, *count),
                }
                replayed.draws += count;
                replayed.commands += 1;
                i += *count as usize;
                continue;
            }
            match &self.cmds[i] {
                Cmd::Pipeline(p) => pass.set_pipeline(p),
                Cmd::BindGroup(index, group) => pass.set_bind_group(*index, *group, &[]),
                Cmd::Vertex(buffer) => pass.set_vertex_buffer(0, buffer.slice(..)),
                Cmd::Index(buffer, format) => pass.set_index_buffer(buffer.slice(..), *format),
                Cmd::Draw { vertices, instances } => {
                    pass.draw(vertices.clone(), instances.clone());
                    replayed.draws += 1;
                    replayed.commands += 1;
                }
                Cmd::DrawIndexed { indices, base_vertex, instances } => {
                    pass.draw_indexed(indices.clone(), *base_vertex, instances.clone());
                    replayed.draws += 1;
                    replayed.commands += 1;
                }
            }
            i += 1;
        }
        replayed
    }
}

#[cfg(test)]
mod tests {
    use super::{Cmd, PassPlan};

    fn noop() -> wgpu::Device {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device").0
    }

    /// What is set again unchanged records nothing, so draws sharing what is bound stay next to each
    /// other; a run of two or more of one kind is merged, its arguments packed as wgpu's indirect
    /// structs; anything set between draws ends a run, and so does a change of kind.
    #[test]
    fn a_plan_merges_runs_of_draws_with_nothing_set_between() {
        let device = noop();
        let buffer = |usage| device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 64, usage, mapped_at_creation: false });
        let (vertices, other_vertices, indices) = (buffer(wgpu::BufferUsages::VERTEX), buffer(wgpu::BufferUsages::VERTEX), buffer(wgpu::BufferUsages::INDEX));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[] });
        let group = |_: u8| device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[] });
        let (red, green) = (group(0), group(1));

        let mut plan = PassPlan::default();
        plan.set_bind_group(1, &red);
        plan.set_vertex_buffer(&vertices);
        plan.draw(0..3, 0..1);
        plan.set_bind_group(1, &red); // the same: nothing recorded
        plan.set_vertex_buffer(&vertices); // the same
        plan.draw(3..9, 1..3);
        plan.draw(9..12, 3..4);
        plan.set_bind_group(1, &green); // a change ends the run
        plan.draw(12..15, 4..5);
        plan.set_vertex_buffer(&other_vertices);
        plan.set_index_buffer(&indices, wgpu::IndexFormat::Uint32);
        plan.draw_indexed(0..6, 10, 5..6);
        plan.draw_indexed(6..9, 20, 6..8);
        plan.draw(0..3, 8..9); // a change of kind ends it too
        plan.draw(0..0, 0..1); // empty: not a draw
        let kinds: Vec<&str> = plan
            .cmds
            .iter()
            .map(|c| match c {
                Cmd::Pipeline(_) => "pipeline",
                Cmd::BindGroup(..) => "group",
                Cmd::Vertex(_) => "vertex",
                Cmd::Index(..) => "index",
                Cmd::Draw { .. } => "draw",
                Cmd::DrawIndexed { .. } => "indexed",
            })
            .collect();
        assert_eq!(
            kinds,
            ["group", "vertex", "draw", "draw", "draw", "group", "draw", "vertex", "index", "indexed", "indexed", "draw"],
        );
        let (args, runs) = plan.runs(true);
        assert_eq!(runs[2], Some((0, 3)), "three draws, one run");
        assert_eq!(runs[9], Some((48, 2)), "two indexed draws, after the three four-word ones");
        assert_eq!(runs.iter().flatten().count(), 2, "a single draw is left alone");
        assert_eq!(
            args,
            [3, 1, 0, 0, 6, 2, 3, 1, 3, 1, 9, 3, 6, 1, 0, 10, 5, 3, 2, 6, 20, 6],
            "(count, instances, first, first instance) and (indices, instances, first index, base vertex, first instance)"
        );
        let (args, runs) = plan.runs(false);
        assert!(args.is_empty() && runs.is_empty(), "nothing is merged without multi-draw");
    }
}
