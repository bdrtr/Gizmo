use gizmo_core::query::{Query, Mut};
use gizmo_core::system::Res;
use gizmo_core::input::Input;
use crate::components::{Interaction, Node};

pub fn ui_interaction_system(
    input: Res<Input>,
    mut interactions: Query<(&Node, Mut<Interaction>)>,
) {
    let mouse_pos = input.mouse_position();
    let is_clicked = input.is_mouse_button_pressed(0); // 0 is left click

    // Note: This is a simplified check that doesn't account for z-index or hierarchy properly yet.
    // In a real UI system, we would need to walk the tree from front to back.
    for (_, (node, mut interaction)) in interactions.iter_mut() {
        let is_hovered = mouse_pos.0 >= node.position.x
            && mouse_pos.0 <= node.position.x + node.size.x
            && mouse_pos.1 >= node.position.y
            && mouse_pos.1 <= node.position.y + node.size.y;

        if is_hovered {
            if is_clicked {
                *interaction = Interaction::Pressed;
            } else {
                *interaction = Interaction::Hovered;
            }
        } else {
            *interaction = Interaction::None;
        }
    }
}
