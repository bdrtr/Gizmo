use gizmo_core::system::Schedule;
use gizmo_core::world::World;
use crate::plugin::Plugin;

pub struct App<State: 'static = ()> {
    pub world: World,
    pub schedule: Schedule,
    setup_fn: Option<Box<dyn FnOnce(&mut World) -> State + 'static>>,
    update_fn: Option<Box<dyn FnMut(&mut World, &mut State, f32)>>, // dt
    runner: Option<Box<dyn FnOnce(App<State>)>>,
}

impl<State: 'static> App<State> {
    pub fn new(_title: &str, _width: u32, _height: u32) -> Self {
        Self {
            world: World::new(),
            schedule: Schedule::new(),
            setup_fn: None,
            update_fn: None,
            runner: None,
        }
    }

    pub fn set_runner<F>(mut self, f: F) -> Self
    where
        F: FnOnce(App<State>) + 'static,
    {
        self.runner = Some(Box::new(f));
        self
    }

    pub fn add_plugin<P: Plugin<State>>(mut self, plugin: P) -> Self {
        plugin.build(&mut self);
        self
    }

    pub fn set_setup<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut World) -> State + 'static,
    {
        self.setup_fn = Some(Box::new(f));
        self
    }

    pub fn set_update<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, f32) + 'static,
    {
        self.update_fn = Some(Box::new(f));
        self
    }

    pub fn add_system<Params, S: gizmo_core::system::IntoSystemConfig<Params>>(
        mut self,
        system: S,
    ) -> Self {
        self.schedule.add_di_system(system);
        self
    }

    pub fn configure_set(mut self, config: gizmo_core::system::SetConfig) -> Self {
        self.schedule.configure_set(config);
        self
    }

    pub fn run(mut self) {
        if let Some(runner) = self.runner.take() {
            runner(self);
            return;
        }
        self.run_default();
    }

    fn run_default(mut self) {
        let mut state = if let Some(setup) = self.setup_fn.take() {
            setup(&mut self.world)
        } else {
            panic!("setup() fonksiyonu atanmadi!");
        };

        let mut last_time = std::time::Instant::now();

        loop {
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_time).as_secs_f32();
            last_time = now;

            if let Some(update) = self.update_fn.as_mut() {
                update(&mut self.world, &mut state, dt);
            }

            self.schedule.run(&mut self.world, dt);

            // Simple busy wait or sleep to avoid 100% CPU in headless if not limited
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
