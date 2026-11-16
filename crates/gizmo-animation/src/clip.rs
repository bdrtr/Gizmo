use gizmo_math::{Quat, Vec2, Vec3, Vec4};

use crate::hermite::{hermite, hermite_quat, hermite_vec3};

/// Error returned when constructing a [`Track`] with inconsistent keyframe data.
///
/// A well-formed track requires its `keyframe_timestamps` to match the keyframe
/// values one-to-one, to be sorted ascending, and to contain only finite values.
/// Violating any of these invariants would otherwise lead to out-of-bounds
/// indexing or `NaN`-poisoned interpolation at sample time, so [`Track::new`]
/// rejects such data up front.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackError {
    /// The number of keyframe timestamps did not match the number of keyframe
    /// values.
    LengthMismatch {
        /// Number of supplied timestamps.
        timestamps: usize,
        /// Number of supplied keyframe values.
        values: usize,
    },
    /// A timestamp was `NaN` or infinite.
    NonFiniteTimestamp {
        /// Index of the offending timestamp.
        index: usize,
    },
    /// Timestamps were not sorted in ascending order.
    UnsortedTimestamps {
        /// Index of the timestamp that was smaller than its predecessor.
        index: usize,
    },
    /// The keyframes are of the wrong kind for the track: a transform track ([`Track::new`])
    /// was given value keyframes, or a property track ([`Track::property`]) was given
    /// translation/rotation/scale ones.
    ChannelMismatch {
        /// The property the track was to animate, `None` for a transform track.
        property: Option<String>,
    },
    /// A property track's name is not `"Component.field"` (optionally `"Component.field.lane"`):
    /// it has no dot, or an empty part.
    InvalidPropertyName {
        /// The name as given.
        name: String,
    },
}

impl std::fmt::Display for TrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackError::LengthMismatch { timestamps, values } => write!(
                f,
                "keyframe timestamp count ({timestamps}) does not match keyframe value count ({values})"
            ),
            TrackError::NonFiniteTimestamp { index } => {
                write!(f, "keyframe timestamp at index {index} is not finite")
            }
            TrackError::UnsortedTimestamps { index } => write!(
                f,
                "keyframe timestamp at index {index} is not in ascending order"
            ),
            TrackError::ChannelMismatch { property: None } => {
                write!(f, "a transform track takes translation, rotation or scale keyframes, not value keyframes")
            }
            TrackError::ChannelMismatch { property: Some(p) } => write!(
                f,
                "property track `{p}` takes scalar or vector keyframes, not translation, rotation or scale ones"
            ),
            TrackError::InvalidPropertyName { name } => {
                write!(f, "property name `{name}` is not of the form `Component.field`")
            }
        }
    }
}

impl std::error::Error for TrackError {}

/// What a value channel holds: the kind of a property's value, and of the [`Keyframes`] that
/// animate it. The transform channels (translation, rotation, scale) have no kind — they are
/// not properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PropertyKind {
    /// One `f32`.
    Scalar,
    /// A `Vec2`.
    Vector2,
    /// A `Vec3`.
    Vector3,
    /// A `Vec4` (a colour with alpha, say).
    Vector4,
}

impl std::fmt::Display for PropertyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PropertyKind::Scalar => "scalar",
            PropertyKind::Vector2 => "vector2",
            PropertyKind::Vector3 => "vector3",
            PropertyKind::Vector4 => "vector4",
        })
    }
}

/// The keyframe data for a [`Track`]: the transform channels, and the value channels a
/// property track animates.
///
/// Each vector is parallel to [`Track::keyframe_timestamps`]. Translation, rotation and scale
/// are written to the target's `Transform`; scale is a first-class channel, sampled and
/// applied just like the other two. `Scalar` … `Vector4` belong to a property track
/// ([`Track::property`]), whose value goes to the component field it names.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Keyframes {
    /// Position keyframes (linearly interpolated).
    Translation(Vec<Vec3>),
    /// Rotation keyframes (spherically interpolated).
    Rotation(Vec<Quat>),
    /// Scale keyframes (linearly interpolated).
    Scale(Vec<Vec3>),
    /// One-number property keyframes (linearly interpolated).
    Scalar(Vec<f32>),
    /// Two-number property keyframes (linearly interpolated).
    Vector2(Vec<Vec2>),
    /// Three-number property keyframes (linearly interpolated).
    Vector3(Vec<Vec3>),
    /// Four-number property keyframes — a colour, say (linearly interpolated).
    Vector4(Vec<Vec4>),
}

impl Keyframes {
    /// The value at `index`, as a sample of this channel would give it.
    #[must_use]
    pub fn value(&self, index: usize) -> InterpolatedValue {
        match self {
            Keyframes::Translation(v) => v.get(index).map_or(InterpolatedValue::None, |x| InterpolatedValue::Translation(*x)),
            Keyframes::Rotation(v) => v.get(index).map_or(InterpolatedValue::None, |x| InterpolatedValue::Rotation(*x)),
            Keyframes::Scale(v) => v.get(index).map_or(InterpolatedValue::None, |x| InterpolatedValue::Scale(*x)),
            Keyframes::Scalar(v) => v.get(index).map_or(InterpolatedValue::None, |x| InterpolatedValue::Scalar(*x)),
            Keyframes::Vector2(v) => v.get(index).map_or(InterpolatedValue::None, |x| InterpolatedValue::Vector2(*x)),
            Keyframes::Vector3(v) => v.get(index).map_or(InterpolatedValue::None, |x| InterpolatedValue::Vector3(*x)),
            Keyframes::Vector4(v) => v.get(index).map_or(InterpolatedValue::None, |x| InterpolatedValue::Vector4(*x)),
        }
    }

    /// The kind of value this channel holds, `None` for a transform channel.
    #[must_use]
    pub fn kind(&self) -> Option<PropertyKind> {
        match self {
            Keyframes::Translation(_) | Keyframes::Rotation(_) | Keyframes::Scale(_) => None,
            Keyframes::Scalar(_) => Some(PropertyKind::Scalar),
            Keyframes::Vector2(_) => Some(PropertyKind::Vector2),
            Keyframes::Vector3(_) => Some(PropertyKind::Vector3),
            Keyframes::Vector4(_) => Some(PropertyKind::Vector4),
        }
    }

    /// A value channel of `kind` with no keys.
    #[must_use]
    pub fn empty(kind: PropertyKind) -> Self {
        match kind {
            PropertyKind::Scalar => Keyframes::Scalar(Vec::new()),
            PropertyKind::Vector2 => Keyframes::Vector2(Vec::new()),
            PropertyKind::Vector3 => Keyframes::Vector3(Vec::new()),
            PropertyKind::Vector4 => Keyframes::Vector4(Vec::new()),
        }
    }

    /// Inserts `value` at `index` when it is of this channel; whether it was.
    fn insert(&mut self, index: usize, value: InterpolatedValue) -> bool {
        match (self, value) {
            (Keyframes::Translation(v), InterpolatedValue::Translation(x)) | (Keyframes::Scale(v), InterpolatedValue::Scale(x)) => {
                v.insert(index, x);
                true
            }
            (Keyframes::Rotation(v), InterpolatedValue::Rotation(x)) => {
                v.insert(index, x);
                true
            }
            (Keyframes::Scalar(v), InterpolatedValue::Scalar(x)) => {
                v.insert(index, x);
                true
            }
            (Keyframes::Vector2(v), InterpolatedValue::Vector2(x)) => {
                v.insert(index, x);
                true
            }
            (Keyframes::Vector3(v), InterpolatedValue::Vector3(x)) => {
                v.insert(index, x);
                true
            }
            (Keyframes::Vector4(v), InterpolatedValue::Vector4(x)) => {
                v.insert(index, x);
                true
            }
            _ => false,
        }
    }

