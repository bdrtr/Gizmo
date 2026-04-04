use crate::world::World;

pub trait System {
    fn run(&mut self, world: &World);
}

// Bir &World referansı alan her fonksiyonu otomatikman System yapıyoruz.
impl<F> System for F
where
    F: FnMut(&World),
{
    fn run(&mut self, world: &World) {
        (self)(world);
    }
}

pub struct Schedule {
    systems: Vec<Box<dyn System>>,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.systems.push(Box::new(system));
    }

    pub fn run(&mut self, world: &World) {
        for system in &mut self.systems {
            system.run(world);
        }
    }
}
