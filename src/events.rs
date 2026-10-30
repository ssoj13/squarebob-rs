//! Application event bus and domain events backed by `appbus::bus`.
//!
//! Wraps [`appbus::bus::EventBus`] and [`appbus::bus::EventEmitter`] while preserving
//! the queued emit/poll ergonomics used across `squarebob`'s UI panels and render loop.

use std::collections::VecDeque;

pub use appbus::bus::{BoxedEvent, Event, EventEmitter, downcast_event};

/// Event bus backed by [`appbus::bus::EventBus`].
#[derive(Clone, Debug, Default)]
pub struct EventBus {
    inner: appbus::bus::EventBus,
}

impl EventBus {
    /// Creates a new [`EventBus`].
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: appbus::bus::EventBus::new(),
        }
    }

    /// Returns a reference to the underlying [`appbus::bus::EventBus`].
    #[inline]
    pub fn app_bus(&self) -> &appbus::bus::EventBus {
        &self.inner
    }

    /// Creates a lightweight [`EventEmitter`] handle sharing this bus.
    #[inline]
    pub fn emitter(&self) -> EventEmitter {
        self.inner.emitter()
    }

    /// Emits an event (queued for later processing and dispatched to any registered subscribers).
    #[inline]
    pub fn emit<E: Event + Clone>(&self, event: E) {
        self.inner.emit(event);
    }

    /// Drains all queued events into `out` (clearing `out` first) and returns the count.
    #[inline]
    pub fn poll_into(&self, out: &mut Vec<BoxedEvent>) -> usize {
        self.inner.poll_into(out)
    }

    /// Polls all queued events into a [`VecDeque`].
    #[inline]
    pub fn poll(&self) -> VecDeque<BoxedEvent> {
        VecDeque::from(self.inner.poll())
    }

    /// Returns `true` if there is at least one queued event waiting to be polled.
    #[inline]
    pub fn has_pending(&self) -> bool {
        self.inner.queue_len() > 0
    }
}

/// Downcasts a [`BoxedEvent`] to concrete event type `E`.
#[inline]
pub fn downcast<E: Event>(event: &BoxedEvent) -> Option<&E> {
    downcast_event::<E>(event)
}

// ============================================================================
// Events
// ============================================================================

/// Material library mutated (params, variance, weight, add / remove /
/// rename / reorder / load). Triggers a PT accumulation reset + a
/// forced 3D re-render — without rebuilding the treemap layout.
///
/// The previous blanket `SettingsChangedEvent` is gone: settings-panel
/// callbacks now emit a typed `SettingsDirty` record (see
/// `app::settings::dirty`) which is dispatched directly inside the
/// settings panel, so layout / preset-dirty / repaint side-effects
/// stay scoped to the field that actually changed.
#[derive(Clone, Debug)]
pub struct MaterialsChangedEvent;

/// Activate render/display Settings after the dock has returned to its host.
#[derive(Clone, Debug)]
pub struct OpenSettingsEvent;

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