    /// Replaces the value at `index` when it is of this channel; whether it was.
    fn replace(&mut self, index: usize, value: InterpolatedValue) -> bool {
        match (self, value) {
            (Keyframes::Translation(v), InterpolatedValue::Translation(x)) | (Keyframes::Scale(v), InterpolatedValue::Scale(x)) => {
                v.get_mut(index).map(|slot| *slot = x).is_some()
            }
            (Keyframes::Rotation(v), InterpolatedValue::Rotation(x)) => v.get_mut(index).map(|slot| *slot = x).is_some(),
            (Keyframes::Scalar(v), InterpolatedValue::Scalar(x)) => v.get_mut(index).map(|slot| *slot = x).is_some(),
            (Keyframes::Vector2(v), InterpolatedValue::Vector2(x)) => v.get_mut(index).map(|slot| *slot = x).is_some(),
            (Keyframes::Vector3(v), InterpolatedValue::Vector3(x)) => v.get_mut(index).map(|slot| *slot = x).is_some(),
            (Keyframes::Vector4(v), InterpolatedValue::Vector4(x)) => v.get_mut(index).map(|slot| *slot = x).is_some(),
            _ => false,
        }
    }

    fn remove(&mut self, index: usize) {
        // Bounds-checked like every other access here: the fields are public, so a channel can be
        // shorter than the track's timestamps.
        fn take<T>(v: &mut Vec<T>, index: usize) {
            if index < v.len() {
                v.remove(index);
            }
        }
        match self {
            Keyframes::Translation(v) | Keyframes::Scale(v) | Keyframes::Vector3(v) => take(v, index),
            Keyframes::Rotation(v) => take(v, index),
            Keyframes::Scalar(v) => take(v, index),
            Keyframes::Vector2(v) => take(v, index),
            Keyframes::Vector4(v) => take(v, index),
        }
    }

    /// A zero value of this channel — what a new key's cubic tangents start as.
    fn zero(&self) -> InterpolatedValue {
        match self {
            Keyframes::Translation(_) => InterpolatedValue::Translation(Vec3::ZERO),
            Keyframes::Rotation(_) => InterpolatedValue::Rotation(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)),
            Keyframes::Scale(_) => InterpolatedValue::Scale(Vec3::ZERO),
            Keyframes::Scalar(_) => InterpolatedValue::Scalar(0.0),
            Keyframes::Vector2(_) => InterpolatedValue::Vector2(Vec2::ZERO),
            Keyframes::Vector3(_) => InterpolatedValue::Vector3(Vec3::ZERO),
            Keyframes::Vector4(_) => InterpolatedValue::Vector4(Vec4::ZERO),
        }
    }

    /// Returns the number of keyframe values held by this channel.
    pub fn len(&self) -> usize {
        match self {
            Keyframes::Translation(v) => v.len(),
            Keyframes::Rotation(v) => v.len(),
            Keyframes::Scale(v) => v.len(),
            Keyframes::Scalar(v) => v.len(),
            Keyframes::Vector2(v) => v.len(),
            Keyframes::Vector3(v) => v.len(),
            Keyframes::Vector4(v) => v.len(),
        }
    }

    /// Returns `true` if this channel holds no keyframe values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// How values between two keyframes are blended.
///
/// Mirrors the glTF animation sampler interpolation modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Interpolation {
    /// Linear (lerp for vectors, slerp for rotations).
    #[default]
    Linear,
    /// Hold the value of the previous keyframe until the next one.
    Step,
    /// Cubic Hermite spline using per-keyframe in/out tangents. Requires
    /// [`Track::tangents`] to be populated; falls back to `Linear` if absent.
    CubicSpline,
}

/// In/out tangents for [`Interpolation::CubicSpline`].
///
/// glTF stores cubic-spline output accessors as interleaved
/// `[in_tangent, value, out_tangent]` triples per keyframe; when loading, split
/// them into the track's `keyframes` (the values) and this struct. Both tangent
/// arrays must use the same [`Keyframes`] variant and length as the values.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CubicTangents {
    /// In-tangent per keyframe (aligned with `keyframe_timestamps`).
    pub in_tangents: Keyframes,
    /// Out-tangent per keyframe (aligned with `keyframe_timestamps`).
    pub out_tangents: Keyframes,
}

/// A single animated channel targeting one named entity: its transform ([`Track::new`]), or
/// one property of one of its components ([`Track::property`]).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Track {
    /// Name of the entity this track animates (resolved at runtime).
    pub target_name: String,
    /// The property this track animates — `"PointLight.intensity"`, `"Material.albedo.w"` — or
    /// `None` for a transform track. The name is `"<component>.<field>[.<lane>]"`, the component
    /// named as the scene registry names it; the driver looks it up in the world's
    /// `PropertyRegistry` (`gizmo_animation::property`). Absent from a saved transform track,
    /// so a clip saved before property tracks existed loads unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    /// Keyframe times in seconds; must be sorted ascending and match `keyframes` in length.
    pub keyframe_timestamps: Vec<f32>,
    /// The keyframe values for this track.
    pub keyframes: Keyframes,
    /// How to blend between keyframes. Defaults to [`Interpolation::Linear`].
    pub interpolation: Interpolation,
    /// Cubic-spline tangents; only consulted when
    /// `interpolation == Interpolation::CubicSpline`.
    pub tangents: Option<CubicTangents>,
}

impl Track {
    /// Keys `value` at `time`: replacing the key already within a millisecond of it, else inserted
    /// where the timestamps stay sorted (sampling binary-searches them). Returns the key's index, or
    /// `None` when `value` is not of this track's channel or `time` is not finite. A cubic track's
    /// new key gets zero tangents.
    pub fn set_key(&mut self, time: f32, value: InterpolatedValue) -> Option<usize> {
        if !time.is_finite() {
            return None;
        }
        if let Some(i) = self.keyframe_timestamps.iter().position(|t| (t - time).abs() < 1.0e-3) {
            return self.keyframes.replace(i, value).then_some(i);
        }
        let i = self.keyframe_timestamps.partition_point(|t| *t < time);
        if !self.keyframes.insert(i, value) {
            return None;
        }
        self.keyframe_timestamps.insert(i, time);
        if let Some(t) = self.tangents.as_mut() {
            let zero = t.in_tangents.zero();
            t.in_tangents.insert(i, zero);
            t.out_tangents.insert(i, zero);
        }
        Some(i)
    }

    /// Removes key `index`; whether there was one.
    pub fn remove_key(&mut self, index: usize) -> bool {
        if index >= self.keyframe_timestamps.len() {
            return false;
        }
        self.keyframe_timestamps.remove(index);
        self.keyframes.remove(index);
        if let Some(t) = self.tangents.as_mut() {
            t.in_tangents.remove(index);
            t.out_tangents.remove(index);
        }
        true
    }

