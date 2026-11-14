//! [`FluidState`] — the water itself, saved with a scene.

use super::types::FluidParticle;
use std::sync::Arc;

/// **The scene's water, as it stands: where every particle is, how fast it moves, what phase it
/// is.** It sits beside the scene's `FluidSimulation`, put there by `gizmo`'s frame preparation,
/// and a scene saves it — so a reopened scene's water goes on from where it was rather than being
/// laid out afresh.
///
/// The particles live on the GPU, and copying a hundred thousand of them back every frame to keep
/// a copy here would cost more than the frame. So the component holds a link to the solver's
/// buffer instead, and **reads the GPU at the moment it is saved** — a save, or the editor's ▶
/// snapshot, waits for the copy (the size of the water: 100 000 particles are about 2.8 MB of
/// numbers, about 7 MB as scene text). What a reopened scene brings is handed back to the solver
/// by the frame preparation, after the settings, and the link is made again.
///
/// Not editable: it is the solver's state, not a dial. A `FluidSimulation` edit that lays the water
/// out again (a new box, particle size or count) replaces it.
#[derive(Clone, Default)]
pub struct FluidState {
    link: Option<FluidLink>,
    saved: Option<Arc<SavedFluid>>,
}

/// Where the water is on the GPU.
#[derive(Clone)]
struct FluidLink {
    device: wgpu::Device,
    queue: wgpu::Queue,
    buffer: wgpu::Buffer,
    count: u32,
}

/// The water as a scene file holds it: one list per channel, the phases left out while every
/// particle is plain water.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct SavedFluid {
    positions: Vec<[f32; 3]>,
    velocities: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    phases: Vec<u32>,
}

impl SavedFluid {
    fn of(particles: &[FluidParticle]) -> Self {
        let phases: Vec<u32> = particles.iter().map(|p| p.phase).collect();
        Self {
            positions: particles.iter().map(|p| p.position).collect(),
            velocities: particles.iter().map(|p| p.velocity).collect(),
            phases: if phases.iter().all(|&p| p == 0) { Vec::new() } else { phases },
        }
    }

    fn particles(&self, rest_density: f32) -> Vec<FluidParticle> {
        self.positions
            .iter()
            .enumerate()
            .map(|(i, &position)| FluidParticle {
                position,
                density: rest_density,
                velocity: self.velocities.get(i).copied().unwrap_or([0.0; 3]),
                lambda: 0.0,
                predicted_position: position,
                phase: self.phases.get(i).copied().unwrap_or(0),
                vorticity: [0.0; 3],
                _pad_vort: 0.0,
            })
            .collect()
    }
}

impl FluidState {
    /// Links it to the solver's particles: `count` of them, from the start of `buffer`.
    pub fn link(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer, count: u32) {
        let same = self.link.as_ref().is_some_and(|l| l.count == count && l.buffer == *buffer);
        if !same {
            self.link = Some(FluidLink { device: device.clone(), queue: queue.clone(), buffer: buffer.clone(), count });
        }
    }

    /// The water a scene file brought, taken once — to be given to the solver — as particles at
    /// `rest_density`. `None` when there is none waiting.
    pub fn take_saved(&mut self, rest_density: f32) -> Option<Vec<FluidParticle>> {
        self.saved.take().map(|s| s.particles(rest_density))
    }

    /// Whether water from a scene file is waiting to be given to the solver.
    pub fn has_saved(&self) -> bool {
        self.saved.is_some()
    }

    /// The water now: read from the GPU when linked, else what a file brought.
    fn current(&self) -> Result<Option<Arc<SavedFluid>>, String> {
        match &self.link {
            Some(l) => {
                let size = l.count as u64 * std::mem::size_of::<FluidParticle>() as u64;
                if size == 0 {
                    return Ok(Some(Arc::new(SavedFluid::of(&[]))));
                }
                let bytes = super::system::read_buffer(&l.device, &l.queue, &l.buffer, size)
                    .ok_or_else(|| "the fluid's particles could not be read back from the GPU".to_string())?;
                Ok(Some(Arc::new(SavedFluid::of(bytemuck::cast_slice(&bytes)))))
            }
            None => Ok(self.saved.clone()),
        }
    }
}

impl serde::Serialize for FluidState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.current().map_err(serde::ser::Error::custom)? {
            Some(water) => water.serialize(serializer),
            None => SavedFluid::of(&[]).serialize(serializer),
        }
    }
}

impl<'de> serde::Deserialize<'de> for FluidState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = SavedFluid::deserialize(deserializer)?;
        Ok(Self { link: None, saved: (!saved.positions.is_empty()).then(|| Arc::new(saved)) })
    }
}

impl std::fmt::Debug for FluidState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FluidState")
            .field("linked", &self.link.as_ref().map(|l| l.count))
            .field("saved", &self.saved.as_ref().map(|s| s.positions.len()))
            .finish()
    }
}

gizmo_core::impl_component!(FluidState);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_fluid_comes_back_as_the_particles_it_was() {
        let ps: Vec<FluidParticle> = (0..5)
            .map(|i| FluidParticle {
                position: [i as f32, 0.5, -1.0],
                density: 0.0,
                velocity: [0.0, -(i as f32), 0.25],
                lambda: 0.0,
                predicted_position: [i as f32, 0.5, -1.0],
                phase: if i == 3 { 2 } else { 0 },
                vorticity: [0.0; 3],
                _pad_vort: 0.0,
            })
            .collect();
        let text = ron::to_string(&SavedFluid::of(&ps)).unwrap();
        let mut state: FluidState = ron::from_str(&text).unwrap();
        assert!(state.has_saved());
        let back = state.take_saved(1000.0).unwrap();
        assert_eq!(back.len(), 5);
        for (a, b) in ps.iter().zip(&back) {
            assert_eq!((a.position, a.velocity, a.phase), (b.position, b.velocity, b.phase));
        }
        assert!(!state.has_saved(), "the saved water is given once");
        let plain = ron::to_string(&SavedFluid::of(&ps[..2])).unwrap();
        assert!(!plain.contains("phases"), "all-water phases are left out: {plain}");
    }
}
