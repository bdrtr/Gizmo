//! A surface built from nodes instead of written in WGSL.
//!
//! A [`ShaderGraph`] is what a node editor edits: nodes ([`NodeKind`]) with input and output slots,
//! and links from an output to an input. [`ShaderGraph::to_wgsl`] turns it into the
//! `material_surface` function a [`SurfaceMaterial`](crate::custom_material::SurfaceMaterial) is
//! built from, so a graph is drawn exactly as a hand-written surface is: through the G-buffer on the
//! desktop, by the lit forward shader in a browser and on a phone, with every light, shadow and
//! post effect the engine's own material gets. [`SurfaceMaterial::from_graph`](crate::custom_material::SurfaceMaterial::from_graph)
//! does both steps.
//!
//! # What a graph starts from
//!
//! Every graph has one [`NodeKind::Output`] node, and an input of it that nothing is linked to keeps
//! what the engine's own material gives there (`standard_surface`, what `MaterialType::Pbr` draws).
//! So [`ShaderGraph::new`] — the output and nothing linked — draws as `Pbr` does, to the byte, and a
//! graph that links only the colour keeps the material's normal map, roughness and the rest.
//!
//! # Types
//!
//! A value is a float or a 2-, 3- or 4-vector ([`ValueType`]). A link may join any two of them: a
//! float is spread over every component, a longer vector is cut short (`.xyz`), a shorter one is
//! padded with zeros. The math nodes take the widest of what reaches them ([`SlotType::Any`]), so
//! `Multiply` of a colour by a float is a colour.
//!
//! An input nothing is linked to holds a value of its own ([`GraphNode::values`]), a float spread
//! over the slot's width — or, for a slot whose default is something the fragment knows
//! ([`InputDefault`]), that: a texture lookup with nothing linked to its UV reads at the material's
//! UV.
//!
//! # Persistence
//!
//! A graph is `serde`: the editor saves it as it saves a scene. Links name nodes by [`NodeId`],
//! which stays the same when other nodes are removed. A graph read from a file is checked when it is
//! turned into WGSL — a link to a node that is not there, or a loop, is a [`GraphError`] then, not a
//! panic.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// A node of one [`ShaderGraph`]. Stable: removing another node does not change it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct NodeId(pub u32);

/// What flows along a link.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum ValueType {
    /// `f32`.
    Float,
    /// `vec2<f32>`.
    Vec2,
    /// `vec3<f32>`: a colour, a position, a direction.
    Vec3,
    /// `vec4<f32>`.
    Vec4,
}

impl ValueType {
    /// How many components it has, 1 to 4.
    #[must_use]
    pub fn width(self) -> usize {
        match self {
            Self::Float => 1,
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 => 4,
        }
    }

    fn of_width(width: usize) -> Self {
        match width {
            0 | 1 => Self::Float,
            2 => Self::Vec2,
            3 => Self::Vec3,
            _ => Self::Vec4,
        }
    }

    /// Its name in WGSL.
    #[must_use]
    pub fn wgsl(self) -> &'static str {
        match self {
            Self::Float => "f32",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
        }
    }
}

/// What a slot holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum SlotType {
    /// Always this type: what is linked to it is converted.
    Fixed(ValueType),
    /// The widest of the node's `Any` inputs: a math node's operands and its result.
    Any,
}

/// What an input nothing is linked to holds.
#[derive(Clone, Copy, PartialEq, Debug)]
#[non_exhaustive]
pub enum InputDefault {
    /// A value of its own, kept in [`GraphNode::values`] and editable there; this is where it starts.
    Value([f32; 4]),
    /// The mesh's UV, as the vertex carried it.
    Uv,
    /// The mesh's UV with the material's own scale, rotation and offset applied: where the engine's
    /// material reads its textures.
    MaterialUv,
    /// The surface normal in the world, unit length.
    WorldNormal,
    /// The fragment's position in the world.
    WorldPosition,
    /// An input of the output node: what the engine's material gives there.
    Engine,
}

/// An input slot of a node kind.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct InputSlot {
    /// What an editor labels it.
    pub name: &'static str,
    /// What it takes.
    pub ty: SlotType,
    /// What it holds while nothing is linked to it.
    pub default: InputDefault,
}

/// An output slot of a node kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutputSlot {
    /// What an editor labels it.
    pub name: &'static str,
    /// What it gives.
    pub ty: SlotType,
}

/// What a node does. The constants carry their value; every other kind's values are its inputs'.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum NodeKind {
    /// The surface: colour, alpha, roughness, metalness, emission and normal. One per graph.
    Output,

    // ── What the fragment knows ─────────────────────────────────────────────
    /// The mesh's texture coordinate.
    Uv,
    /// The fragment's position in the world.
    WorldPosition,
    /// The surface normal in the world, unit length.
    WorldNormal,
    /// From the fragment toward the camera, unit length.
    ViewDirection,
    /// The vertex colour.
    VertexColor,
    /// Seconds since the renderer started.
    Time,
    /// Where the camera is.
    CameraPosition,
    /// The fragment's pixel, in pixels from the target's top left.
    ScreenPosition,
    /// The instance's `Material` values: albedo, alpha, roughness, metalness, emission.
    Material,
    /// What the engine's own material gives at this fragment — textures, normal map and all.
    Standard,
    /// The material's base colour texture, at a UV (the material's own when nothing is linked).
    Texture,
    /// The material's normal map at a UV, turned into the world: the normal it gives, unit length.
    /// `strength` scales its tilt as the material's own normal scale does.
    NormalMap,
    /// The material's metallic-roughness map at a UV: roughness from green, metalness from blue.
    MetallicRoughnessMap,
    /// The material's emissive map at a UV.
    EmissiveMap,
    /// The material's occlusion map at a UV: 1 is unoccluded.
    OcclusionMap,

    // ── Constants ───────────────────────────────────────────────────────────
    /// A number.
    Float(f32),
    /// Two numbers.
    Vector2([f32; 2]),
    /// Three numbers.
    Vector3([f32; 3]),
    /// A colour in linear RGB, as `Material` takes one, and its alpha.
    Color([f32; 4]),

    // ── Math, component by component ────────────────────────────────────────
    /// `a + b`.
    Add,
    /// `a - b`.
    Subtract,
    /// `a * b`.
    Multiply,
    /// `a / b`.
    Divide,
    /// `pow(base, exponent)`.
    Power,
    /// The smaller of the two.
    Minimum,
    /// The larger of the two.
    Maximum,
    /// `a - b * floor(a / b)`: the remainder that repeats, also below zero.
    Modulo,
    /// Without its sign.
    Abs,
    /// Rounded down.
    Floor,
    /// Rounded up.
    Ceil,
    /// What is left after rounding down.
    Fract,
    /// The sine, in radians.
    Sin,
    /// The cosine, in radians.
    Cos,
    /// The square root.
    Sqrt,
    /// `e` to the power.
    Exp,
    /// `-in`.
    Negate,
    /// `1 - in`.
    OneMinus,
    /// Clamped to 0..1.
    Saturate,
    /// `mix(a, b, t)`.
    Lerp,
    /// Kept between `min` and `max`.
    Clamp,
    /// 0 below the edge, 1 from it on.
    Step,
    /// 0 below `edge0`, 1 above `edge1`, a smooth curve between.
    Smoothstep,
    /// From one range onto another.
    Remap,

    // ── Vectors ─────────────────────────────────────────────────────────────
    /// Unit length; a float becomes its sign.
    Normalize,
    /// How long it is; a float's is its size.
    Length,
    /// How far apart two points are.
    Distance,
    /// The dot product; two floats multiply.
    Dot,
    /// The cross product of two 3-vectors.
    Cross,
    /// A vector's components as floats; what it does not have reads 0.
    Split,
    /// Four floats as a vector, of each width.
    Combine,
    /// A tangent-space direction (a normal map's) in the world.
    TangentToWorld,

    // ── Patterns ────────────────────────────────────────────────────────────
    /// How much a surface faces away from the camera: 0 face-on, 1 at the rim.
    Fresnel,
    /// Squares of 0 and 1 over a UV.
    Checker,
    /// Smooth value noise in 0..1 over a position.
    Noise,
}