    /// Moves key `index` to `time` (clamped at zero), the list kept sorted; returns where it landed.
    pub fn retime_key(&mut self, index: usize, time: f32) -> Option<usize> {
        if index >= self.keyframe_timestamps.len() || !time.is_finite() {
            return None;
        }
        let value = self.keyframes.value(index);
        let tangents = self.tangents.as_ref().map(|t| (t.in_tangents.value(index), t.out_tangents.value(index)));
        self.remove_key(index);
        let time = time.max(0.0);
        // Onto another key's time: that key is replaced, as `set_key` does.
        let landed = self.set_key(time, value)?;
        if let (Some((i, o)), Some(t)) = (tangents, self.tangents.as_mut()) {
            t.in_tangents.replace(landed, i);
            t.out_tangents.replace(landed, o);
        }
        Some(landed)
    }

    /// Creates a new linearly-interpolated transform track targeting `target_name` with the
    /// given keyframe timestamps and translation, rotation or scale values.
    ///
    /// # Errors
    ///
    /// Returns a [`TrackError`] if `keyframe_timestamps` does not match the
    /// length of the data inside `keyframes`, if any timestamp is non-finite,
    /// or if the timestamps are not sorted in ascending order. Enforcing these
    /// invariants here guarantees that [`Track::sample`] cannot panic or emit
    /// `NaN` values for a track built through this constructor.
    /// [`TrackError::ChannelMismatch`] for value keyframes, which belong to [`Track::property`].
    pub fn new(
        target_name: impl Into<String>,
        keyframe_timestamps: Vec<f32>,
        keyframes: Keyframes,
    ) -> Result<Self, TrackError> {
        if keyframes.kind().is_some() {
            return Err(TrackError::ChannelMismatch { property: None });
        }
        Self::checked(target_name.into(), None, keyframe_timestamps, keyframes)
    }

    /// Creates a new linearly-interpolated property track: `property` (`"PointLight.intensity"`,
    /// `"Sprite.color.w"`) of the entity `target_name` names, keyed with scalar or vector values.
    ///
    /// Nothing here knows which properties exist — the driver looks the name up in the world's
    /// `PropertyRegistry` when it plays, and skips (warning once) a name it does not know or one
    /// whose kind is not the keyframes' kind.
    ///
    /// # Errors
    ///
    /// Everything [`Track::new`] checks about the timestamps; [`TrackError::ChannelMismatch`] for
    /// translation, rotation or scale keyframes; [`TrackError::InvalidPropertyName`] for a name
    /// that is not `"Component.field"`.
    pub fn property(
        target_name: impl Into<String>,
        property: impl Into<String>,
        keyframe_timestamps: Vec<f32>,
        keyframes: Keyframes,
    ) -> Result<Self, TrackError> {
        let property = property.into();
        let mut parts = property.split('.');
        let well_formed = parts.clone().count() >= 2 && parts.all(|p| !p.is_empty());
        if !well_formed {
            return Err(TrackError::InvalidPropertyName { name: property });
        }
        if keyframes.kind().is_none() {
            return Err(TrackError::ChannelMismatch { property: Some(property) });
        }
        Self::checked(target_name.into(), Some(property), keyframe_timestamps, keyframes)
    }

    /// The timestamp checks both constructors share.
    fn checked(
        target_name: String,
        property: Option<String>,
        keyframe_timestamps: Vec<f32>,
        keyframes: Keyframes,
    ) -> Result<Self, TrackError> {
        let values = keyframes.len();
        let timestamps = keyframe_timestamps.len();
        if timestamps != values {
            return Err(TrackError::LengthMismatch { timestamps, values });
        }

        let mut prev: Option<f32> = None;
        for (index, &ts) in keyframe_timestamps.iter().enumerate() {
            if !ts.is_finite() {
                return Err(TrackError::NonFiniteTimestamp { index });
            }
            if let Some(p) = prev {
                if ts < p {
                    return Err(TrackError::UnsortedTimestamps { index });
                }
            }
            prev = Some(ts);
        }

        Ok(Self {
            target_name,
            property,
            keyframe_timestamps,
            keyframes,
            interpolation: Interpolation::Linear,
            tangents: None,
        })
    }

