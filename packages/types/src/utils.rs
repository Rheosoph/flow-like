use std::sync::Arc;

pub use constant_time_eq::constant_time_eq;

pub mod data_url;
pub mod img;

#[inline]
pub fn ptr_key<T>(arc: &Arc<T>) -> usize {
    Arc::as_ptr(arc) as usize
}
