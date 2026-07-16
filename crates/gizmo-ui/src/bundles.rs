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

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::component::Bundle;
    use gizmo_core::world::World;
    use gizmo_math::Vec4;

    fn register_ui(world: &mut World) {
        world.register_component_type::<Style>();
        world.register_component_type::<Node>();
        world.register_component_type::<BackgroundColor>();
        world.register_component_type::<Interaction>();
    }

    #[test]
    fn node_bundle_attaches_only_the_visual_components() {
        let mut world = World::new();
        register_ui(&mut world);
        let e = world.spawn();
        NodeBundle::default().apply(&mut world, e);

        assert!(world.borrow::<Style>().get(e.id()).is_some());
        assert!(world.borrow::<Node>().get(e.id()).is_some());
        assert!(world.borrow::<BackgroundColor>().get(e.id()).is_some());
        // A plain node is non-interactive: it must NOT carry an Interaction, or it
        // would be picked up by the interaction system's hit-test query.
        assert!(world.borrow::<Interaction>().get(e.id()).is_none());
        // The advertised archetype shape must match what `apply` actually writes.
        assert_eq!(NodeBundle::get_infos().len(), 3);
    }

    #[test]
    fn button_bundle_adds_interaction_with_sane_defaults() {
        let mut world = World::new();
        register_ui(&mut world);
        let e = world.spawn();
        ButtonBundle::default().apply(&mut world, e);

        assert!(world.borrow::<Style>().get(e.id()).is_some());
        assert!(world.borrow::<Node>().get(e.id()).is_some());
        // A fresh button starts un-interacted.
        assert_eq!(*world.borrow::<Interaction>().get(e.id()).unwrap(), Interaction::None);
        // Default fill is OPAQUE white: a transparent (alpha 0) or black default
        // would make every button invisible, so this default is a real contract.
        let bg = *world.borrow::<BackgroundColor>().get(e.id()).unwrap();
        assert_eq!(bg.0, Vec4::new(1.0, 1.0, 1.0, 1.0));
        // Button advertises one more component (Interaction) than a plain node.
        assert_eq!(ButtonBundle::get_infos().len(), 4);
        assert_eq!(ButtonBundle::get_infos().len(), NodeBundle::get_infos().len() + 1);
    }
}
