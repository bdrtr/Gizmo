use crate::App;

pub trait Plugin<State: 'static = ()> {
    fn build(&self, app: &mut App<State>);
}
