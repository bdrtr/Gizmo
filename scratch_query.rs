pub trait WorldQuery {
    type Item<'w>;
}
pub struct Mut<'a, T>(&'a mut T);
impl<T: 'static> WorldQuery for Mut<'_, T> {
    type Item<'w> = Mut<'w, T>;
}