    /// Set the interpolation mode (builder style).
    pub fn with_interpolation(mut self, interpolation: Interpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    /// Attach cubic-spline tangents and switch to [`Interpolation::CubicSpline`].
    pub fn with_cubic_tangents(mut self, in_tangents: Keyframes, out_tangents: Keyframes) -> Self {
        self.interpolation = Interpolation::CubicSpline;
        self.tangents = Some(CubicTangents { in_tangents, out_tangents });
        self
    }

    /// Returns the time of the last keyframe, or `0.0` if the track is empty.
    pub fn duration(&self) -> f32 {
        self.keyframe_timestamps.last().copied().unwrap_or(0.0)
    }

    /// Interpolates the track at a given time `t`.
    ///
    /// Times before the first or after the last keyframe are clamped to the
    /// endpoint value. An empty track returns [`InterpolatedValue::None`].
    pub fn sample(&self, t: f32) -> InterpolatedValue {
        if self.keyframe_timestamps.is_empty() {
            return InterpolatedValue::None;
        }

        if t <= *self.keyframe_timestamps.first().unwrap() {
            return self.get_value(0);
        }

        if t >= *self.keyframe_timestamps.last().unwrap() {
            return self.get_value(self.keyframe_timestamps.len() - 1);
        }

        // Tek keyframe'de segment YOKTUR, ve buraya ancak bir NaN ile gelinebilir: NaN her
        // iki karşılaştırmayı da false yaptığı için yukarıdaki iki erken-dönüş atlanır.
        // Aşağıdaki clamp o durumda `0.clamp(1, 0)` olur ve `Ord::clamp` `min > max` diye
        // PANİKLER — yani NaN koruması sandığımız satırın kendisi panikleyen şeydi. Tek
        // keyframe'in cevabı zaten o keyframe.
        if self.keyframe_timestamps.len() < 2 {
            return self.get_value(0);
        }

        // Find the segment.
        let idx = self.keyframe_timestamps.partition_point(|&ts| ts <= t);
        // `idx` is in `1..len` here (the t-before-first / t-after-last cases
        // returned above), but guard the subtraction defensively in case of
        // NaN timestamps that break the ordering assumptions of partition_point.
        let idx1 = idx.clamp(1, self.keyframe_timestamps.len() - 1);
        let idx0 = idx1 - 1;

        let t0 = self.keyframe_timestamps[idx0];
        let t1 = self.keyframe_timestamps[idx1];
        // Guard against a zero/degenerate (or non-finite) segment span: two
        // identical or out-of-order timestamps would produce a division by zero
        // (NaN/Inf). Fall back to the start value rather than emitting NaN.
        let segment = t1 - t0;
        let factor = if segment.abs() > f32::EPSILON {
            ((t - t0) / segment).clamp(0.0, 1.0)
        } else {
            0.0
        };

        match self.effective_interpolation() {
            Interpolation::Step => self.get_value(idx0),
            Interpolation::Linear => self.interpolate_linear(idx0, idx1, factor),
            Interpolation::CubicSpline => self.interpolate_cubic(idx0, idx1, factor, segment),
        }
    }

    /// The interpolation actually used: falls back to `Linear` if `CubicSpline`
    /// was requested without tangents.
    fn effective_interpolation(&self) -> Interpolation {
        match self.interpolation {
            Interpolation::CubicSpline if self.tangents.is_none() => Interpolation::Linear,
            other => other,
        }
    }

    fn get_value(&self, index: usize) -> InterpolatedValue {
        // Bounds-checked access: the keyframe vectors are public and may be
        // shorter than `keyframe_timestamps`, so a mismatched-length Track must
        // not panic with an out-of-bounds index.
        match &self.keyframes {
            Keyframes::Translation(v) => match v.get(index) {
                Some(&val) => InterpolatedValue::Translation(val),
                None => InterpolatedValue::None,
            },
            Keyframes::Rotation(v) => match v.get(index) {
                Some(&val) => InterpolatedValue::Rotation(val),
                None => InterpolatedValue::None,
            },
            Keyframes::Scale(v) => match v.get(index) {
                Some(&val) => InterpolatedValue::Scale(val),
                None => InterpolatedValue::None,
            },
            Keyframes::Scalar(_) | Keyframes::Vector2(_) | Keyframes::Vector3(_) | Keyframes::Vector4(_) => {
                self.keyframes.value(index)
            }
        }
    }

    fn interpolate_linear(&self, idx0: usize, idx1: usize, factor: f32) -> InterpolatedValue {
        // Bounds-checked: a Track whose `keyframes` vector is shorter than its
        // `keyframe_timestamps` must degrade gracefully instead of panicking.
        match &self.keyframes {
            Keyframes::Translation(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Translation(v0.lerp(v1, factor)),
                _ => InterpolatedValue::None,
            },
            Keyframes::Rotation(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Rotation(v0.slerp(v1, factor)),
                _ => InterpolatedValue::None,
            },
            Keyframes::Scale(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Scale(v0.lerp(v1, factor)),
                _ => InterpolatedValue::None,
            },
            // The value channels blend as `v0 + (v1 - v0) * t`, not glam's `lerp`
            // (`v0 * (1 - t) + v1 * t`): a segment between two equal keys then holds the key
            // exactly, so a field held at a keyed value reads back as that value.
            Keyframes::Scalar(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Scalar(v0 + (v1 - v0) * factor),
                _ => InterpolatedValue::None,
            },
            Keyframes::Vector2(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Vector2(v0 + (v1 - v0) * factor),
                _ => InterpolatedValue::None,
            },
            Keyframes::Vector3(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Vector3(v0 + (v1 - v0) * factor),
                _ => InterpolatedValue::None,
            },
            Keyframes::Vector4(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Vector4(v0 + (v1 - v0) * factor),
                _ => InterpolatedValue::None,
            },
        }
    }

    /// Cubic Hermite spline using the real in/out tangents from the animation
    /// format (glTF convention: `m0 = out_tangent[idx0] * dt`,
    /// `m1 = in_tangent[idx1] * dt`, where `dt` is the segment duration).
    ///
    /// Falls back to [`Self::interpolate_linear`] if the value/tangent variants
    /// disagree or the tangent arrays are shorter than the indexed keyframes, so
    /// a malformed cubic track degrades gracefully instead of panicking.
    fn interpolate_cubic(
        &self,
        idx0: usize,
        idx1: usize,
        factor: f32,
        segment: f32,
    ) -> InterpolatedValue {
        // `effective_interpolation` guarantees tangents are present here.
        let tangents = self.tangents.as_ref().expect("cubic requires tangents");
        match (&self.keyframes, &tangents.in_tangents, &tangents.out_tangents) {
            (Keyframes::Translation(v), Keyframes::Translation(in_t), Keyframes::Translation(out_t)) => {
                match (v.get(idx0), out_t.get(idx0), v.get(idx1), in_t.get(idx1)) {
                    (Some(&p0), Some(&m0), Some(&p1), Some(&m1)) => InterpolatedValue::Translation(
                        hermite_vec3(p0, m0 * segment, p1, m1 * segment, factor),
                    ),
                    _ => self.interpolate_linear(idx0, idx1, factor),
                }
            }
            (Keyframes::Scale(v), Keyframes::Scale(in_t), Keyframes::Scale(out_t)) => {
                match (v.get(idx0), out_t.get(idx0), v.get(idx1), in_t.get(idx1)) {
                    (Some(&p0), Some(&m0), Some(&p1), Some(&m1)) => InterpolatedValue::Scale(
                        hermite_vec3(p0, m0 * segment, p1, m1 * segment, factor),
                    ),
                    _ => self.interpolate_linear(idx0, idx1, factor),
                }
            }
            (Keyframes::Rotation(v), Keyframes::Rotation(in_t), Keyframes::Rotation(out_t)) => {
                let scale = |q: Quat, s: f32| Quat::from_xyzw(q.x * s, q.y * s, q.z * s, q.w * s);
                match (v.get(idx0), out_t.get(idx0), v.get(idx1), in_t.get(idx1)) {
                    (Some(&p0), Some(&m0), Some(&p1), Some(&m1)) => InterpolatedValue::Rotation(
                        hermite_quat(p0, scale(m0, segment), p1, scale(m1, segment), factor),
                    ),
                    _ => self.interpolate_linear(idx0, idx1, factor),
                }
            }
            (Keyframes::Scalar(v), Keyframes::Scalar(in_t), Keyframes::Scalar(out_t)) => {
                Self::cubic_value(v, in_t, out_t, idx0, idx1, factor, segment)
                    .map_or_else(|| self.interpolate_linear(idx0, idx1, factor), InterpolatedValue::Scalar)
            }
            (Keyframes::Vector2(v), Keyframes::Vector2(in_t), Keyframes::Vector2(out_t)) => {
                Self::cubic_value(v, in_t, out_t, idx0, idx1, factor, segment)
                    .map_or_else(|| self.interpolate_linear(idx0, idx1, factor), InterpolatedValue::Vector2)
            }
            (Keyframes::Vector3(v), Keyframes::Vector3(in_t), Keyframes::Vector3(out_t)) => {
                Self::cubic_value(v, in_t, out_t, idx0, idx1, factor, segment)
                    .map_or_else(|| self.interpolate_linear(idx0, idx1, factor), InterpolatedValue::Vector3)
            }
            (Keyframes::Vector4(v), Keyframes::Vector4(in_t), Keyframes::Vector4(out_t)) => {
                Self::cubic_value(v, in_t, out_t, idx0, idx1, factor, segment)
                    .map_or_else(|| self.interpolate_linear(idx0, idx1, factor), InterpolatedValue::Vector4)
            }
            // Mismatched value/tangent variants: fall back to linear rather than panic.
            _ => self.interpolate_linear(idx0, idx1, factor),
        }
    }

    /// One value channel's cubic segment, through the generic Hermite basis; `None` when a value
    /// or tangent is missing (the caller falls back to linear).
    fn cubic_value<T>(v: &[T], in_t: &[T], out_t: &[T], idx0: usize, idx1: usize, factor: f32, segment: f32) -> Option<T>
    where
        T: Copy + std::ops::Add<Output = T> + std::ops::Mul<f32, Output = T>,
    {
        let (p0, m0, p1, m1) = (*v.get(idx0)?, *out_t.get(idx0)?, *v.get(idx1)?, *in_t.get(idx1)?);
        Some(hermite(p0, m0 * segment, p1, m1 * segment, factor))
    }
}

/// A value sampled from a [`Track`] at a specific time.
///
/// The variant is the channel the value belongs to: a transform channel (translation,
/// rotation, scale) or a value channel (scalar, vector), as [`Keyframes`] has them.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum InterpolatedValue {
    /// The track produced no value (e.g. it had no keyframes).
    None,
    /// A sampled position.
    Translation(Vec3),
    /// A sampled rotation.
    Rotation(Quat),
    /// A sampled scale.
    Scale(Vec3),
    /// A sampled one-number property value.
    Scalar(f32),
    /// A sampled two-number property value.
    Vector2(Vec2),
    /// A sampled three-number property value.
    Vector3(Vec3),
    /// A sampled four-number property value.
    Vector4(Vec4),
}

