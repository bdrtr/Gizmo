pub trait WorldQuery<'w> {}
impl<'w> WorldQuery<'w> for &'w i32 {}

pub struct Query<'w, Q: WorldQuery<'w>>(std::marker::PhantomData<Q>, &'w ());

pub type QRef<T> = &'static T;

pub trait SystemParam {
    type Item<'w>;
}
impl<Q: WorldQuery<'static> + 'static> SystemParam for Query<'static, Q> {
    type Item<'w> = Query<'w, &'w i32>; // hack for test
}

pub trait IntoSystem<Marker> {
    fn into_system(self);
}

impl<F, P1> IntoSystem<fn(P1)> for F
where
    P1: SystemParam + 'static,
    for<'w> F: FnMut(P1::Item<'w>) + FnMut(P1) + 'static
{
    fn into_system(self) {}
}

fn test_sys(q: Query<QRef<i32>>) {}

fn main() {
    test_sys.into_system();
}
