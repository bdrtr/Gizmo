use crate::App;

/// A reusable bundle of application setup logic.
///
/// Plugins encapsulate world/resource/system registration so it can be added
/// to an [`App`] in one call via `App::add_plugin`.
pub trait Plugin<State: 'static = ()> {
    /// Applies this plugin's setup to the given [`App`].
    fn build(&self, app: &mut App<State>);
}
