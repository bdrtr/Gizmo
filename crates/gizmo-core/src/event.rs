//! Frame-delayed, double-buffered event queues: [`Events<T>`] plus the [`EventReader`] and
//! [`EventWriter`] system parameters.
//!
//! One queue per event type, held as a world resource. Sends land in a write buffer; reads
//! come from the buffer that was the write buffer *last* frame. An event is therefore never
//! visible during the frame that produced it, and it is discarded by the second
//! [`Events::update`] after it was sent — a one-frame readable window, no more.
//!
//! Reading is non-destructive and there is no per-reader cursor: every reader of a type sees
//! the same whole batch, in send order, however many times it looks. The flip side is that a
//! system which does not run on a given frame misses that frame's events permanently instead
//! of catching up later.
//!
//! Nothing in this module rotates the buffers by itself. A queue whose owner never calls
//! [`Events::update`] never becomes readable at all, while its write buffer grows without
//! bound.

use crate::system::{AccessInfo, Res, ResMut, SystemParam, SystemParamFetchError};
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
///     tracing::info!("Çarpışma oldu: {:?}", event);
/// }
/// ```
pub struct Events<T> {
    /// Bu frame'e yazılan eventler.
    current: Vec<T>,
    /// Önceki frame'den kalan, okunabilir eventler.
    previous: Vec<T>,
}

impl<T> Events<T> {
    /// Creates a queue with both buffers empty and nothing pre-allocated.
    ///
    /// A brand-new queue reads as empty even right after a burst of [`send`](Self::send):
    /// reads come from the *previous* buffer, so the first events become visible only after
    /// the first [`update`](Self::update). `T` needs no bounds here — only storing the queue
    /// as a world resource requires `T: Send + Sync + 'static`.
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

/// System parameter for reading events of type `T` — a shared borrow of the world's
/// [`Events<T>`] resource, held for the duration of the system body.
///
/// Reading is non-destructive and there is no per-reader cursor: every reader of the same
/// `T` sees exactly the previous frame's batch, in send order, however many times it looks.
/// Because it declares a resource *read*, several reader systems may run in the same batch.
/// The flip side of having no cursor is that a system which does not run on a given frame
/// misses that frame's events permanently.
///
/// Only events already rotated into the readable buffer by [`Events::update`] are visible;
/// anything sent during the current frame is not. If nobody ever calls `update`, readers
/// see nothing at all while the send buffer grows without bound.
///
/// Fetching the parameter fails — which panics the system — when no `Events<T>` resource was
/// inserted into the world, or when the same system's parameter list also asks for an
/// [`EventWriter<T>`] of the same `T`: the two borrows exclude each other, and whichever is
/// fetched second is the one that fails.
pub struct EventReader<'w, T: 'static> {
    events: Res<'w, Events<T>>,
}

impl<'w, T: 'static> EventReader<'w, T> {
    /// Olayları okumak için iterator döndürür (önceki frame'in eventleri).
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.events.iter()
    }

    /// Number of events in the readable batch (the previous frame's).
    ///
    /// Does not count anything sent during this frame — those sit in the write buffer until
    /// the next [`Events::update`]. Stable for the whole system body, since reading consumes
    /// nothing.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// `true` when this frame's readable batch is empty.
    ///
    /// Not "no events exist": events written during the current frame land in the *write*
    /// batch and are invisible until the swap, so a reader can see `true` immediately after
    /// another system sent one.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl<T: 'static> crate::system::sealed::Sealed for EventReader<'static, T> {}
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

/// System parameter for sending events of type `T` — an exclusive borrow of the world's
/// [`Events<T>`] resource, held for the duration of the system body.
///
/// Sends land in the write buffer and are invisible to readers until [`Events::update`]
/// rotates the buffers, so an event is never observed in the frame that produced it.
/// Events are appended, so readers see them in the order they were sent.
///
/// Declaring one is a resource *write*, so the scheduler will not put this system in the
/// same batch as any other system taking an `EventReader<T>` or `EventWriter<T>`; the
/// exclusion is per event type — writers of different `T` do not conflict. Asking
/// for `EventWriter<T>` twice — or for a writer and a reader of the same `T` — in one
/// parameter list makes the second fetch fail and panics the system.
///
/// Fetching also fails, and panics, when no `Events<T>` resource was inserted.
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

impl<T: 'static> crate::system::sealed::Sealed for EventWriter<'static, T> {}
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