impl InterpolatedValue {
    /// The kind of a value-channel sample, `None` for a transform channel or no value.
    #[must_use]
    pub fn kind(&self) -> Option<PropertyKind> {
        match self {
            InterpolatedValue::Scalar(_) => Some(PropertyKind::Scalar),
            InterpolatedValue::Vector2(_) => Some(PropertyKind::Vector2),
            InterpolatedValue::Vector3(_) => Some(PropertyKind::Vector3),
            InterpolatedValue::Vector4(_) => Some(PropertyKind::Vector4),
            InterpolatedValue::None
            | InterpolatedValue::Translation(_)
            | InterpolatedValue::Rotation(_)
            | InterpolatedValue::Scale(_) => None,
        }
    }
}

/// Represents an animation sequence: a named collection of [`Track`]s.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct AnimationClip {
    /// Human-readable clip name.
    pub name: String,
    /// The tracks that make up this clip.
    pub tracks: Vec<Track>,
}

impl AnimationClip {
    /// Returns the length of the longest track, in seconds.
    pub fn duration(&self) -> f32 {
        self.tracks.iter().map(|t| t.duration()).fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f32 = 1e-4;

    fn scale_track(interp: Interpolation) -> Track {
        Track::new(
            "bone",
            vec![0.0, 1.0],
            Keyframes::Scale(vec![Vec3::new(1.0, 1.0, 1.0), Vec3::new(2.0, 4.0, 8.0)]),
        )
        .expect("valid track")
        .with_interpolation(interp)
    }

    /// Keys stay sorted however they are set, moved or removed, and a key set onto an existing time
    /// replaces it; a value of another channel is refused.
    #[test]
    fn keys_are_set_moved_and_removed_in_order() {
        let mut t = Track::new("", vec![0.0, 1.0], Keyframes::Translation(vec![Vec3::ZERO, Vec3::X])).expect("valid");
        assert_eq!(t.set_key(0.5, InterpolatedValue::Translation(Vec3::Y)), Some(1));
        assert_eq!(t.keyframe_timestamps, vec![0.0, 0.5, 1.0]);
        assert_eq!(t.set_key(1.0, InterpolatedValue::Translation(Vec3::Z)), Some(2), "onto 1.0: replaced");
        assert_eq!(t.keyframes.len(), 3);
        assert_eq!(t.keyframes.value(2), InterpolatedValue::Translation(Vec3::Z));
        assert_eq!(t.set_key(0.2, InterpolatedValue::Scale(Vec3::ONE)), None, "another channel");
        assert_eq!(t.retime_key(0, 2.0), Some(2), "the first key moved past the others");
        assert_eq!(t.keyframe_timestamps, vec![0.5, 1.0, 2.0]);
        assert_eq!(t.keyframes.value(2), InterpolatedValue::Translation(Vec3::ZERO));
        assert!(t.remove_key(1));
        assert_eq!(t.keyframe_timestamps, vec![0.5, 2.0]);
        assert!(!t.remove_key(5));
        assert_eq!(t.sample(1.25), InterpolatedValue::Translation(Vec3::Y * 0.5));
    }

    #[test]
    fn scale_track_linear_non_uniform() {
        // A non-uniform scale keyed 1..(2,4,8) sampled at t=0.5 must lerp each axis
        // independently. This is the regression guard for scale reaching the pose.
        let track = scale_track(Interpolation::Linear);
        match track.sample(0.5) {
            InterpolatedValue::Scale(s) => {
                assert!((s - Vec3::new(1.5, 2.5, 4.5)).length() < TOL, "got {s:?}");
            }
            other => panic!("expected Scale, got {other:?}"),
        }
    }

    #[test]
    fn scale_track_endpoints_and_clamp() {
        let track = scale_track(Interpolation::Linear);
        assert_eq!(track.sample(0.0), InterpolatedValue::Scale(Vec3::new(1.0, 1.0, 1.0)));
        assert_eq!(track.sample(1.0), InterpolatedValue::Scale(Vec3::new(2.0, 4.0, 8.0)));
        // Clamp beyond the ends.
        assert_eq!(track.sample(-5.0), InterpolatedValue::Scale(Vec3::new(1.0, 1.0, 1.0)));
        assert_eq!(track.sample(9.0), InterpolatedValue::Scale(Vec3::new(2.0, 4.0, 8.0)));
    }

    #[test]
    fn scale_track_step_holds_previous() {
        // Step interpolation did not exist before; a linear-only sampler would
        // return (1.5,2.5,4.5) here instead of holding the first keyframe.
        let track = scale_track(Interpolation::Step);
        match track.sample(0.5) {
            InterpolatedValue::Scale(s) => {
                assert!((s - Vec3::new(1.0, 1.0, 1.0)).length() < TOL, "step should hold prev, got {s:?}");
            }
            other => panic!("expected Scale, got {other:?}"),
        }
    }

    #[test]
    fn scale_track_cubic_uses_real_tangents() {
        // Cubic spline with real (non-zero) tangents must differ from the linear
        // midpoint. A sampler that ignored tangents (or lacked cubic support)
        // would return the linear value (1.5,2.5,4.5) and fail this test.
        let values = Keyframes::Scale(vec![Vec3::new(1.0, 1.0, 1.0), Vec3::new(2.0, 4.0, 8.0)]);
        // Steep out-tangent at k0, flat in-tangent at k1 -> curve overshoots the
        // linear line in the first half.
        let in_t = Keyframes::Scale(vec![Vec3::ZERO, Vec3::ZERO]);
        let out_t = Keyframes::Scale(vec![Vec3::new(4.0, 4.0, 4.0), Vec3::ZERO]);
        let track = Track::new("bone", vec![0.0, 1.0], values)
            .unwrap()
            .with_cubic_tangents(in_t, out_t);

        let cubic = match track.sample(0.5) {
            InterpolatedValue::Scale(s) => s,
            other => panic!("expected Scale, got {other:?}"),
        };
        let linear = Vec3::new(1.5, 2.5, 4.5);
        assert!((cubic - linear).length() > 0.1, "cubic must differ from linear, got {cubic:?}");

        // Verify against the hand-computed Hermite value for the X axis.
        // p0=1, m0=4*1 (dt=1), p1=2, m1=0, t=0.5:
        // h00=0.5, h10=0.125, h01=0.5, h11=-0.125 -> 0.5*1 + 0.125*4 + 0.5*2 = 2.0
        assert!((cubic.x - 2.0).abs() < TOL, "cubic X expected 2.0, got {}", cubic.x);
    }

    #[test]
    fn cubic_without_tangents_falls_back_to_linear() {
        let mut track = scale_track(Interpolation::CubicSpline);
        track.tangents = None; // request cubic but provide nothing
        match track.sample(0.5) {
            InterpolatedValue::Scale(s) => assert!((s - Vec3::new(1.5, 2.5, 4.5)).length() < TOL),
            other => panic!("expected Scale, got {other:?}"),
        }
    }

    #[test]
    fn empty_track_samples_none() {
        let track = Track::new("bone", vec![], Keyframes::Scale(vec![])).unwrap();
        assert_eq!(track.sample(0.5), InterpolatedValue::None);
    }

    // ── Track::new invariant / error paths ─────────────────────────────

    #[test]
    fn track_new_rejects_length_mismatch() {
        // Two timestamps, one value: indexing at sample time would go OOB.
        let err = Track::new(
            "bone",
            vec![0.0, 1.0],
            Keyframes::Scale(vec![Vec3::ONE]),
        )
        .unwrap_err();
        assert_eq!(err, TrackError::LengthMismatch { timestamps: 2, values: 1 });
    }

    #[test]
    fn track_new_rejects_non_finite_timestamp() {
        // A NaN/Inf timestamp would poison the segment-span division at sample time.
        for (bad, idx) in [(f32::NAN, 1usize), (f32::INFINITY, 1)] {
            let err = Track::new(
                "bone",
                vec![0.0, bad],
                Keyframes::Scale(vec![Vec3::ONE, Vec3::ONE]),
            )
            .unwrap_err();
            assert_eq!(err, TrackError::NonFiniteTimestamp { index: idx }, "for {bad}");
        }
    }

    #[test]
    fn track_new_rejects_unsorted_timestamps() {
        // Descending pair; the error must point at the first out-of-order index.
        let err = Track::new(
            "bone",
            vec![0.0, 2.0, 1.0],
            Keyframes::Scale(vec![Vec3::ONE, Vec3::ONE, Vec3::ONE]),
        )
        .unwrap_err();
        assert_eq!(err, TrackError::UnsortedTimestamps { index: 2 });
    }

    #[test]
    fn track_new_accepts_equal_adjacent_timestamps() {
        // Equal (non-descending) timestamps are allowed — sorting is `>=`, not `>`.
        assert!(Track::new(
            "bone",
            vec![0.0, 1.0, 1.0],
            Keyframes::Scale(vec![Vec3::ONE, Vec3::ONE, Vec3::ONE]),
        )
        .is_ok());
    }

    // ── Rotation channel: slerp boundary + normalization ───────────────

    fn rotation_track(interp: Interpolation) -> Track {
        use std::f32::consts::PI;
        Track::new(
            "bone",
            vec![0.0, 1.0],
            Keyframes::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_y(PI / 2.0)]),
        )
        .expect("valid track")
        .with_interpolation(interp)
    }

    #[test]
    fn rotation_track_slerp_midpoint_is_half_angle_and_unit() {
        use std::f32::consts::PI;
        // slerp(identity, 90°_y, 0.5) is the 45°_y rotation and must stay unit-length.
        let track = rotation_track(Interpolation::Linear);
        match track.sample(0.5) {
            InterpolatedValue::Rotation(q) => {
                assert!((q.length() - 1.0).abs() < TOL, "slerp result must be unit, got {}", q.length());
                let expected = Quat::from_rotation_y(PI / 4.0);
                assert!(q.dot(expected).abs() > 0.999, "midpoint should be 45°, got {q:?}");
            }
            other => panic!("expected Rotation, got {other:?}"),
        }
    }

    #[test]
    fn rotation_track_endpoints_and_clamp_are_exact() {
        use std::f32::consts::PI;
        let track = rotation_track(Interpolation::Linear);
        let ends = [
            (0.0_f32, Quat::IDENTITY),
            (1.0, Quat::from_rotation_y(PI / 2.0)),
            (-3.0, Quat::IDENTITY),               // clamp before start
            (7.0, Quat::from_rotation_y(PI / 2.0)), // clamp after end
        ];
        for (t, expected) in ends {
            match track.sample(t) {
                InterpolatedValue::Rotation(q) => {
                    assert!(q.dot(expected).abs() > 0.9999, "t={t} expected {expected:?}, got {q:?}");
                }
                other => panic!("expected Rotation, got {other:?}"),
            }
        }
    }

    #[test]
    fn rotation_cubic_result_is_normalized() {
        // Component-wise Hermite on a quaternion denormalizes; the sampler must
        // re-normalize so the pose stays a valid unit rotation.
        let values = Keyframes::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_y(1.0)]);
        let zero = Quat::from_xyzw(0.0, 0.0, 0.0, 0.0);
        let in_t = Keyframes::Rotation(vec![zero, zero]);
        let out_t = Keyframes::Rotation(vec![Quat::from_xyzw(0.0, 0.4, 0.0, 0.0), zero]);
        let track = Track::new("bone", vec![0.0, 1.0], values)
            .unwrap()
            .with_cubic_tangents(in_t, out_t);
        match track.sample(0.5) {
            InterpolatedValue::Rotation(q) => {
                assert!((q.length() - 1.0).abs() < 1e-4, "cubic quat must be unit, got {}", q.length());
            }
            other => panic!("expected Rotation, got {other:?}"),
        }
    }

    // ── Translation channel (lerp) ─────────────────────────────────────

    #[test]
    fn translation_track_lerps_each_axis() {
        let track = Track::new(
            "bone",
            vec![0.0, 2.0],
            Keyframes::Translation(vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, -8.0, 2.0)]),
        )
        .unwrap();
        // t=0.5 in [0,2] → factor 0.25 → quarter of the way.
        match track.sample(0.5) {
            InterpolatedValue::Translation(v) => {
                assert!((v - Vec3::new(1.0, -2.0, 0.5)).length() < TOL, "got {v:?}");
            }
            other => panic!("expected Translation, got {other:?}"),
        }
    }

    // ── Graceful degradation / fallback edges ──────────────────────────

    #[test]
    fn mismatched_length_track_samples_none_instead_of_panicking() {
        // A track whose values are shorter than its timestamps (constructed by
        // mutating a valid track) must degrade to None, not panic on OOB.
        let mut track = Track::new(
            "bone",
            vec![0.0, 1.0],
            Keyframes::Scale(vec![Vec3::ONE, Vec3::splat(2.0)]),
        )
        .unwrap();
        track.keyframe_timestamps.push(2.0); // now 3 timestamps, 2 values
        // t between the 2nd and (phantom) 3rd timestamp → idx1 indexes past the values.
        assert_eq!(track.sample(1.5), InterpolatedValue::None);
    }

    #[test]
    fn cubic_mismatched_variant_falls_back_to_linear() {
        // Scale values but Translation tangents: the sampler must not panic; it
        // falls back to the linear midpoint instead.
        let track = Track::new(
            "bone",
            vec![0.0, 1.0],
            Keyframes::Scale(vec![Vec3::ONE, Vec3::splat(3.0)]),
        )
        .unwrap()
        .with_cubic_tangents(
            Keyframes::Translation(vec![Vec3::ZERO, Vec3::ZERO]),
            Keyframes::Translation(vec![Vec3::ZERO, Vec3::ZERO]),
        );
        match track.sample(0.5) {
            InterpolatedValue::Scale(s) => {
                assert!((s - Vec3::splat(2.0)).length() < TOL, "expected linear midpoint, got {s:?}");
            }
            other => panic!("expected Scale, got {other:?}"),
        }
    }

    // ── Duration ───────────────────────────────────────────────────────

    #[test]
    fn empty_track_duration_is_zero() {
        let track = Track::new("bone", vec![], Keyframes::Scale(vec![])).unwrap();
        assert_eq!(track.duration(), 0.0);
    }

    #[test]
    fn clip_duration_is_the_longest_track() {
        let short = Track::new(
            "a",
            vec![0.0, 1.0],
            Keyframes::Scale(vec![Vec3::ONE, Vec3::ONE]),
        )
        .unwrap();
        let long = Track::new(
            "b",
            vec![0.0, 2.5],
            Keyframes::Scale(vec![Vec3::ONE, Vec3::ONE]),
        )
        .unwrap();
        let clip = AnimationClip {
            tracks: vec![short, long],
            ..Default::default()
        };
        assert!((clip.duration() - 2.5).abs() < TOL, "got {}", clip.duration());
        // An empty clip has zero duration (fold starts at 0.0).
        assert_eq!(AnimationClip::default().duration(), 0.0);
    }
}