const fn value(name: &'static str, ty: ValueType, v: f32) -> InputSlot {
    InputSlot { name, ty: SlotType::Fixed(ty), default: InputDefault::Value([v, v, v, v]) }
}

const fn any(name: &'static str, v: f32) -> InputSlot {
    InputSlot { name, ty: SlotType::Any, default: InputDefault::Value([v, v, v, v]) }
}

const fn engine(name: &'static str, ty: ValueType) -> InputSlot {
    InputSlot { name, ty: SlotType::Fixed(ty), default: InputDefault::Engine }
}

const fn out(name: &'static str, ty: ValueType) -> OutputSlot {
    OutputSlot { name, ty: SlotType::Fixed(ty) }
}

const RESULT: &[OutputSlot] = &[OutputSlot { name: "result", ty: SlotType::Any }];
const FLOAT_RESULT: &[OutputSlot] = &[out("result", ValueType::Float)];
const UNARY: &[InputSlot] = &[any("in", 0.0)];

use ValueType::{Float as F, Vec2 as V2, Vec3 as V3, Vec4 as V4};

impl NodeKind {
    /// Every kind, the constants at their starting value: what a node editor offers to add.
    #[must_use]
    pub fn catalogue() -> Vec<NodeKind> {
        use NodeKind::*;
        vec![
            Uv, WorldPosition, WorldNormal, ViewDirection, VertexColor, Time, CameraPosition, ScreenPosition,
            Material, Standard, Texture, NormalMap, MetallicRoughnessMap, EmissiveMap, OcclusionMap,
            Float(1.0), Vector2([0.0, 0.0]), Vector3([0.0, 0.0, 0.0]), Color([1.0, 1.0, 1.0, 1.0]),
            Add, Subtract, Multiply, Divide, Power, Minimum, Maximum, Modulo,
            Abs, Floor, Ceil, Fract, Sin, Cos, Sqrt, Exp, Negate, OneMinus, Saturate,
            Lerp, Clamp, Step, Smoothstep, Remap,
            Normalize, Length, Distance, Dot, Cross, Split, Combine, TangentToWorld,
            Fresnel, Checker, Noise,
        ]
    }

    /// Which group an editor lists it under: `"Input"`, `"Constant"`, `"Math"`, `"Vector"`,
    /// `"Pattern"`, or `"Output"` for the surface itself.
    #[must_use]
    pub fn category(&self) -> &'static str {
        use NodeKind::*;
        match self {
            Output => "Output",
            Uv | WorldPosition | WorldNormal | ViewDirection | VertexColor | Time | CameraPosition | ScreenPosition
            | Material | Standard | Texture | NormalMap | MetallicRoughnessMap | EmissiveMap | OcclusionMap => "Input",
            Float(_) | Vector2(_) | Vector3(_) | Color(_) => "Constant",
            Add | Subtract | Multiply | Divide | Power | Minimum | Maximum | Modulo | Abs | Floor | Ceil | Fract
            | Sin | Cos | Sqrt | Exp | Negate | OneMinus | Saturate | Lerp | Clamp | Step | Smoothstep | Remap => {
                "Math"
            }
            Normalize | Length | Distance | Dot | Cross | Split | Combine | TangentToWorld => "Vector",
            Fresnel | Checker | Noise => "Pattern",
        }
    }

    /// Its name, as an editor shows it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        use NodeKind::*;
        match self {
            Output => "Surface",
            Uv => "UV",
            WorldPosition => "World Position",
            WorldNormal => "World Normal",
            ViewDirection => "View Direction",
            VertexColor => "Vertex Color",
            Time => "Time",
            CameraPosition => "Camera Position",
            ScreenPosition => "Screen Position",
            Material => "Material",
            Standard => "Standard Surface",
            Texture => "Base Texture",
            NormalMap => "Normal Map",
            MetallicRoughnessMap => "Metallic-Roughness Map",
            EmissiveMap => "Emissive Map",
            OcclusionMap => "Occlusion Map",
            Float(_) => "Float",
            Vector2(_) => "Vector2",
            Vector3(_) => "Vector3",
            Color(_) => "Color",
            Add => "Add",
            Subtract => "Subtract",
            Multiply => "Multiply",
            Divide => "Divide",
            Power => "Power",
            Minimum => "Minimum",
            Maximum => "Maximum",
            Modulo => "Modulo",
            Abs => "Abs",
            Floor => "Floor",
            Ceil => "Ceil",
            Fract => "Fract",
            Sin => "Sin",
            Cos => "Cos",
            Sqrt => "Sqrt",
            Exp => "Exp",
            Negate => "Negate",
            OneMinus => "One Minus",
            Saturate => "Saturate",
            Lerp => "Lerp",
            Clamp => "Clamp",
            Step => "Step",
            Smoothstep => "Smoothstep",
            Remap => "Remap",
            Normalize => "Normalize",
            Length => "Length",
            Distance => "Distance",
            Dot => "Dot",
            Cross => "Cross",
            Split => "Split",
            Combine => "Combine",
            TangentToWorld => "Tangent To World",
            Fresnel => "Fresnel",
            Checker => "Checker",
            Noise => "Noise",
        }
    }

    /// Its input slots, in order: a link's `input` indexes this.
    #[must_use]
    pub fn inputs(&self) -> &'static [InputSlot] {
        use NodeKind::*;
        match self {
            Output => const { &[
                engine("albedo", V3),
                engine("alpha", F),
                engine("roughness", F),
                engine("metallic", F),
                engine("emissive", V3),
                engine("normal", V3),
            ] },
            Uv | WorldPosition | WorldNormal | ViewDirection | VertexColor | Time | CameraPosition
            | ScreenPosition | Material | Standard | Float(_) | Vector2(_) | Vector3(_) | Color(_) => &[],
            Texture => const { &[InputSlot { name: "uv", ty: SlotType::Fixed(V2), default: InputDefault::MaterialUv }] },
            NormalMap => const {
                &[
                    InputSlot { name: "uv", ty: SlotType::Fixed(V2), default: InputDefault::MaterialUv },
                    value("strength", F, 1.0),
                ]
            },
            MetallicRoughnessMap | EmissiveMap | OcclusionMap => const {
                &[InputSlot { name: "uv", ty: SlotType::Fixed(V2), default: InputDefault::MaterialUv }]
            },
            Add | Subtract => const { &[any("a", 0.0), any("b", 0.0)] },
            Multiply | Divide => const { &[any("a", 1.0), any("b", 1.0)] },
            Power => const { &[any("base", 1.0), any("exponent", 1.0)] },
            Minimum | Maximum => const { &[any("a", 0.0), any("b", 0.0)] },
            Modulo => const { &[any("a", 0.0), any("b", 1.0)] },
            Abs | Floor | Ceil | Fract | Sin | Cos | Sqrt | Exp | Negate | OneMinus | Saturate | Normalize
            | Length => UNARY,
            Lerp => const { &[any("a", 0.0), any("b", 1.0), any("t", 0.5)] },
            Clamp => const { &[any("in", 0.0), any("min", 0.0), any("max", 1.0)] },
            Step => const { &[any("edge", 0.5), any("in", 0.0)] },
            Smoothstep => const { &[any("edge0", 0.0), any("edge1", 1.0), any("in", 0.0)] },
            Remap => const { &[any("in", 0.0), any("from min", 0.0), any("from max", 1.0), any("to min", 0.0), any("to max", 1.0)] },
            Distance | Dot => const { &[any("a", 0.0), any("b", 0.0)] },
            Cross => const { &[value("a", V3, 0.0), value("b", V3, 0.0)] },
            Split => UNARY,
            Combine => const { &[value("x", F, 0.0), value("y", F, 0.0), value("z", F, 0.0), value("w", F, 0.0)] },
            TangentToWorld => const { &[InputSlot {
                name: "tangent",
                ty: SlotType::Fixed(V3),
                default: InputDefault::Value([0.0, 0.0, 1.0, 0.0]),
            }] },
            Fresnel => const { &[
                value("power", F, 5.0),
                InputSlot { name: "normal", ty: SlotType::Fixed(V3), default: InputDefault::WorldNormal },
            ] },
            Checker => const { &[
                InputSlot { name: "uv", ty: SlotType::Fixed(V2), default: InputDefault::Uv },
                value("scale", F, 8.0),
            ] },
            Noise => const { &[
                InputSlot { name: "position", ty: SlotType::Fixed(V3), default: InputDefault::WorldPosition },
                value("scale", F, 1.0),
            ] },
        }
    }

    /// Its output slots, in order: a link's `output` indexes this.
    #[must_use]
    pub fn outputs(&self) -> &'static [OutputSlot] {
        use NodeKind::*;
        match self {
            Output => &[],
            Uv | ScreenPosition => const { &[out("uv", V2)] },
            WorldPosition | CameraPosition => const { &[out("position", V3)] },
            WorldNormal => const { &[out("normal", V3)] },
            ViewDirection => const { &[out("direction", V3)] },
            VertexColor => const { &[out("color", V3)] },
            Time => const { &[out("seconds", F)] },
            Material => const { &[
                out("albedo", V3),
                out("alpha", F),
                out("roughness", F),
                out("metallic", F),
                out("emissive", V3),
            ] },
            Standard => const { &[
                out("albedo", V3),
                out("alpha", F),
                out("roughness", F),
                out("metallic", F),
                out("emissive", V3),
                out("normal", V3),
            ] },
            Texture => const { &[out("rgb", V3), out("a", F)] },
            NormalMap => const { &[out("normal", V3)] },
            MetallicRoughnessMap => const { &[out("roughness", F), out("metallic", F)] },
            EmissiveMap => const { &[out("rgb", V3)] },
            OcclusionMap => const { &[out("occlusion", F)] },
            Float(_) => const { &[out("value", F)] },
            Vector2(_) => const { &[out("value", V2)] },
            Vector3(_) => const { &[out("value", V3)] },
            Color(_) => const { &[out("rgb", V3), out("a", F)] },
            Add | Subtract | Multiply | Divide | Power | Minimum | Maximum | Modulo | Abs | Floor | Ceil
            | Fract | Sin | Cos | Sqrt | Exp | Negate | OneMinus | Saturate | Lerp | Clamp | Step
            | Smoothstep | Remap | Normalize => RESULT,
            Length | Distance | Dot | Fresnel | Checker | Noise => FLOAT_RESULT,
            Cross | TangentToWorld => const { &[out("result", V3)] },
            Split => const { &[out("x", F), out("y", F), out("z", F), out("w", F)] },
            Combine => const { &[out("xyzw", V4), out("xyz", V3), out("xy", V2)] },
        }
    }
}

