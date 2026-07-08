pub mod clip;
pub mod ik;
pub mod player;
pub mod system;

use gizmo_core::system::IntoSystemConfig;

// Gizmo uygulamasında (App) animation plugin'i için:
// pub struct AnimationPlugin;
// impl Plugin... (bunu gizmo kütüphanesine veya buraya koyabiliriz).

pub struct AnimationPlugin;

impl<State: 'static> gizmo_app::Plugin<State> for AnimationPlugin {
    fn build(&self, app: &mut gizmo_app::App<State>) {
        app.world.register_component_type::<player::AnimationPlayer>();
        app.world.register_component_type::<player::Animated>();
        app.world.register_component_type::<ik::TwoBoneIkChain>();
        app.schedule.add_di_system(
            system::animation_system
                .into_config()
                .label("animation_update")
                .before("transform_propagate"), // Animasyonların yerel transformları, global'e dönüşmeden önce güncellenmeli.
        );
    }
}
