use crate::system::{Res, ResMut, SystemParam, SystemParamFetchError, AccessInfo};
use crate::world::World;

/// Gizmo ECS Event System — Double-buffered olay kuyruğu.
///
/// Her frame'de `update()` çağrıldığında, önceki frame'in eventleri atılır ve
/// mevcut frame'in eventleri "önceki" konumuna taşınır. Bu sayede:
/// - Yazarlar (`send`) her zaman `current` buffer'a yazar.
/// - Okuyucular (`iter`) her zaman `previous` buffer'dan okur (non-destructive).
/// - Birden fazla sistem aynı eventleri bağımsız olarak okuyabilir.
///
/// # Kullanım
/// ```rust,ignore
/// // Kayıt (App seviyesinde):
/// app.add_event::<CollisionEvent>();
///
/// // Olay gönderme (herhangi bir sistem):
/// world.get_resource_mut::<Events<CollisionEvent>>().unwrap().send(CollisionEvent(..));
///
/// // Olay okuma (herhangi bir sistem, non-destructive):
/// let events = world.get_resource::<Events<CollisionEvent>>().unwrap();
/// for event in events.iter() {
///     println!("Çarpışma oldu: {:?}", event);
/// }
/// ```
pub struct Events<T> {
    /// Bu frame'e yazılan eventler.
    current: Vec<T>,
    /// Önceki frame'den kalan, okunabilir eventler.
    previous: Vec<T>,
}

impl<T> Events<T> {
    pub fn new() -> Self {
        Self {
            current: Vec::new(),
            previous: Vec::new(),
        }
    }

    /// Yeni bir event gönderir (mevcut frame'in buffer'ına yazar).
    #[inline]
    pub fn send(&mut self, event: T) {
        self.current.push(event);
    }

    /// Geriye dönük uyumluluk — `send()` ile aynı.
    #[inline]
    pub fn push(&mut self, event: T) {
        self.send(event);
    }

    /// Frame sonu: önceki frame'in eventlerini temizler, mevcut frame'i önceki konuma taşır.
    ///
    /// Bu metot her frame sonunda **bir kez** çağrılmalıdır — `App::add_event()` bunu
    /// otomatik olarak yapar.
    pub fn update(&mut self) {
        self.previous.clear();
        std::mem::swap(&mut self.current, &mut self.previous);
    }

    /// Önceki frame'in eventlerini okumak için non-destructive iterator.
    /// Birden fazla sistem aynı eventleri bağımsız olarak okuyabilir.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.previous.iter()
    }

    /// Önceki frame'deki event sayısı.
    #[inline]
    pub fn len(&self) -> usize {
        self.previous.len()
    }

    /// Önceki frame'de event var mı?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.previous.is_empty()
    }

    /// Tüm eventleri (hem mevcut hem önceki) temizler.
    pub fn clear(&mut self) {
        self.current.clear();
        self.previous.clear();
    }

    /// Eventleri tüketmek için destructive iterator.
    /// **Dikkat:** Bu metot tüm eventleri (önceki frame) tüketir. Birden fazla okuyucu
    /// varsa diğer okuyucular eventleri kaçırır. Mümkünse `iter()` tercih edin.
    pub fn drain(&mut self) -> std::vec::IntoIter<T> {
        self.previous.drain(..).collect::<Vec<_>>().into_iter()
    }
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ==============================================================
// EventReader
// ==============================================================

pub struct EventReader<'w, T: 'static> {
    events: Res<'w, Events<T>>,
}

impl<'w, T: 'static> EventReader<'w, T> {
    /// Olayları okumak için iterator döndürür (önceki frame'in eventleri).
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.events.iter()
    }
    
    pub fn len(&self) -> usize {
        self.events.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl<T: 'static> SystemParam for EventReader<'static, T> {
    type Item<'w> = EventReader<'w, T>;
    fn fetch<'w>(world: &'w World, _dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let events = Res::<Events<T>>::fetch(world, _dt)?;
        Ok(EventReader { events })
    }
    fn get_access_info(info: &mut AccessInfo) {
        Res::<'static, Events<T>>::get_access_info(info);
    }
}