/// A node of a graph: what it is, where the editor drew it, and what its unlinked inputs hold.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    /// Its id in the graph.
    pub id: NodeId,
    /// What it does.
    pub kind: NodeKind,
    /// Where it sits in the editor's canvas. Nothing else reads it.
    pub position: [f32; 2],
    /// One per input slot: the value an unlinked [`InputDefault::Value`] input holds. Only the first
    /// `width` components are read; a slot that takes [`SlotType::Any`] reads the first, as a float.
    /// Missing entries (a file from a version with fewer inputs) read as the slot's default.
    pub values: Vec<[f32; 4]>,
}

impl GraphNode {
    /// What unlinked input `slot` holds, if it holds a value of its own.
    #[must_use]
    pub fn value(&self, slot: usize) -> Option<[f32; 4]> {
        match self.kind.inputs().get(slot)?.default {
            InputDefault::Value(default) => Some(self.values.get(slot).copied().unwrap_or(default)),
            _ => None,
        }
    }
}

/// From output `output` of node `from` to input `input` of node `to`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Link {
    /// The node it starts from.
    pub from: NodeId,
    /// Which of that node's outputs.
    pub output: usize,
    /// The node it ends in.
    pub to: NodeId,
    /// Which of that node's inputs.
    pub input: usize,
}

/// Why a graph could not be edited as asked, or turned into WGSL.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum GraphError {
    /// No [`NodeKind::Output`] node.
    NoOutput,
    /// More than one [`NodeKind::Output`] node: which one is the surface?
    SeveralOutputs,
    /// The output node cannot be removed.
    OutputRemoved,
    /// No node has this id.
    NoSuchNode(NodeId),
    /// The node has no such input (`input: true`) or output slot.
    NoSuchSlot {
        /// The node.
        node: NodeId,
        /// The slot asked for.
        slot: usize,
        /// An input slot, or an output one.
        input: bool,
    },
    /// Two links end in this input.
    InputLinkedTwice {
        /// The node.
        node: NodeId,
        /// Its input.
        input: usize,
    },
    /// The links run in a loop through this node, so it would need its own result to compute it.
    Cycle(NodeId),
    /// A value of this node is NaN or infinite, which WGSL cannot write.
    NotFinite(NodeId),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOutput => write!(f, "the graph has no surface output node"),
            Self::SeveralOutputs => write!(f, "the graph has more than one surface output node"),
            Self::OutputRemoved => write!(f, "the surface output node cannot be removed"),
            Self::NoSuchNode(id) => write!(f, "no node {}", id.0),
            Self::NoSuchSlot { node, slot, input } => {
                write!(f, "node {} has no {} slot {slot}", node.0, if *input { "input" } else { "output" })
            }
            Self::InputLinkedTwice { node, input } => write!(f, "input {input} of node {} has two links", node.0),
            Self::Cycle(id) => write!(f, "the links run in a loop through node {}", id.0),
            Self::NotFinite(id) => write!(f, "node {} holds a value that is not a finite number", id.0),
        }
    }
}

impl std::error::Error for GraphError {}

/// A surface as nodes and links — see the [module docs](self).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ShaderGraph {
    nodes: Vec<GraphNode>,
    links: Vec<Link>,
}

impl Default for ShaderGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ShaderGraph {
    /// The output node alone: draws as the engine's material does.
    #[must_use]
    pub fn new() -> Self {
        let output = GraphNode { id: NodeId(0), kind: NodeKind::Output, position: [0.0, 0.0], values: Vec::new() };
        Self { nodes: vec![output], links: Vec::new() }
    }

    /// Every node, in the order they were added.
    #[must_use]
    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    /// Every link.
    #[must_use]
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// The node with this id.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// A node to move or to change a value of. Its kind may change too: links to slots the new kind
    /// does not have are a [`GraphError::NoSuchSlot`] at [`to_wgsl`](Self::to_wgsl) — use
    /// [`set_kind`](Self::set_kind), which drops them.
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut GraphNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// The output node, if the graph has exactly one.
    pub fn output(&self) -> Result<NodeId, GraphError> {
        let mut outputs = self.nodes.iter().filter(|n| n.kind == NodeKind::Output);
        let first = outputs.next().ok_or(GraphError::NoOutput)?;
        if outputs.next().is_some() {
            return Err(GraphError::SeveralOutputs);
        }
        Ok(first.id)
    }

