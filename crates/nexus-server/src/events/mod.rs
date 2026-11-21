//! Cube-scoped event backbone (skeleton) and the router to connection fan-out.

pub mod bus;
pub mod router;

pub use bus::EventBus;
pub use router::EventRouter;
