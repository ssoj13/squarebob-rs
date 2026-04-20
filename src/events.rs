//! Simple event bus for decoupled component communication.
//!
//! Minimal design: emit events → poll in main loop → handle.
//! No pub/sub callbacks - keeps it simple for this app's needs.



use std::any::Any;
use std::collections::VecDeque;

/// Marker trait for events
pub trait Event: Any + Send + 'static {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Send + 'static> Event for T {
    fn as_any(&self) -> &dyn Any { self }
}

/// Boxed event for queue storage
pub type BoxedEvent = Box<dyn Event>;

/// Simple event queue
#[derive(Default)]
pub struct EventBus {
    queue: VecDeque<BoxedEvent>,
}

impl EventBus {
    pub fn new() -> Self { Self::default() }

    /// Emit an event (queued for later processing)
    pub fn emit<E: Event>(&mut self, event: E) {
        self.queue.push_back(Box::new(event));
    }

    /// Poll all queued events (zero-alloc swap)
    pub fn poll(&mut self) -> VecDeque<BoxedEvent> {
        std::mem::take(&mut self.queue)
    }

    /// Check if any events are pending
    #[allow(dead_code)]
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }
}

/// Downcast boxed event to concrete type
#[inline]
pub fn downcast<E: Event>(event: &BoxedEvent) -> Option<&E> {
    (**event).as_any().downcast_ref::<E>()
}

// ============================================================================
// Events
// ============================================================================

/// Settings changed (triggers re-render)
#[derive(Clone, Debug)]
pub struct SettingsChangedEvent;

/// Navigation: go into directory
#[derive(Clone, Debug)]
pub struct NavigateIntoEvent(pub std::path::PathBuf);

/// Navigation: go up one level (zoom out)
#[derive(Clone, Debug)]
pub struct NavigateUpEvent;

/// Reset zoom to root
#[derive(Clone, Debug)]
pub struct ZoomResetEvent;

/// Select a file/folder
#[derive(Clone, Debug)]
pub struct SelectPathEvent(pub std::path::PathBuf);

/// Rebuild layout needed
#[derive(Clone, Debug)]
pub struct LayoutDirtyEvent;

/// 3D render tick (decouple viewport render from UI)
#[derive(Clone, Debug)]
pub struct RenderTick3DEvent;