// ==============================================================
// EventWriter
// ==============================================================

pub struct EventWriter<'w, T: 'static> {
    events: ResMut<'w, Events<T>>,
}

impl<'w, T: 'static> EventWriter<'w, T> {
    /// Yeni bir olay fırlatır (mevcut frame'in buffer'ına yazar).
    pub fn send(&mut self, event: T) {
        self.events.send(event);
    }
    
    /// Birden fazla olay fırlatır.
    pub fn send_batch(&mut self, events: impl IntoIterator<Item = T>) {
        for event in events {
            self.events.send(event);
        }
    }
}

impl<T: 'static> SystemParam for EventWriter<'static, T> {
    type Item<'w> = EventWriter<'w, T>;
    fn fetch<'w>(world: &'w World, _dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let events = ResMut::<Events<T>>::fetch(world, _dt)?;
        Ok(EventWriter { events })
    }
    fn get_access_info(info: &mut AccessInfo) {
        ResMut::<'static, Events<T>>::get_access_info(info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_and_iter() {
        let mut events = Events::new();
        events.send(1);
        events.send(2);
        events.send(3);

        // Henüz update() çağrılmadı — iter() önceki frame (boş)
        assert!(events.iter().next().is_none());
        assert!(events.is_empty());

        // Frame ilerlet
        events.update();

        // Artık eventler okunabilir
        let collected: Vec<&i32> = events.iter().collect();
        assert_eq!(collected, vec![&1, &2, &3]);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_non_destructive_iter() {
        let mut events = Events::new();
        events.send(42);
        events.update();

        // İlk okuma
        assert_eq!(events.iter().next(), Some(&42));
        // İkinci okuma — hâlâ erişilebilir
        assert_eq!(events.iter().next(), Some(&42));
    }

    #[test]
    fn test_double_buffer_isolation() {
        let mut events = Events::new();

        // Frame 1: event gönder
        events.send(1);
        events.update();

        // Frame 2: yeni event gönder + eski eventleri oku
        events.send(2);
        let frame1_events: Vec<&i32> = events.iter().collect();
        assert_eq!(frame1_events, vec![&1]); // Sadece önceki frame

        events.update();

        // Frame 3: frame 2'nin eventleri okunabilir, frame 1'inkiler gitmiş
        let frame2_events: Vec<&i32> = events.iter().collect();
        assert_eq!(frame2_events, vec![&2]);
    }

    #[test]
    fn test_update_clears_previous() {
        let mut events = Events::new();
        events.send(1);
        events.update();
        assert_eq!(events.len(), 1);

        // Yeni frame — eski event temizlenmeli
        events.update();
        assert!(events.is_empty());
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_push_backward_compat() {
        let mut events = Events::new();
        events.push(99); // Eski API
        events.update();
        assert_eq!(events.iter().next(), Some(&99));
    }

    #[test]
    fn test_clear() {
        let mut events = Events::new();
        events.send(1);
        events.update();
        events.send(2);

        events.clear();
        assert!(events.is_empty());

        events.update();
        assert!(events.is_empty());
    }

    #[test]
    fn test_drain_consumes() {
        let mut events = Events::new();
        events.send(10);
        events.send(20);
        events.update();

        let drained: Vec<i32> = events.drain().collect();
        assert_eq!(drained, vec![10, 20]);

        // drain sonrası boş
        assert!(events.is_empty());
    }

    #[test]
    fn test_no_static_bound() {
        // 'static bound kaldırıldığını doğrula — kısa ömürlü tipler de çalışır
        struct Ephemeral<'a>(&'a str);
        let mut events = Events::new();
        let msg = String::from("test");
        events.send(Ephemeral(&msg));
        events.update();
        assert_eq!(events.iter().next().unwrap().0, "test");
    }
}