    /// Adds a node and returns its id. A second [`NodeKind::Output`] is added as asked, and makes
    /// the graph a [`GraphError::SeveralOutputs`] until one is removed.
    pub fn add(&mut self, kind: NodeKind, position: [f32; 2]) -> NodeId {
        let id = NodeId(self.nodes.iter().map(|n| n.id.0 + 1).max().unwrap_or(0));
        let values = kind
            .inputs()
            .iter()
            .map(|slot| match slot.default {
                InputDefault::Value(v) => v,
                _ => [0.0; 4],
            })
            .collect();
        self.nodes.push(GraphNode { id, kind, position, values });
        id
    }

    /// Removes a node and every link to or from it.
    ///
    /// # Errors
    ///
    /// [`GraphError::OutputRemoved`] for the output node, [`GraphError::NoSuchNode`] for an id that is
    /// not in the graph.
    pub fn remove(&mut self, id: NodeId) -> Result<(), GraphError> {
        let index = self.nodes.iter().position(|n| n.id == id).ok_or(GraphError::NoSuchNode(id))?;
        if self.nodes[index].kind == NodeKind::Output && self.output() == Ok(id) {
            return Err(GraphError::OutputRemoved);
        }
        self.nodes.remove(index);
        self.links.retain(|l| l.from != id && l.to != id);
        Ok(())
    }

    /// Changes what a node is, keeping its links where the new kind has the slot and dropping the rest.
    ///
    /// # Errors
    ///
    /// [`GraphError::NoSuchNode`].
    pub fn set_kind(&mut self, id: NodeId, kind: NodeKind) -> Result<(), GraphError> {
        let inputs = kind.inputs().len();
        let outputs = kind.outputs().len();
        let node = self.node_mut(id).ok_or(GraphError::NoSuchNode(id))?;
        let old = node.kind.inputs();
        node.values.resize(inputs, [0.0; 4]);
        // A slot that is the same as before keeps its value; any other starts from its default.
        for (index, (slot, value)) in kind.inputs().iter().zip(node.values.iter_mut()).enumerate() {
            if old.get(index) != Some(slot) {
                *value = match slot.default {
                    InputDefault::Value(default) => default,
                    _ => [0.0; 4],
                };
            }
        }
        node.kind = kind;
        self.links.retain(|l| !(l.to == id && l.input >= inputs) && !(l.from == id && l.output >= outputs));
        Ok(())
    }

    /// Links output `output` of `from` to input `input` of `to`, replacing whatever was linked there.
    ///
    /// # Errors
    ///
    /// [`GraphError::NoSuchNode`], [`GraphError::NoSuchSlot`], and [`GraphError::Cycle`] when `from`
    /// already depends on `to` (or is `to`): the graph is left as it was.
    pub fn connect(&mut self, from: NodeId, output: usize, to: NodeId, input: usize) -> Result<(), GraphError> {
        let source = self.node(from).ok_or(GraphError::NoSuchNode(from))?;
        if output >= source.kind.outputs().len() {
            return Err(GraphError::NoSuchSlot { node: from, slot: output, input: false });
        }
        let target = self.node(to).ok_or(GraphError::NoSuchNode(to))?;
        if input >= target.kind.inputs().len() {
            return Err(GraphError::NoSuchSlot { node: to, slot: input, input: true });
        }
        if self.depends_on(from, to) {
            return Err(GraphError::Cycle(to));
        }
        self.links.retain(|l| !(l.to == to && l.input == input));
        self.links.push(Link { from, output, to, input });
        Ok(())
    }

    /// Removes the link into input `input` of `to`, returning it.
    pub fn disconnect(&mut self, to: NodeId, input: usize) -> Option<Link> {
        let index = self.links.iter().position(|l| l.to == to && l.input == input)?;
        Some(self.links.remove(index))
    }

    /// The link into input `input` of `to`, if there is one.
    #[must_use]
    pub fn link_into(&self, to: NodeId, input: usize) -> Option<&Link> {
        self.links.iter().find(|l| l.to == to && l.input == input)
    }

