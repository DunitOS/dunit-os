//! FileCapOp suboperations shared by kernel and native wrappers.
#![no_std]
mod generated;
pub use generated::{Operation, Relation};
impl Relation {
    pub fn paths(source: &str, destination: &str) -> Self {
        if source == destination {
            return Self::Same;
        }
        fn ancestor(a: &str, b: &str) -> bool {
            a.is_empty() || b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
        }
        if ancestor(source, destination) {
            Self::SourceAncestor
        } else if ancestor(destination, source) {
            Self::DestinationAncestor
        } else {
            Self::Unrelated
        }
    }
}
