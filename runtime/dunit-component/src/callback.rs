//! Typed callback composition for reusable child components. No global dispatch.
use alloc::rc::Rc;
use core::fmt;
pub(crate) enum Callback<A, M> {
    Direct(fn(A) -> M),
    Mapped(Rc<dyn Fn(A) -> M>),
}
impl<A, M> Callback<A, M> {
    pub fn new(f: fn(A) -> M) -> Self {
        Self::Direct(f)
    }
    pub fn call(&self, a: A) -> M {
        match self {
            Self::Direct(f) => f(a),
            Self::Mapped(f) => f(a),
        }
    }
}
impl<A: 'static, M: 'static> Callback<A, M> {
    pub fn map<N: 'static>(self, mapper: Rc<dyn Fn(M) -> N>) -> Callback<A, N> {
        Callback::Mapped(Rc::new(move |a| mapper(self.call(a))))
    }
}
impl<A, M> Clone for Callback<A, M> {
    fn clone(&self) -> Self {
        match self {
            Self::Direct(f) => Self::Direct(*f),
            Self::Mapped(f) => Self::Mapped(f.clone()),
        }
    }
}
impl<A, M> PartialEq for Callback<A, M> {
    fn eq(&self, b: &Self) -> bool {
        match (self, b) {
            (Self::Direct(a), Self::Direct(b)) => core::ptr::fn_addr_eq(*a, *b),
            (Self::Mapped(a), Self::Mapped(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}
impl<A, M> fmt::Debug for Callback<A, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Callback")
    }
}
