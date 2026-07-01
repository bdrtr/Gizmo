//! Sequential fallback for `rayon`'s parallel iterators on targets without OS
//! threads (`wasm32-unknown-unknown`).
//!
//! On native builds `rayon` is a real dependency and the call sites `use
//! rayon::prelude::*`. On wasm `rayon` is not compiled at all; the same call
//! sites `use crate::parallel_compat::*` instead, which provides
//! `par_iter`/`par_iter_mut`/`into_par_iter` delegating to the standard-library
//! iterators and makes `with_min_len`/`with_max_len` no-ops. Every downstream
//! combinator (`zip`/`map`/`filter_map`/`for_each`/`try_for_each`/`collect`) is a
//! plain `Iterator` method, so the results are identical — just single-threaded.
//!
//! Order is preserved (rayon's indexed iterators are order-preserving too), so
//! this is behaviour- and determinism-neutral relative to the native path.
#![allow(dead_code)]

/// `[T]::par_iter` / `par_iter_mut` → `iter` / `iter_mut`.
pub trait ParSliceIter<T> {
    fn par_iter(&self) -> core::slice::Iter<'_, T>;
    fn par_iter_mut(&mut self) -> core::slice::IterMut<'_, T>;
}

impl<T> ParSliceIter<T> for [T] {
    #[inline]
    fn par_iter(&self) -> core::slice::Iter<'_, T> {
        self.iter()
    }
    #[inline]
    fn par_iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.iter_mut()
    }
}

/// `Vec<T>::into_par_iter` / `Range<usize>::into_par_iter` → `into_iter`.
pub trait IntoParIterSeq {
    type Iter: Iterator;
    fn into_par_iter(self) -> Self::Iter;
}

impl<T> IntoParIterSeq for Vec<T> {
    type Iter = std::vec::IntoIter<T>;
    #[inline]
    fn into_par_iter(self) -> Self::Iter {
        self.into_iter()
    }
}

impl IntoParIterSeq for std::ops::Range<usize> {
    type Iter = std::ops::Range<usize>;
    #[inline]
    fn into_par_iter(self) -> Self::Iter {
        self
    }
}

/// rayon's `IndexedParallelIterator::with_min_len`/`with_max_len` tuning knobs —
/// no-ops for a single-threaded iterator.
pub trait SeqParTuning: Iterator + Sized {
    #[inline]
    fn with_min_len(self, _len: usize) -> Self {
        self
    }
    #[inline]
    fn with_max_len(self, _len: usize) -> Self {
        self
    }
}

impl<I: Iterator> SeqParTuning for I {}
