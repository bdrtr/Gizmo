//! glTF animation parsing — keyframe/channel extraction into `AnimationClip`s.
//! Extracted verbatim from `loaders.rs` (pure move). Called from `load_gltf_from_import`.

use super::*;

/// Build a keyframe list from a glTF channel's raw outputs, preserving cubic tangents.
///
/// For `CUBICSPLINE` the raw output stores three entries per sample —
/// `[inTangent, value, outTangent]` — so we pick the value (offset 1) and keep both
/// tangents so the sampler can do true cubic-Hermite instead of downgrading to linear.
/// For Linear/Step there is one entry per sample and no tangents.
fn build_keyframes<V, T>(
    times: &[f32],
    vals: &[V],
    cubic: bool,
    conv: impl Fn(&V) -> T,
) -> Vec<Keyframe<T>> {
    let (stride, off) = if cubic { (3usize, 1usize) } else { (1usize, 0usize) };
    times
        .iter()
        .enumerate()
        .filter_map(|(i, &t)| {
            let base = i * stride;
            let value = conv(vals.get(base + off)?);
            if cubic {
                match (vals.get(base), vals.get(base + 2)) {
                    (Some(a), Some(b)) => {
                        Some(Keyframe::with_tangents(t, value, conv(a), conv(b)))
                    }
                    // Malformed cubic block → keep the value, sampler falls back to lerp.
                    _ => Some(Keyframe::new(t, value)),
                }
            } else {
                Some(Keyframe::new(t, value))
            }
        })
        .collect()
}

pub(super) fn parse_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Vec<AnimationClip> {
    document
        .animations()
        .map(|anim| {
            let mut translations = Vec::new();
            let mut rotations = Vec::new();
            let mut scales = Vec::new();

            for channel in anim.channels() {
                let target_node = channel.target().node().index();
                let target_node_name = channel.target().node().name().map(str::to_owned);
                let reader = channel.reader(|b| Some(&buffers[b.index()]));

                let times: Vec<f32> = match reader.read_inputs() {
                    Some(it) => it.collect(),
                    None => continue,
                };

                let interp = match channel.sampler().interpolation() {
                    gltf::animation::Interpolation::Step => {
                        gizmo_animation::skeletal::InterpolationMode::Step
                    }
                    gltf::animation::Interpolation::CubicSpline => {
                        gizmo_animation::skeletal::InterpolationMode::CubicSpline
                    }
                    _ => gizmo_animation::skeletal::InterpolationMode::Linear,
                };

                let outputs = match reader.read_outputs() {
                    Some(o) => o,
                    None => continue,
                };

                match outputs {
                    gltf::animation::util::ReadOutputs::Translations(tr) => {
                        let vals: Vec<[f32; 3]> = tr.collect();
                        let cubic = matches!(interp, gizmo_animation::skeletal::InterpolationMode::CubicSpline);
                        let keyframes = build_keyframes(&times, &vals, cubic, |v| Vec3::new(v[0], v[1], v[2]));
                        translations.push(Track {
                            target_node,
                            target_node_name: target_node_name.clone(),
                            interpolation: interp,
                            keyframes,
                        });
                    }
                    gltf::animation::util::ReadOutputs::Rotations(rt) => {
                        let vals: Vec<[f32; 4]> = rt.into_f32().collect();
                        let cubic = matches!(interp, gizmo_animation::skeletal::InterpolationMode::CubicSpline);
                        let keyframes = build_keyframes(&times, &vals, cubic, |v| Quat::from_xyzw(v[0], v[1], v[2], v[3]));
                        rotations.push(Track {
                            target_node,
                            target_node_name: target_node_name.clone(),
                            interpolation: interp,
                            keyframes,
                        });
                    }
                    gltf::animation::util::ReadOutputs::Scales(sc) => {
                        let vals: Vec<[f32; 3]> = sc.collect();
                        let cubic = matches!(interp, gizmo_animation::skeletal::InterpolationMode::CubicSpline);
                        let keyframes = build_keyframes(&times, &vals, cubic, |v| Vec3::new(v[0], v[1], v[2]));
                        scales.push(Track {
                            target_node,
                            target_node_name,
                            interpolation: interp,
                            keyframes,
                        });
                    }
                    _ => {} // Morph targets and other outputs are intentionally ignored.
                }
            }

            // Duration = time of the last keyframe across all tracks.
            let d_tr = translations
                .iter()
                .filter_map(|t| t.keyframes.last().map(|k| k.time))
                .fold(0.0f32, f32::max);
            let d_rot = rotations
                .iter()
                .filter_map(|t| t.keyframes.last().map(|k| k.time))
                .fold(0.0f32, f32::max);
            let d_scl = scales
                .iter()
                .filter_map(|t| t.keyframes.last().map(|k| k.time))
                .fold(0.0f32, f32::max);
            let duration = d_tr.max(d_rot).max(d_scl);

            AnimationClip {
                name: anim.name().unwrap_or("unnamed").to_string(),
                duration,
                translations,
                rotations,
                scales,
            }
        })
        .collect()
}
