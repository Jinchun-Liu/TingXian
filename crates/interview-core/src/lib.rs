pub mod index;
pub mod model;
pub use index::*;
pub use model::*;
pub type Result<T> = std::result::Result<T, String>;