    /// Whether `node`'s value is computed from `upstream`'s (or `node` is `upstream`).
    fn depends_on(&self, node: NodeId, upstream: NodeId) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n == upstream {
                return true;
            }
            if seen.insert(n) {
                stack.extend(self.links.iter().filter(|l| l.to == n).map(|l| l.from));
            }
        }
        false
    }

    /// The type each output of each node the surface reads carries, after the `Any` slots take the
    /// width of what reaches them. A node the output does not reach is not in it.
    ///
    /// # Errors
    ///
    /// As [`to_wgsl`](Self::to_wgsl).
    pub fn output_types(&self) -> Result<HashMap<(NodeId, usize), ValueType>, GraphError> {
        Ok(self.compile()?.types)
    }

    /// The `material_surface` function this graph is, with its imports: what
    /// [`SurfaceMaterial::from_wgsl`](crate::custom_material::SurfaceMaterial::from_wgsl) takes.
    /// The same graph gives the same text.
    ///
    /// # Errors
    ///
    /// A [`GraphError`] for a graph that is not one surface: no output or two, a link to a node or
    /// slot that is not there, two links into one input, a loop, a value that is not finite.
    pub fn to_wgsl(&self) -> Result<String, GraphError> {
        Ok(self.compile()?.wgsl)
    }

    fn compile(&self) -> Result<Compiled, GraphError> {
        let output = self.output()?;
        let mut into: HashMap<(NodeId, usize), Link> = HashMap::new();
        for link in &self.links {
            let source = self.node(link.from).ok_or(GraphError::NoSuchNode(link.from))?;
            if link.output >= source.kind.outputs().len() {
                return Err(GraphError::NoSuchSlot { node: link.from, slot: link.output, input: false });
            }
            let target = self.node(link.to).ok_or(GraphError::NoSuchNode(link.to))?;
            if link.input >= target.kind.inputs().len() {
                return Err(GraphError::NoSuchSlot { node: link.to, slot: link.input, input: true });
            }
            if into.insert((link.to, link.input), *link).is_some() {
                return Err(GraphError::InputLinkedTwice { node: link.to, input: link.input });
            }
        }

        // Every node the output reads, each after what it reads.
        let mut order = Vec::new();
        let mut state: HashMap<NodeId, bool> = HashMap::new(); // false = on the path, true = done
        self.visit(output, &into, &mut state, &mut order)?;

        let mut compiled = Compiled { types: HashMap::new(), wgsl: String::new() };
        let mut body = String::new();
        let mut noise = false;
        for id in order {
            let node = self.node(id).ok_or(GraphError::NoSuchNode(id))?;
            let inputs = node.kind.inputs();
            // Each input's expression and type before conversion.
            let mut raw: Vec<Option<(String, ValueType)>> = Vec::with_capacity(inputs.len());
            for (slot_index, slot) in inputs.iter().enumerate() {
                raw.push(match into.get(&(id, slot_index)) {
                    Some(link) => {
                        let ty = compiled.types[&(link.from, link.output)];
                        Some((var(link.from, link.output), ty))
                    }
                    None => match slot.default {
                        InputDefault::Value(_) => {
                            let v = node.value(slot_index).unwrap_or([0.0; 4]);
                            let ty = match slot.ty {
                                SlotType::Fixed(t) => t,
                                SlotType::Any => ValueType::Float,
                            };
                            Some((literal(&v[..ty.width()], id)?, ty))
                        }
                        InputDefault::Uv => Some(("input.uv".to_string(), ValueType::Vec2)),
                        InputDefault::MaterialUv => Some(("surface_uv(input.uv)".to_string(), ValueType::Vec2)),
                        InputDefault::WorldNormal => Some(("normalize(input.N)".to_string(), ValueType::Vec3)),
                        InputDefault::WorldPosition => Some(("input.world_position".to_string(), ValueType::Vec3)),
                        InputDefault::Engine => None,
                    },
                });
            }
            let width = inputs
                .iter()
                .zip(&raw)
                .filter(|(slot, _)| slot.ty == SlotType::Any)
                .filter_map(|(_, r)| r.as_ref().map(|(_, t)| t.width()))
                .max()
                .unwrap_or(1);
            let any = ValueType::of_width(width);
            let args: Vec<String> = inputs
                .iter()
                .zip(&raw)
                .map(|(slot, r)| match r {
                    Some((expr, from)) => {
                        let to = match slot.ty {
                            SlotType::Fixed(t) => t,
                            SlotType::Any => any,
                        };
                        convert(expr, *from, to)
                    }
                    None => String::new(),
                })
                .collect();

            if node.kind == NodeKind::Output {
                for (slot, (arg, r)) in inputs.iter().zip(args.iter().zip(&raw)) {
                    if r.is_none() {
                        continue;
                    }
                    match slot.name {
                        "albedo" => {
                            let _ = writeln!(body, "    surface.base_color = {arg};\n    surface.albedo = {arg};");
                        }
                        "normal" => {
                            let _ = writeln!(body, "    surface.N = normalize({arg});");
                        }
                        // The ranges the engine's material keeps them in: a roughness of 0 is a
                        // GGX lobe of zero width, and a metalness outside 0..1 is no material.
                        "roughness" => {
                            let _ = writeln!(body, "    surface.roughness = clamp({arg}, 0.05, 1.0);");
                        }
                        "metallic" => {
                            let _ = writeln!(body, "    surface.metallic = saturate({arg});");
                        }
                        field => {
                            let _ = writeln!(body, "    surface.{field} = {arg};");
                        }
                    }
                }
                continue;
            }

            noise |= node.kind == NodeKind::Noise;
            let exprs = node_exprs(node, id, &args, any, &mut body)?;
            for (index, (slot, expr)) in node.kind.outputs().iter().zip(exprs).enumerate() {
                let ty = match slot.ty {
                    SlotType::Fixed(t) => t,
                    SlotType::Any => any,
                };
                let _ = writeln!(body, "    let {}: {} = {expr};", var(id, index), ty.wgsl());
                compiled.types.insert((id, index), ty);
            }
        }

        let mut wgsl = String::from(
            "// Generated from a shader graph (gizmo_renderer::shader_graph).\n\
             #import gizmo::surface::{Surface, SurfaceInput}\n\
             #import gizmo::material::{standard_surface, surface_uv, t_diffuse, s_diffuse, t_normal, t_mr, t_emissive, t_ao}\n\n",
        );
        if noise {
            wgsl.push_str(NOISE_WGSL);
        }
        wgsl.push_str("fn material_surface(input: SurfaceInput) -> Surface {\n    var surface = standard_surface(input);\n");
        wgsl.push_str(&body);
        wgsl.push_str("    return surface;\n}\n");
        compiled.wgsl = wgsl;
        Ok(compiled)
    }

    fn visit(
        &self,
        id: NodeId,
        into: &HashMap<(NodeId, usize), Link>,
        state: &mut HashMap<NodeId, bool>,
        order: &mut Vec<NodeId>,
    ) -> Result<(), GraphError> {
        match state.get(&id) {
            Some(true) => return Ok(()),
            Some(false) => return Err(GraphError::Cycle(id)),
            None => {}
        }
        state.insert(id, false);
        let node = self.node(id).ok_or(GraphError::NoSuchNode(id))?;
        for slot in 0..node.kind.inputs().len() {
            if let Some(link) = into.get(&(id, slot)) {
                self.visit(link.from, into, state, order)?;
            }
        }
        state.insert(id, true);
        order.push(id);
        Ok(())
    }
}

/// A material built from nodes, as a component: an entity with a `Material` and this is drawn with
/// the graph's surface.
///
/// The graph is the authored thing and travels in a scene file (`gizmo-app` registers it as
/// `"SurfaceGraph"`); the [`SurfaceId`](crate::custom_material::SurfaceId) it is drawn with is the
/// renderer's and does not. [`resolve_surface_graphs`] — part of every frame the engine prepares —
/// builds the surface when the graph is new or has changed, sets the entity's
/// [`MaterialType::CustomSurface`](crate::components::MaterialType::CustomSurface) to it, and puts it
/// back after anything else writes the material type, so a scene that loads a stale id draws the
/// graph. A graph that is not one surface draws the engine's material and says why once. Removing
/// the component gives the entity the engine's material back.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct SurfaceGraph {
    /// The graph.
    pub graph: ShaderGraph,
}

impl SurfaceGraph {
    /// A component drawing `graph`.
    #[must_use]
    pub fn new(graph: ShaderGraph) -> Self {
        Self { graph }
    }
}

/// What [`resolve_surface_graphs`] last built an entity's graph into: the graph, to tell when it
/// changes, and the surface, `None` when it did not build.
#[derive(Clone)]
struct ResolvedSurfaceGraph {
    graph: ShaderGraph,
    surface: Option<crate::custom_material::SurfaceId>,
}

gizmo_core::impl_component!(SurfaceGraph, ResolvedSurfaceGraph);

/// Gives every entity with a [`SurfaceGraph`] and a `Material` its graph's surface — see
/// [`SurfaceGraph`]. A graph's surface is built once per distinct graph and shared; one no entity
/// draws any more is released, pipelines and all.
pub fn resolve_surface_graphs(world: &mut gizmo_core::World, renderer: &mut crate::Renderer) {
    use crate::components::{Material, MaterialType};
    use gizmo_core::Entity;

    // The graphs that are new or changed since they were last built.
    let stale: Vec<(Entity, ShaderGraph)> = {
        let resolved = world.query::<&ResolvedSurfaceGraph>();
        world
            .query::<(Entity, &SurfaceGraph)>()
            .iter()
            .filter(|(e, g)| resolved.get(*e).is_none_or(|r| r.graph != g.graph))
            .map(|(e, g)| (e, g.graph.clone()))
            .collect()
    };
    for (entity, graph) in stale {
        let previous = world.query::<&ResolvedSurfaceGraph>().get(entity).and_then(|r| r.surface);
        let surface = match renderer.surface_for_graph(&graph) {
            Ok(id) => Some(id),
            Err(error) => {
                tracing::warn!(entity = entity.id(), %error, "[SurfaceGraph] the graph does not build; drawing the engine's material");
                None
            }
        };
        // A graph that stopped building gives back the engine's material, not a released surface.
        if surface.is_none() {
            if let (Some(old), Some(mut material)) = (previous, world.query_mut::<&mut Material>().get_mut(entity)) {
                if material.material_type == MaterialType::CustomSurface(old) {
                    material.material_type = MaterialType::Pbr;
                }
            }
        }
        world.add_component(entity, ResolvedSurfaceGraph { graph, surface });
    }

    // A graph taken off an entity takes its surface with it.
    let removed: Vec<(Entity, Option<crate::custom_material::SurfaceId>)> = {
        let graphs = world.query::<&SurfaceGraph>();
        world
            .query::<(Entity, &ResolvedSurfaceGraph)>()
            .iter()
            .filter(|(e, _)| graphs.get(*e).is_none())
            .map(|(e, r)| (e, r.surface))
            .collect()
    };
    for (entity, surface) in removed {
        if let (Some(id), Some(mut material)) = (surface, world.query_mut::<&mut Material>().get_mut(entity)) {
            if material.material_type == MaterialType::CustomSurface(id) {
                material.material_type = MaterialType::Pbr;
            }
        }
        world.remove_component::<ResolvedSurfaceGraph>(entity);
    }

    // Every material drawn by a graph says so, whatever wrote its type since.
    let mut used = std::collections::HashSet::new();
    {
        let resolved: Vec<(Entity, crate::custom_material::SurfaceId)> = world
            .query::<(Entity, &ResolvedSurfaceGraph)>()
            .iter()
            .filter_map(|(e, r)| r.surface.map(|s| (e, s)))
            .collect();
        let mut materials = world.query_mut::<&mut Material>();
        for (entity, id) in resolved {
            used.insert(id);
            if let Some(mut material) = materials.get_mut(entity) {
                if material.material_type != MaterialType::CustomSurface(id) {
                    material.material_type = MaterialType::CustomSurface(id);
                }
            }
        }
    }
    renderer.custom_materials.release_graph_surfaces_except(&used);
}

