/// Gizmo ECS Event System
/// 
/// `World` içerisinde Resource olarak tutulan evrensel olay (event) kuyruklarıdır.
/// Örnek Kullanım:
/// ```rust,ignore
/// world.insert_resource(Events::<CollisionEvent>::new());
/// 
/// // Olay Fırlatma:
/// world.get_resource_mut::<Events<CollisionEvent>>().unwrap().push(CollisionEvent(..));
/// 
/// // Olay Okuma:
/// for event in world.get_resource_mut::<Events<CollisionEvent>>().unwrap().drain() {
///    println!("Çarpışma oldu!");
/// }
/// ```
pub struct Events<T: 'static> {
    pub list: Vec<T>,
}

impl<T> Events<T> {
    pub fn new() -> Self {
        Self { list: Vec::new() }
    }

    pub fn push(&mut self, event: T) {
        self.list.push(event);
    }

    /// Olayları tüketmek (işlemek) için kuyruğu boşaltır.
    pub fn drain(&mut self) -> std::vec::Drain<'_, T> {
        self.list.drain(..)
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
    
    pub fn clear(&mut self) {
        self.list.clear();
    }
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}
