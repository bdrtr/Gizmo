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
    pub events_a: Vec<T>,
    pub events_b: Vec<T>,
    pub a_is_active: bool,
}

impl<T> Events<T> {
    pub fn new() -> Self {
        Self {
            events_a: Vec::new(),
            events_b: Vec::new(),
            a_is_active: true,
        }
    }

    pub fn push(&mut self, event: T) {
        if self.a_is_active {
            self.events_a.push(event);
        } else {
            self.events_b.push(event);
        }
    }

    /// Çift-buffer (Double-buffer) çerçeve ilerletmesi.
    /// En eski buffer'ı temizler ve aktif yazma hedefini diğerine kaydırır.
    pub fn update(&mut self) {
        if self.a_is_active {
            self.events_b.clear();
        } else {
            self.events_a.clear();
        }
        self.a_is_active = !self.a_is_active;
    }

    /// Olayları tüketmek (işlemek) için tüm kuyruğu boşaltır. (Geriye dönük uyumluluk)
    pub fn drain(&mut self) -> std::vec::IntoIter<T> {
        let mut all = Vec::new();
        if self.a_is_active {
            all.append(&mut self.events_b);
            all.append(&mut self.events_a);
        } else {
            all.append(&mut self.events_a);
            all.append(&mut self.events_b);
        }
        all.into_iter()
    }

    pub fn is_empty(&self) -> bool {
        self.events_a.is_empty() && self.events_b.is_empty()
    }

    pub fn clear(&mut self) {
        self.events_a.clear();
        self.events_b.clear();
    }
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}