/// Value channels (property tracks): sampling, the constructors' checks, editing and the saved form.
#[cfg(test)]
mod value_channels {
    use super::*;

    fn scalar(interp: Interpolation) -> Track {
        Track::property("", "PointLight.intensity", vec![0.0, 2.0], Keyframes::Scalar(vec![1.0, 5.0]))
            .expect("valid")
            .with_interpolation(interp)
    }

    /// Linear is a lerp, step holds the previous key, and the ends clamp.
    #[test]
    fn scalar_track_lerps_and_steps() {
        let t = scalar(Interpolation::Linear);
        assert_eq!(t.sample(0.5), InterpolatedValue::Scalar(2.0));
        assert_eq!(t.sample(-1.0), InterpolatedValue::Scalar(1.0));
        assert_eq!(t.sample(9.0), InterpolatedValue::Scalar(5.0));
        assert_eq!(scalar(Interpolation::Step).sample(1.9), InterpolatedValue::Scalar(1.0));
        let v = Track::property("", "Sprite.size", vec![0.0, 1.0], Keyframes::Vector2(vec![Vec2::ZERO, Vec2::new(2.0, 4.0)]))
            .expect("valid");
        assert_eq!(v.sample(0.25), InterpolatedValue::Vector2(Vec2::new(0.5, 1.0)));
        // Between two equal keys the key itself, exactly (glam's `lerp` would give 0.19999999).
        let c = Vec4::new(0.2, 0.4, 0.6, 1.0);
        let held = Track::property("", "Sprite.color", vec![0.0, 1.0], Keyframes::Vector4(vec![c, c])).expect("valid");
        assert_eq!(held.sample(0.37), InterpolatedValue::Vector4(c));
    }

