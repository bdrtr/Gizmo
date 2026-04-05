#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    id: u32,
    generation: u32,
}

impl Entity {
    #[inline]
    pub fn new(id: u32, generation: u32) -> Self {
        Self { id, generation }
    }

    #[inline]
    pub fn id(&self) -> u32 {
        self.id
    }

    #[inline]
    pub fn generation(&self) -> u32 {
        self.generation
    }
}
