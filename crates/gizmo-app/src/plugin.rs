use crate::App;

/// A reusable bundle of application setup logic.
///
/// Plugins encapsulate world/resource/system registration so it can be added
/// to an [`App`] in one call via `App::add_plugin`.
///
/// This trait is a deliberate **extension point**: downstream crates and
/// applications are expected to implement it for their own plugin types, so it
/// is intentionally *not* sealed. Future methods, if needed, will be added with
/// default implementations to remain backwards-compatible.
pub trait Plugin<State: 'static = ()> {
    /// Applies this plugin's setup to the given [`App`].
    fn build(&self, app: &mut App<State>);
}
