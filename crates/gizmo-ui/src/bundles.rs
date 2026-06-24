use crate::components::{Style, Node, BackgroundColor, Interaction};

/// Bundle for a plain, non-interactive UI element.
#[derive(Default)]
pub struct NodeBundle {
    /// Layout style of the element.
    pub style: Style,
    /// Computed layout geometry of the element.
    pub node: Node,
    /// Fill color of the element.
    pub background_color: BackgroundColor,
}

impl gizmo_core::component::Bundle for NodeBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> {
        vec![
            gizmo_core::archetype::ComponentInfo::of::<Style>(),
            gizmo_core::archetype::ComponentInfo::of::<Node>(),
            gizmo_core::archetype::ComponentInfo::of::<BackgroundColor>(),
        ]
    }

    fn apply(self, world: &mut gizmo_core::world::World, entity: gizmo_core::entity::Entity) {
        world.add_component(entity, self.style);
        world.add_component(entity, self.node);
        world.add_component(entity, self.background_color);
    }

    unsafe fn write_to_archetype(
        self,
        arch: &mut gizmo_core::archetype::Archetype,
        row: usize,
        tick: u32,
    ) {
        self.style.write_to_archetype(arch, row, tick);
        self.node.write_to_archetype(arch, row, tick);
        self.background_color.write_to_archetype(arch, row, tick);
    }
}

/// Bundle for an interactive button UI element, adding an [`Interaction`] state.
#[derive(Default)]
pub struct ButtonBundle {
    /// Layout style of the element.
    pub style: Style,
    /// Computed layout geometry of the element.
    pub node: Node,
    /// Fill color of the element.
    pub background_color: BackgroundColor,
    /// Current pointer interaction state of the element.
    pub interaction: Interaction,
}

impl gizmo_core::component::Bundle for ButtonBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> {
        vec![
            gizmo_core::archetype::ComponentInfo::of::<Style>(),
            gizmo_core::archetype::ComponentInfo::of::<Node>(),
            gizmo_core::archetype::ComponentInfo::of::<BackgroundColor>(),
            gizmo_core::archetype::ComponentInfo::of::<Interaction>(),
        ]
    }

    fn apply(self, world: &mut gizmo_core::world::World, entity: gizmo_core::entity::Entity) {
        world.add_component(entity, self.style);
        world.add_component(entity, self.node);
        world.add_component(entity, self.background_color);
        world.add_component(entity, self.interaction);
    }

    unsafe fn write_to_archetype(
        self,
        arch: &mut gizmo_core::archetype::Archetype,
        row: usize,
        tick: u32,
    ) {
        self.style.write_to_archetype(arch, row, tick);
        self.node.write_to_archetype(arch, row, tick);
        self.background_color.write_to_archetype(arch, row, tick);
        self.interaction.write_to_archetype(arch, row, tick);
    }
}