    /// A cubic `Vector4` segment is the closed-form Hermite polynomial, lane by lane, with the
    /// tangents scaled by the segment's length (glTF's convention, as the transform channels).
    #[test]
    fn vector4_cubic_matches_closed_form_hermite() {
        let (p0, p1) = (Vec4::new(0.0, 1.0, -2.0, 0.5), Vec4::new(4.0, -1.0, 2.0, 1.0));
        let (m0, m1) = (Vec4::new(1.0, 0.0, 3.0, -2.0), Vec4::new(-1.0, 2.0, 0.0, 0.5));
        let t = Track::property("", "Material.albedo", vec![1.0, 3.0], Keyframes::Vector4(vec![p0, p1]))
            .expect("valid")
            .with_cubic_tangents(Keyframes::Vector4(vec![Vec4::ZERO, m1]), Keyframes::Vector4(vec![m0, Vec4::ZERO]));
        let (dt, s) = (2.0_f32, 0.3_f32);
        let InterpolatedValue::Vector4(got) = t.sample(1.0 + s * dt) else { panic!("a Vector4 sample") };
        for lane in 0..4 {
            let h00 = 2.0 * s.powi(3) - 3.0 * s * s + 1.0;
            let h10 = s.powi(3) - 2.0 * s * s + s;
            let h01 = -2.0 * s.powi(3) + 3.0 * s * s;
            let h11 = s.powi(3) - s * s;
            let want = h00 * p0[lane] + h10 * m0[lane] * dt + h01 * p1[lane] + h11 * m1[lane] * dt;
            assert!((got[lane] - want).abs() < 1e-5, "lane {lane}: {} vs {want}", got[lane]);
        }
        // A cubic track missing a tangent degrades to linear, as the transform channels do.
        let mut broken = t.clone();
        broken.tangents.as_mut().unwrap().in_tangents = Keyframes::Vector4(vec![]);
        assert_eq!(broken.sample(2.0), InterpolatedValue::Vector4(p0 + (p1 - p0) * 0.5));
    }

    /// Adding value channels must not move a single bit of a translation track's cubic samples:
    /// these are `to_bits` captured from the sampler before the value channels existed.
    #[test]
    fn translation_cubic_is_bit_identical() {
        let values = Keyframes::Translation(vec![Vec3::new(1.0, -2.0, 0.5), Vec3::new(4.0, 3.0, -1.5)]);
        let in_t = Keyframes::Translation(vec![Vec3::ZERO, Vec3::new(0.3, -1.1, 2.0)]);
        let out_t = Keyframes::Translation(vec![Vec3::new(2.5, 0.7, -0.9), Vec3::ZERO]);
        let track = Track::new("", vec![0.0, 1.7], values).unwrap().with_cubic_tangents(in_t, out_t);
        let expected: [(f32, [u32; 3]); 3] = [
            (0.3, [0x3fdf167e, 0xbfb2df8f, 0x3d84c043]),
            (0.85, [0x403deb85, 0x3f61eb86, 0xbf8ee148]),
            (1.4, [0x40733def, 0x4035e5d5, 0xbfe40152]),
        ];
        for (t, bits) in expected {
            let InterpolatedValue::Translation(v) = track.sample(t) else { panic!("a Translation sample") };
            assert_eq!([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()], bits, "t = {t}");
        }
    }

