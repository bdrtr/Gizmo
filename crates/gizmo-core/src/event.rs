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

/// Gizmo ECS Event System — double-buffered event queue.
///
/// When `update()` is called each frame, the previous frame's events are discarded and the
/// current frame's events are moved into the "previous" position. Thanks to this:
/// - Writers (`send`) always write to the `current` buffer.
/// - Readers (`iter`) always read from the `previous` buffer (non-destructive).
/// - More than one system can read the same events independently.
///
/// # Usage
/// ```
/// use gizmo_core::prelude::*;
/// struct CollisionEvent(u32);
///
/// // Registration: at App level `app.add_event::<CollisionEvent>()` both inserts this
/// // resource and calls `update()` every frame. Setting it up by hand:
/// let mut world = World::new();
/// world.insert_resource(Events::<CollisionEvent>::new());
///
/// // Sending an event (from any system):
/// world.get_resource_mut::<Events<CollisionEvent>>().unwrap().send(CollisionEvent(7));
///
/// // Not readable yet in the frame it was sent:
/// assert_eq!(world.get_resource::<Events<CollisionEvent>>().unwrap().len(), 0);
///
/// // Frame sonu rotasyonu:
/// world.get_resource_mut::<Events<CollisionEvent>>().unwrap().update();
///
/// // Reading events (any system, non-destructive — every reader sees the same batch):
/// let events = world.get_resource::<Events<CollisionEvent>>().unwrap();
/// assert_eq!(events.iter().map(|e| e.0).collect::<Vec<_>>(), vec![7]);
/// assert_eq!(events.iter().count(), 1);
/// ```
pub struct Events<T> {
    /// The events written in this frame.
    current: Vec<T>,
    /// The readable events left over from the previous frame.
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

    /// Sends a new event (writes into the current frame's buffer).
    #[inline]
    pub fn send(&mut self, event: T) {
        self.current.push(event);
    }

    /// Backward compatibility — same as `send()`.
    #[inline]
    pub fn push(&mut self, event: T) {
        self.send(event);
    }

    /// End of frame: clears the previous frame's events, moves the current frame into the
    /// previous position.
    ///
    /// This method must be called **once** at the end of every frame — `App::add_event()` does
    /// this automatically.
    pub fn update(&mut self) {
        self.previous.clear();
        std::mem::swap(&mut self.current, &mut self.previous);
    }

    /// Non-destructive iterator for reading the previous frame's events.
    /// More than one system can read the same events independently.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.previous.iter()
    }

    /// The number of events in the previous frame.
    #[inline]
    pub fn len(&self) -> usize {
        self.previous.len()
    }

    /// Are there any events in the previous frame?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.previous.is_empty()
    }

    /// Clears all events (both current and previous).
    pub fn clear(&mut self) {
        self.current.clear();
        self.previous.clear();
    }

    /// Destructive iterator for consuming the events.
    /// **Caution:** This method consumes all the events (the previous frame). If there is more
    /// than one reader, the other readers miss the events. Prefer `iter()` when possible.
    pub fn drain(&mut self) -> std::vec::IntoIter<T> {
        // `mem::take`, not `drain(..).collect()`: same result, one fewer allocation, and
        // clippy's `drain_collect` (1.98) rejects the latter.
        std::mem::take(&mut self.previous).into_iter()
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
    /// Returns an iterator for reading the events (the previous frame's events).
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
    type State = ();
    fn fetch<'w>(
        world: &'w World,
        _dt: f32,
        _state: &'w mut (),
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let events = Res::<Events<T>>::fetch_stateless(world, _dt)?;
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
    /// Fires a new event (writes into the current frame's buffer).
    pub fn send(&mut self, event: T) {
        self.events.send(event);
    }

    /// Fires more than one event.
    pub fn send_batch(&mut self, events: impl IntoIterator<Item = T>) {
        for event in events {
            self.events.send(event);
        }
    }
}

impl<T: 'static> crate::system::sealed::Sealed for EventWriter<'static, T> {}
impl<T: 'static> SystemParam for EventWriter<'static, T> {
    type Item<'w> = EventWriter<'w, T>;
    type State = ();
    fn fetch<'w>(
        world: &'w World,
        _dt: f32,
        _state: &'w mut (),
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let events = ResMut::<Events<T>>::fetch_stateless(world, _dt)?;
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
