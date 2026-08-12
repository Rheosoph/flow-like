//! Feature preprocessing nodes.
//!
//! Fitted transformers that learn statistics on a training set and can then be applied to held-out
//! data, which is what separates these from the stateless vector helpers in the std catalog.

pub mod apply;
pub mod scaler;
pub mod vectorizer;