    #[test]
    fn property_track_rejects_transform_keyframes() {
        let err = Track::property("", "PointLight.intensity", vec![0.0], Keyframes::Translation(vec![Vec3::ZERO])).unwrap_err();
        assert_eq!(err, TrackError::ChannelMismatch { property: Some("PointLight.intensity".into()) });
        for bad in ["intensity", "PointLight.", ".intensity", "A..b", ""] {
            let err = Track::property("", bad, vec![0.0], Keyframes::Scalar(vec![1.0])).unwrap_err();
            assert_eq!(err, TrackError::InvalidPropertyName { name: bad.into() }, "{bad:?}");
        }
        assert!(Track::property("", "Sprite.color.w", vec![0.0], Keyframes::Scalar(vec![1.0])).is_ok());
        // The timestamp checks still run.
        let err = Track::property("", "A.b", vec![0.0, 1.0], Keyframes::Scalar(vec![1.0])).unwrap_err();
        assert_eq!(err, TrackError::LengthMismatch { timestamps: 2, values: 1 });
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn transform_track_rejects_value_keyframes() {
        let err = Track::new("", vec![0.0], Keyframes::Scalar(vec![1.0])).unwrap_err();
        assert_eq!(err, TrackError::ChannelMismatch { property: None });
        assert!(err.to_string().contains("transform track"));
    }

    /// `set_key`, `retime_key` and `remove_key` work on value channels as on transform ones, and a
    /// cubic track's new key gets zero tangents of the track's own kind.
    #[test]
    fn set_key_retime_remove_on_property_tracks() {
        let mut t = Track::property("", "Text.color", vec![0.0, 1.0], Keyframes::Vector4(vec![Vec4::ZERO, Vec4::ONE]))
            .expect("valid")
            .with_cubic_tangents(Keyframes::Vector4(vec![Vec4::ONE; 2]), Keyframes::Vector4(vec![Vec4::ONE; 2]));
        assert_eq!(t.set_key(0.5, InterpolatedValue::Vector4(Vec4::splat(0.5))), Some(1));
        assert_eq!(t.set_key(0.5, InterpolatedValue::Scalar(1.0)), None, "another kind is refused");
        let tan = t.tangents.as_ref().unwrap();
        assert_eq!(tan.in_tangents.value(1), InterpolatedValue::Vector4(Vec4::ZERO));
        assert_eq!(tan.out_tangents.value(1), InterpolatedValue::Vector4(Vec4::ZERO));
        assert_eq!(tan.in_tangents.len(), 3);
        assert_eq!(t.retime_key(0, 2.0), Some(2));
        assert_eq!(t.keyframe_timestamps, vec![0.5, 1.0, 2.0]);
        assert_eq!(t.keyframes.value(2), InterpolatedValue::Vector4(Vec4::ZERO));
        assert_eq!(t.tangents.as_ref().unwrap().in_tangents.value(2), InterpolatedValue::Vector4(Vec4::ONE), "its tangents moved with it");
        assert!(t.remove_key(0));
        assert_eq!(t.keyframes.len(), 2);
        assert_eq!(t.tangents.as_ref().unwrap().out_tangents.len(), 2);

        let mut s = Track::property("", "Camera.fov", vec![], Keyframes::empty(PropertyKind::Scalar)).expect("valid");
        assert_eq!(s.set_key(1.0, InterpolatedValue::Scalar(60.0)), Some(0));
        assert_eq!(s.keyframes.kind(), Some(PropertyKind::Scalar));
        assert_eq!(s.sample(3.0), InterpolatedValue::Scalar(60.0));
        assert_eq!(Keyframes::Translation(vec![]).kind(), None);
    }

    /// A clip saved before property tracks existed (the commit 76841f86 format: no `property`
    /// field) loads as a transform track, and saves again without the field.
    #[test]
    fn old_transform_clip_ron_still_loads_and_resaves_without_property() {
        let old = r#"(name: "door", tracks: [(target_name: "", keyframe_timestamps: [0.0, 1.0], keyframes: Translation([(0.0, 0.0, 0.0), (10.0, 0.0, 0.0)]), interpolation: Linear, tangents: None)])"#;
        let clip: AnimationClip = ron::from_str(old).expect("the old format loads");
        assert_eq!(clip.tracks[0].property, None);
        assert_eq!(clip.tracks[0].sample(0.5), InterpolatedValue::Translation(Vec3::new(5.0, 0.0, 0.0)));
        let again = ron::to_string(&clip).expect("saves");
        assert!(!again.contains("property"), "{again}");
        let back: AnimationClip = ron::from_str(&again).expect("reloads");
        assert_eq!(back.tracks[0].sample(0.5), clip.tracks[0].sample(0.5));
    }

    #[test]
    fn property_clip_ron_roundtrip() {
        let clip = AnimationClip {
            name: "pulse".into(),
            tracks: vec![
                Track::new("", vec![0.0, 1.0], Keyframes::Translation(vec![Vec3::ZERO, Vec3::X])).unwrap(),
                Track::property("lamp", "PointLight.intensity", vec![0.0, 1.0], Keyframes::Scalar(vec![1.0, 3.0]))
                    .unwrap()
                    .with_cubic_tangents(Keyframes::Scalar(vec![0.0, 2.0]), Keyframes::Scalar(vec![1.0, 0.0])),
                Track::property("", "Material.albedo", vec![0.0, 2.0], Keyframes::Vector4(vec![Vec4::ONE, Vec4::ZERO])).unwrap(),
            ],
        };
        let text = ron::to_string(&clip).expect("saves");
        let back: AnimationClip = ron::from_str(&text).expect("loads");
        assert_eq!(back.tracks.len(), 3);
        assert_eq!(back.tracks[0].property, None);
        assert_eq!(back.tracks[1].property.as_deref(), Some("PointLight.intensity"));
        assert_eq!(back.tracks[1].target_name, "lamp");
        for (a, b) in clip.tracks.iter().zip(&back.tracks) {
            for t in [0.0, 0.3, 0.77, 1.5] {
                assert_eq!(a.sample(t), b.sample(t), "t = {t}");
            }
        }
    }
}

#[cfg(test)]
mod nan_sampling {
    use super::*;
    use gizmo_math::Vec3;

    fn track(timestamps: Vec<f32>) -> Track {
        let n = timestamps.len();
        Track {
            target_name: "probe".into(),
            property: None,
            keyframe_timestamps: timestamps,
            keyframes: Keyframes::Translation(vec![Vec3::ZERO; n]),
            interpolation: Interpolation::Linear,
            tangents: None,
        }
    }

    /// `Track`'s fields are public, so a caller can build a track `Track::new` would have
    /// rejected. `sample` was documented as bounds-checking its way out of that — but it
    /// clamped with `idx.clamp(1, len - 1)`, and `Ord::clamp` itself asserts `min <= max`.
    /// With a single keyframe that is `0.clamp(1, 0)`, reachable whenever a NaN makes both
    /// early-return comparisons false.
    ///
    /// Both directions matter: a NaN in the sampled time, and a NaN in the track's own
    /// timestamps (which is how a hand-built or deserialized track arrives broken).
    #[test]
    fn a_single_keyframe_track_does_not_panic_on_nan() {
        assert!(matches!(
            track(vec![0.0]).sample(f32::NAN),
            InterpolatedValue::Translation(_)
        ));
        assert!(matches!(
            track(vec![f32::NAN]).sample(0.5),
            InterpolatedValue::Translation(_)
        ));
    }

    /// The multi-keyframe path was never the broken one — pinned so the fix cannot be
    /// "mistakenly" generalised into an early return that swallows real segments.
    #[test]
    fn a_multi_keyframe_track_still_samples_its_segments() {
        let t = track(vec![0.0, 1.0, 2.0]);
        assert!(matches!(t.sample(f32::NAN), InterpolatedValue::Translation(_)));
        assert!(matches!(t.sample(0.5), InterpolatedValue::Translation(_)));
        assert!(matches!(t.sample(-1.0), InterpolatedValue::Translation(_)));
        assert!(matches!(t.sample(99.0), InterpolatedValue::Translation(_)));
    }

    /// An empty track still has no value to give.
    #[test]
    fn an_empty_track_still_returns_none() {
        assert!(matches!(track(vec![]).sample(0.0), InterpolatedValue::None));
        assert!(matches!(track(vec![]).sample(f32::NAN), InterpolatedValue::None));
    }
}