struct Compiled {
    types: HashMap<(NodeId, usize), ValueType>,
    wgsl: String,
}

/// Value noise, one lattice hash per corner. Only in a graph that has a [`NodeKind::Noise`].
const NOISE_WGSL: &str = "fn shader_graph_hash(p: vec3<f32>) -> f32 {
    let q = fract(p * 0.3183099 + vec3<f32>(0.1, 0.2, 0.3)) * 17.0;
    return fract(q.x * q.y * q.z * (q.x + q.y + q.z));
}

fn shader_graph_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(shader_graph_hash(i), shader_graph_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x10 = mix(shader_graph_hash(i + vec3<f32>(0.0, 1.0, 0.0)), shader_graph_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x01 = mix(shader_graph_hash(i + vec3<f32>(0.0, 0.0, 1.0)), shader_graph_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x11 = mix(shader_graph_hash(i + vec3<f32>(0.0, 1.0, 1.0)), shader_graph_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

";

fn var(node: NodeId, output: usize) -> String {
    format!("n{}_{output}", node.0)
}

/// A float as WGSL writes one: always with a point or an exponent, and an `f`.
fn float(v: f32, node: NodeId) -> Result<String, GraphError> {
    if !v.is_finite() {
        return Err(GraphError::NotFinite(node));
    }
    Ok(format!("{v:?}f"))
}

fn literal(values: &[f32], node: NodeId) -> Result<String, GraphError> {
    let parts = values.iter().map(|v| float(*v, node)).collect::<Result<Vec<_>, _>>()?;
    Ok(match parts.len() {
        1 => parts[0].clone(),
        n => format!("vec{n}<f32>({})", parts.join(", ")),
    })
}

/// `expr`, of type `from`, as a `to`: a float spread over every component, a longer vector cut short,
/// a shorter one padded with zeros.
fn convert(expr: &str, from: ValueType, to: ValueType) -> String {
    let (a, b) = (from.width(), to.width());
    if a == b {
        expr.to_string()
    } else if a == 1 {
        format!("{}({expr})", to.wgsl())
    } else if b == 1 {
        format!("({expr}).x")
    } else if a > b {
        format!("({expr}).{}", &"xyzw"[..b])
    } else {
        format!("{}({expr}{})", to.wgsl(), ", 0.0".repeat(b - a))
    }
}

/// The expressions a node's outputs are, from its converted inputs; statements it needs first go
/// into `body`.
fn node_exprs(node: &GraphNode, id: NodeId, a: &[String], any: ValueType, body: &mut String) -> Result<Vec<String>, GraphError> {
    use NodeKind::*;
    let one = |s: &str| vec![s.to_string()];
    let vector = any.width() > 1;
    Ok(match &node.kind {
        Output => Vec::new(),
        Uv => one("input.uv"),
        WorldPosition => one("input.world_position"),
        WorldNormal => one("normalize(input.N)"),
        ViewDirection => one("normalize(input.camera_position - input.world_position)"),
        VertexColor => one("input.vertex_color"),
        Time => one("input.time"),
        CameraPosition => one("input.camera_position"),
        ScreenPosition => one("input.frag_coord.xy"),
        Material => vec![
            "input.albedo.rgb".into(),
            "input.albedo.a".into(),
            "input.roughness".into(),
            "input.metallic".into(),
            "input.emissive".into(),
        ],
        Standard => vec![
            "surface.albedo".into(),
            "surface.alpha".into(),
            "surface.roughness".into(),
            "surface.metallic".into(),
            "surface.emissive".into(),
            "surface.N".into(),
        ],
        Texture => {
            let _ = writeln!(body, "    let n{}_texel = textureSample(t_diffuse, s_diffuse, {});", id.0, a[0]);
            vec![format!("n{}_texel.rgb", id.0), format!("n{}_texel.a", id.0)]
        }
        // As the engine's material reads it (`gizmo::material::evaluate_surface`): a flat map is the
        // geometric normal, and a degenerate sum comes out dark rather than NaN.
        NormalMap => {
            let _ = writeln!(
                body,
                "    let n{0}_ts = textureSample(t_normal, s_diffuse, {1}).xyz * 2.0 - 1.0;\n    \
                 let n{0}_w = input.T * (n{0}_ts.x * {2}) + input.B * (n{0}_ts.y * {2}) + input.N * max(n{0}_ts.z, 1e-4);",
                id.0, a[0], a[1]
            );
            vec![format!("n{0}_w / max(length(n{0}_w), 1e-6)", id.0)]
        }
        MetallicRoughnessMap => {
            let _ = writeln!(body, "    let n{}_mr = textureSample(t_mr, s_diffuse, {});", id.0, a[0]);
            vec![format!("n{}_mr.g", id.0), format!("n{}_mr.b", id.0)]
        }
        EmissiveMap => vec![format!("textureSample(t_emissive, s_diffuse, {}).rgb", a[0])],
        OcclusionMap => vec![format!("textureSample(t_ao, s_diffuse, {}).r", a[0])],
        Float(v) => vec![literal(&[*v], id)?],
        Vector2(v) => vec![literal(v, id)?],
        Vector3(v) => vec![literal(v, id)?],
        Color(c) => vec![literal(&c[..3], id)?, literal(&c[3..], id)?],
        Add => vec![format!("{} + {}", a[0], a[1])],
        Subtract => vec![format!("{} - {}", a[0], a[1])],
        Multiply => vec![format!("{} * {}", a[0], a[1])],
        Divide => vec![format!("{} / {}", a[0], a[1])],
        Power => vec![format!("pow({}, {})", a[0], a[1])],
        Minimum => vec![format!("min({}, {})", a[0], a[1])],
        Maximum => vec![format!("max({}, {})", a[0], a[1])],
        Modulo => vec![format!("({0}) - ({1}) * floor(({0}) / ({1}))", a[0], a[1])],
        Abs => vec![format!("abs({})", a[0])],
        Floor => vec![format!("floor({})", a[0])],
        Ceil => vec![format!("ceil({})", a[0])],
        Fract => vec![format!("fract({})", a[0])],
        Sin => vec![format!("sin({})", a[0])],
        Cos => vec![format!("cos({})", a[0])],
        Sqrt => vec![format!("sqrt({})", a[0])],
        Exp => vec![format!("exp({})", a[0])],
        Negate => vec![format!("-({})", a[0])],
        OneMinus => vec![format!("1.0 - ({})", a[0])],
        Saturate => vec![format!("saturate({})", a[0])],
        Lerp => vec![format!("mix({}, {}, {})", a[0], a[1], a[2])],
        Clamp => vec![format!("clamp({}, {}, {})", a[0], a[1], a[2])],
        Step => vec![format!("step({}, {})", a[0], a[1])],
        Smoothstep => vec![format!("smoothstep({}, {}, {})", a[0], a[1], a[2])],
        Remap => vec![format!(
            "({3}) + (({0}) - ({1})) / (({2}) - ({1})) * (({4}) - ({3}))",
            a[0], a[1], a[2], a[3], a[4]
        )],
        // A float has no direction but its sign, and its length is its size.
        Normalize if vector => vec![format!("normalize({})", a[0])],
        Normalize => vec![format!("sign({})", a[0])],
        Length if vector => vec![format!("length({})", a[0])],
        Length => vec![format!("abs({})", a[0])],
        Distance if vector => vec![format!("distance({}, {})", a[0], a[1])],
        Distance => vec![format!("abs({} - {})", a[0], a[1])],
        Dot if vector => vec![format!("dot({}, {})", a[0], a[1])],
        Dot => vec![format!("{} * {}", a[0], a[1])],
        Cross => vec![format!("cross({}, {})", a[0], a[1])],
        Split => {
            let _ = writeln!(body, "    let n{}_in = {};", id.0, a[0]);
            (0..4)
                .map(|i| match (any.width(), i) {
                    (1, 0) => format!("n{}_in", id.0),
                    (w, i) if i < w => format!("n{}_in.{}", id.0, &"xyzw"[i..=i]),
                    _ => "0.0".to_string(),
                })
                .collect()
        }
        Combine => vec![
            format!("vec4<f32>({}, {}, {}, {})", a[0], a[1], a[2], a[3]),
            format!("vec3<f32>({}, {}, {})", a[0], a[1], a[2]),
            format!("vec2<f32>({}, {})", a[0], a[1]),
        ],
        TangentToWorld => {
            let _ = writeln!(body, "    let n{}_t = {};", id.0, a[0]);
            vec![format!(
                "normalize(n{0}_t.x * input.T + n{0}_t.y * input.B + n{0}_t.z * input.N)",
                id.0
            )]
        }
        Fresnel => vec![format!(
            "pow(1.0 - saturate(dot({}, normalize(input.camera_position - input.world_position))), {})",
            a[1], a[0]
        )],
        Checker => {
            let _ = writeln!(body, "    let n{}_cell = floor({} * {});", id.0, a[0], a[1]);
            vec![format!(
                "(n{0}_cell.x + n{0}_cell.y) - 2.0 * floor((n{0}_cell.x + n{0}_cell.y) * 0.5)",
                id.0
            )]
        }
        Noise => vec![format!("shader_graph_noise({} * {})", a[0], a[1])],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::shaders::{native_render_defs, try_compose_wgsl, web_render_defs};
    use naga_oil::compose::ShaderDefValue::Bool;

    /// Every shader a surface is composed into, on both paths, as `custom_material`'s own test lists.
    fn compose_everywhere(wgsl: &str) -> Vec<String> {
        let gbuffer = include_str!("shaders/gbuffer.wgsl");
        let lit = include_str!("shaders/shader.wgsl");
        let with = |mut defs: HashMap<String, naga_oil::compose::ShaderDefValue>, extra: &[&str]| {
            defs.insert("CUSTOM_SURFACE".to_string(), Bool(true));
            for def in extra {
                defs.insert((*def).to_string(), Bool(true));
            }
            defs
        };
        let cases = [
            (gbuffer, with(native_render_defs(), &[]), "graph: desktop G-buffer"),
            (lit, with(native_render_defs(), &[]), "graph: desktop lit"),
            (lit, with(native_render_defs(), &["TRANSMISSION"]), "graph: desktop glass"),
            (lit, with(web_render_defs(), &["DETAILS"]), "graph: forward-only lit"),
            (lit, with(web_render_defs(), &["DETAILS", "LOBES", "TRANSMISSION"]), "graph: forward-only glass"),
        ];
        cases
            .into_iter()
            .map(|(source, defs, name)| try_compose_wgsl(source, name, defs, Some(wgsl)).unwrap_or_else(|e| panic!("{name}: {e}\n{wgsl}")))
            .collect()
    }

    /// **The empty graph is the engine's material.** Nothing linked into the output: the function
    /// returns `standard_surface` untouched, and it composes everywhere a surface is drawn.
    #[test]
    fn the_empty_graph_is_the_engines_material() {
        let wgsl = ShaderGraph::new().to_wgsl().expect("the empty graph compiles");
        assert!(wgsl.contains("var surface = standard_surface(input);"));
        assert!(!wgsl.contains("surface.albedo ="), "an empty graph overrides something:\n{wgsl}");
        compose_everywhere(&wgsl);
    }

    /// **Every node kind, and every output of each, composes on both paths.** One graph that adds
    /// each output of each kind in the catalogue, converted to a colour, into the albedo: so every
    /// expression a node writes is validated by naga with the types the conversions give it, and in
    /// every shader a surface is composed into. A kind left out of the catalogue would not be tested,
    /// so the catalogue is checked to name each of them once as well.
    #[test]
    fn every_node_composes_on_both_paths() {
        let mut graph = ShaderGraph::new();
        let output = graph.output().unwrap();
        let mut sum: Option<(NodeId, usize)> = None;
        let catalogue = NodeKind::catalogue();
        let names: std::collections::HashSet<&str> = catalogue.iter().map(NodeKind::name).collect();
        assert_eq!(names.len(), catalogue.len(), "the catalogue names a kind twice");
        for kind in catalogue {
            let outputs = kind.outputs().len();
            let node = graph.add(kind, [0.0, 0.0]);
            for o in 0..outputs {
                let add = graph.add(NodeKind::Add, [0.0, 0.0]);
                graph.connect(node, o, add, 1).unwrap();
                if let Some((n, s)) = sum {
                    graph.connect(n, s, add, 0).unwrap();
                }
                sum = Some((add, 0));
            }
        }
        // And the other outputs of the surface, each from something.
        let (last, slot) = sum.unwrap();
        graph.connect(last, slot, output, 0).unwrap();
        let noise = graph.nodes().iter().find(|n| n.kind == NodeKind::Noise).unwrap().id;
        for input in 1..6 {
            graph.connect(noise, 0, output, input).unwrap();
        }
        let wgsl = graph.to_wgsl().expect("compiles");
        assert!(wgsl.contains("fn shader_graph_noise"));
        compose_everywhere(&wgsl);
        // The sum carries the widest thing added: a vec4 from `Combine`.
        assert_eq!(graph.output_types().unwrap()[&(last, 0)], ValueType::Vec4);
    }

    /// A link joins any two types: a float spreads, a longer vector is cut short, a shorter one is
    /// padded with zeros.
    #[test]
    fn links_convert_between_widths() {
        assert_eq!(convert("x", ValueType::Float, ValueType::Vec3), "vec3<f32>(x)");
        assert_eq!(convert("x", ValueType::Vec4, ValueType::Vec3), "(x).xyz");
        assert_eq!(convert("x", ValueType::Vec3, ValueType::Float), "(x).x");
        assert_eq!(convert("x", ValueType::Vec2, ValueType::Vec4), "vec4<f32>(x, 0.0, 0.0)");
        assert_eq!(convert("x", ValueType::Vec2, ValueType::Vec2), "x");

        // A math node takes the widest of what reaches it: a colour times a float is a colour, and an
        // unlinked operand is a float.
        let mut graph = ShaderGraph::new();
        let color = graph.add(NodeKind::Color([0.9, 0.05, 0.05, 1.0]), [0.0, 0.0]);
        let multiply = graph.add(NodeKind::Multiply, [0.0, 0.0]);
        graph.node_mut(multiply).unwrap().values[1] = [0.5; 4];
        graph.connect(color, 0, multiply, 0).unwrap();
        graph.connect(multiply, 0, NodeId(0), 0).unwrap();
        let wgsl = graph.to_wgsl().unwrap();
        assert!(wgsl.contains("let n1_0: vec3<f32> = vec3<f32>(0.9f, 0.05f, 0.05f);") && wgsl.contains("n1_0 * vec3<f32>(0.5f)"), "{wgsl}");
        assert!(wgsl.contains("surface.albedo = n2_0;") && wgsl.contains("surface.base_color = n2_0;"), "{wgsl}");
        assert_eq!(graph.output_types().unwrap()[&(multiply, 0)], ValueType::Vec3);
    }

    /// **A loop is refused when it is made, and caught when it comes from a file.** `connect` leaves
    /// the graph as it was; a graph read with one in it is an error from `to_wgsl`, not a stack
    /// overflow.
    #[test]
    fn a_loop_is_refused() {
        let mut graph = ShaderGraph::new();
        let a = graph.add(NodeKind::Add, [0.0, 0.0]);
        let b = graph.add(NodeKind::Add, [0.0, 0.0]);
        graph.connect(a, 0, b, 0).unwrap();
        graph.connect(b, 0, NodeId(0), 0).unwrap();
        let before = graph.clone();
        assert_eq!(graph.connect(b, 0, a, 0), Err(GraphError::Cycle(a)));
        assert_eq!(graph.connect(a, 0, a, 1), Err(GraphError::Cycle(a)));
        assert_eq!(graph, before);

        graph.links.push(Link { from: b, output: 0, to: a, input: 1 });
        assert!(matches!(graph.to_wgsl(), Err(GraphError::Cycle(_))));
    }

    /// What a hand-edited or stale file can hold is an error, each named.
    #[test]
    fn a_broken_graph_says_what_is_wrong() {
        let mut graph = ShaderGraph::new();
        let f = graph.add(NodeKind::Float(1.0), [0.0, 0.0]);
        graph.links.push(Link { from: NodeId(99), output: 0, to: NodeId(0), input: 1 });
        assert_eq!(graph.to_wgsl(), Err(GraphError::NoSuchNode(NodeId(99))));
        graph.links.clear();
        graph.links.push(Link { from: f, output: 3, to: NodeId(0), input: 1 });
        assert_eq!(graph.to_wgsl(), Err(GraphError::NoSuchSlot { node: f, slot: 3, input: false }));
        graph.links.clear();
        graph.links.push(Link { from: f, output: 0, to: NodeId(0), input: 1 });
        graph.links.push(Link { from: f, output: 0, to: NodeId(0), input: 1 });
        assert_eq!(graph.to_wgsl(), Err(GraphError::InputLinkedTwice { node: NodeId(0), input: 1 }));
        graph.links.pop();
        graph.node_mut(f).unwrap().kind = NodeKind::Float(f32::NAN);
        assert_eq!(graph.to_wgsl(), Err(GraphError::NotFinite(f)));
        graph.node_mut(f).unwrap().kind = NodeKind::Float(1.0);
        assert!(graph.to_wgsl().is_ok());

        assert_eq!(graph.remove(NodeId(0)), Err(GraphError::OutputRemoved));
        let second = graph.add(NodeKind::Output, [0.0, 0.0]);
        assert_eq!(graph.to_wgsl(), Err(GraphError::SeveralOutputs));
        // With two, either may go; then the other is the output and stays.
        graph.remove(NodeId(0)).unwrap();
        assert_eq!(graph.output(), Ok(second));
        assert_eq!(graph.remove(second), Err(GraphError::OutputRemoved));
    }

    /// Removing a node takes its links with it; the other nodes keep their ids.
    #[test]
    fn removing_a_node_takes_its_links() {
        let mut graph = ShaderGraph::new();
        let time = graph.add(NodeKind::Time, [0.0, 0.0]);
        let sin = graph.add(NodeKind::Sin, [0.0, 0.0]);
        let color = graph.add(NodeKind::Color([1.0, 0.0, 0.0, 1.0]), [0.0, 0.0]);
        graph.connect(time, 0, sin, 0).unwrap();
        graph.connect(sin, 0, NodeId(0), 2).unwrap();
        graph.connect(color, 0, NodeId(0), 0).unwrap();
        graph.remove(sin).unwrap();
        assert_eq!(graph.links(), &[Link { from: color, output: 0, to: NodeId(0), input: 0 }]);
        assert_eq!(graph.node(color).unwrap().kind, NodeKind::Color([1.0, 0.0, 0.0, 1.0]));
        assert!(!graph.to_wgsl().unwrap().contains("surface.roughness ="));
        // The next node gets a new id, not the removed one's neighbour's.
        assert!(graph.add(NodeKind::Time, [0.0, 0.0]) > color);
    }

    /// A node that changes kind keeps the links its new kind has slots for.
    #[test]
    fn changing_a_kind_keeps_the_links_that_fit() {
        let mut graph = ShaderGraph::new();
        let a = graph.add(NodeKind::Float(0.2), [0.0, 0.0]);
        let b = graph.add(NodeKind::Float(0.7), [0.0, 0.0]);
        let lerp = graph.add(NodeKind::Lerp, [0.0, 0.0]);
        graph.connect(a, 0, lerp, 0).unwrap();
        graph.connect(b, 0, lerp, 2).unwrap();
        graph.connect(lerp, 0, NodeId(0), 2).unwrap();
        graph.set_kind(lerp, NodeKind::Add).unwrap();
        assert!(graph.link_into(lerp, 0).is_some() && graph.link_into(lerp, 2).is_none());
        assert_eq!(graph.node(lerp).unwrap().values.len(), 2);
        assert!(graph.to_wgsl().is_ok());
    }

    /// A graph saved and read back is the same graph and the same shader.
    #[test]
    fn a_graph_survives_a_round_trip_through_a_file() {
        let mut graph = ShaderGraph::new();
        let noise = graph.add(NodeKind::Noise, [120.0, -40.0]);
        graph.node_mut(noise).unwrap().values[1] = [4.0; 4];
        let color = graph.add(NodeKind::Color([0.2, 0.4, 0.8, 1.0]), [10.0, 20.0]);
        let multiply = graph.add(NodeKind::Multiply, [200.0, 0.0]);
        graph.connect(noise, 0, multiply, 0).unwrap();
        graph.connect(color, 0, multiply, 1).unwrap();
        graph.connect(multiply, 0, NodeId(0), 0).unwrap();
        let text = ron::to_string(&graph).unwrap();
        let back: ShaderGraph = ron::from_str(&text).unwrap();
        assert_eq!(back, graph);
        assert_eq!(back.to_wgsl().unwrap(), graph.to_wgsl().unwrap());
    }

    /// A node nothing reads costs nothing: it is not in the shader.
    #[test]
    fn a_node_the_surface_does_not_read_is_left_out() {
        let mut graph = ShaderGraph::new();
        let noise = graph.add(NodeKind::Noise, [0.0, 0.0]);
        let wgsl = graph.to_wgsl().unwrap();
        assert!(!wgsl.contains("shader_graph_noise") && !wgsl.contains(&var(noise, 0)), "{wgsl}");
    }
}
