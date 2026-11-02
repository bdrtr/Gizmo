//! [`TextInput`] — a field the player types into: a name for the high-score table, a seed, a
//! chat line.

use gizmo_core::input::Input;
use gizmo_core::query::{Mut, Query};
use gizmo_core::system::Res;

use crate::components::{Clicked, Interaction};

/// A text field. Put it on a button-like node (a [`ButtonBundle`](crate::ButtonBundle), so it has
/// an [`Interaction`] and a [`Clicked`]) with a `Text` beside it: the `Text`'s own content is the
/// **placeholder**, shown while the field is empty; once something is typed the engine draws the
/// value instead, with a caret while the field has focus.
///
/// Clicking the field focuses it (and only it); clicking anywhere else, Escape, or Enter takes the
/// focus away. While focused it takes [`Input::typed_text`] — what the keyboard layout wrote,
/// `ş` and capitals included — and Backspace deletes the last character. Enter also marks it
/// [`submitted`](Self::is_submitted) for that frame.
///
/// Before this a game had no way to ask for a name: the input layer reported physical keys only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    /// What has been typed.
    pub value: String,
    /// The most characters it takes; `0` is no limit.
    pub max_chars: usize,
    focused: bool,
    submitted: bool,
}

impl TextInput {
    /// An empty field taking up to `max_chars` characters (`0` for no limit).
    #[must_use]
    pub fn new(max_chars: usize) -> Self {
        Self { max_chars, ..Self::default() }
    }

    /// Whether it has the keyboard.
    #[must_use]
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Gives it the keyboard (or takes it away) — a menu opening on its name field.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Whether Enter was pressed in it this frame.
    #[must_use]
    pub fn is_submitted(&self) -> bool {
        self.submitted
    }

    /// What the engine draws for it: the value with a caret while focused, or `None` while it is
    /// empty and unfocused (the `Text`'s placeholder is drawn then).
    #[must_use]
    pub fn display(&self) -> Option<String> {
        match (self.value.is_empty(), self.focused) {
            (true, false) => None,
            (_, true) => Some(format!("{}|", self.value)),
            (false, false) => Some(self.value.clone()),
        }
    }

    /// One frame of editing: `typed` appended (up to the limit), `backspace` deleting the last
    /// character, `enter` submitting and leaving. Does nothing unless focused.
    fn edit(&mut self, typed: &str, backspace: bool, enter: bool, escape: bool) {
        self.submitted = false;
        if !self.focused {
            return;
        }
        if backspace {
            self.value.pop();
        }
        for c in typed.chars() {
            if self.max_chars == 0 || self.value.chars().count() < self.max_chars {
                self.value.push(c);
            }
        }
        if enter {
            self.submitted = true;
            self.focused = false;
        }
        if escape {
            self.focused = false;
        }
    }
}

gizmo_core::impl_component!(TextInput);

fn key(name: &str) -> u32 {
    gizmo_core::input::keys::code_from_name(name).unwrap_or(u32::MAX)
}

/// Focuses the clicked field, unfocuses the rest on a click elsewhere, and feeds the focused one
/// this frame's typing. Runs after [`ui_interaction_system`](crate::interaction::ui_interaction_system),
/// whose `Clicked` it reads.
pub fn text_input_system(input: Res<Input>, mut fields: Query<(Mut<TextInput>, Option<&Clicked>, Option<&Interaction>)>) {
    let pressed_anywhere = input.is_mouse_button_just_pressed(0);
    let typed = input.typed_text();
    let backspace = input.is_key_just_pressed(key("backspace"));
    let enter = input.is_key_just_pressed(key("enter"));
    let escape = input.is_key_just_pressed(key("escape"));
    let clicked: Option<u32> = fields.iter_mut().find(|(_, (_, c, _))| c.is_some_and(|c| c.0)).map(|(id, _)| id);
    for (id, (mut field, _, interaction)) in fields.iter_mut() {
        if let Some(c) = clicked {
            field.focused = c == id;
        } else if pressed_anywhere && interaction.is_none_or(|i| *i == Interaction::None) {
            // A press that is not on this field takes its focus away.
            field.focused = false;
        }
        field.edit(typed, backspace, enter, escape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_focused_field_takes_typing_backspace_and_enter() {
        let mut f = TextInput::new(5);
        f.edit("ab", false, false, false);
        assert_eq!(f.value, "", "an unfocused field took typing");
        f.set_focused(true);
        f.edit("Şeker", false, false, false);
        assert_eq!(f.value, "Şeker");
        f.edit("!", false, false, false);
        assert_eq!(f.value, "Şeker", "past the limit");
        f.edit("", true, false, false);
        assert_eq!(f.value, "Şeke");
        assert_eq!(f.display().as_deref(), Some("Şeke|"));
        f.edit("", false, true, false);
        assert!(f.is_submitted() && !f.is_focused());
        assert_eq!(f.display().as_deref(), Some("Şeke"));
        f.edit("", false, false, false);
        assert!(!f.is_submitted(), "submitted lasts one frame");
        assert_eq!(TextInput::new(0).display(), None, "empty and unfocused shows the placeholder");
    }

    /// Through the system: a click focuses the field, typing lands in it, a click elsewhere
    /// takes the focus away.
    #[test]
    fn clicking_focuses_and_clicking_elsewhere_unfocuses() {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let field = world.spawn_bundle((TextInput::new(0), Clicked(true), Interaction::Hovered)).id();
        let run = |world: &mut gizmo_core::World| {
            let mut s = Schedule::new();
            s.add_di_system(text_input_system.into_config());
            s.build();
            s.run(world, 0.0);
        };
        run(&mut world);
        assert!(world.borrow::<TextInput>().get(field).unwrap().is_focused());

        world.borrow_mut::<Clicked>().get_mut(field).unwrap().0 = false;
        world.get_resource_mut::<Input>().unwrap().on_text("hi");
        run(&mut world);
        assert_eq!(world.borrow::<TextInput>().get(field).unwrap().value, "hi");

        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            input.begin_frame();
            input.on_mouse_button_pressed(0);
        }
        *world.borrow_mut::<Interaction>().get_mut(field).unwrap() = Interaction::None;
        run(&mut world);
        assert!(!world.borrow::<TextInput>().get(field).unwrap().is_focused(), "a click elsewhere kept the focus");
    }
}
