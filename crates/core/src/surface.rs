//! Graph-backed host observations and ordered requests. Adapters own publication;
//! views read attributes and request external work from actions or committed effects.

use crate::{App, Observer, Read, Runtime, State};
use std::{cell::Cell, fmt, rc::Rc};
pub use url::Url;

const REQUEST_CAPACITY: usize = 128;

/// Available layout space in logical pixels, independent of rendered content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f64,
    /// Content-sized browser mounts have no constrained viewport height.
    pub height: Option<f64>,
    pub scale_factor: f64,
}

impl Viewport {
    fn validate(self) -> Result<(), SurfaceError> {
        if !self.width.is_finite()
            || self.width < 0.0
            || self.height.is_some_and(|h| !h.is_finite() || h < 0.0)
            || !self.scale_factor.is_finite()
            || self.scale_factor <= 0.0
        {
            return Err(SurfaceError::InvalidViewport);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceCapabilities {
    pub title: bool,
    pub navigation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    Push,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceError {
    Closed,
    Full,
    Unsupported,
    InvalidViewport,
    InvalidUrl(String),
    Host(String),
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("surface is closed"),
            Self::Full => f.write_str("surface request queue is full"),
            Self::Unsupported => f.write_str("surface operation is not supported"),
            Self::InvalidViewport => f.write_str("invalid viewport dimensions or scale"),
            Self::InvalidUrl(message) => write!(f, "invalid application URL: {message}"),
            Self::Host(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SurfaceError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Applied,
    Failed(SurfaceError),
}

/// Retaining a request allows its completion to be observed through the graph.
#[derive(Clone)]
pub struct Request {
    pub status: Read<RequestStatus>,
}

#[derive(Clone, Debug)]
pub enum SurfaceCommand {
    SetTitle(String),
    /// Resolve relative references when executing, after preceding navigation requests.
    Navigate {
        reference: String,
        mode: HistoryMode,
    },
}

/// One admitted operation. The driver completes it exactly once after host I/O.
#[derive(Clone)]
pub struct SurfaceRequest {
    pub command: SurfaceCommand,
    status: State<RequestStatus>,
}

impl SurfaceRequest {
    pub fn complete(&self, rt: &mut Runtime, result: Result<(), SurfaceError>) {
        if matches!(self.status.get(rt), RequestStatus::Pending) {
            self.status.set(
                rt,
                match result {
                    Ok(()) => RequestStatus::Applied,
                    Err(error) => RequestStatus::Failed(error),
                },
            );
        }
    }
}

/// One application's host context. Reading one field does not subscribe to others.
pub struct Surface {
    pub viewport: Read<Option<Viewport>>,
    pub title: Read<String>,
    pub location: Read<Option<Url>>,
    pub capabilities: Read<SurfaceCapabilities>,
    requests: State<Vec<SurfaceRequest>>,
    active: Rc<Cell<bool>>,
}

impl Surface {
    /// Request a title change; admission is not an acknowledgment from the host.
    pub fn set_title(
        &self,
        rt: &mut Runtime,
        title: impl Into<String>,
    ) -> Result<Request, SurfaceError> {
        self.enqueue(rt, SurfaceCommand::SetTitle(title.into()))
    }

    /// Push or replace an application URL without loading a different document.
    pub fn navigate(
        &self,
        rt: &mut Runtime,
        reference: impl Into<String>,
        mode: HistoryMode,
    ) -> Result<Request, SurfaceError> {
        self.enqueue(
            rt,
            SurfaceCommand::Navigate {
                reference: reference.into(),
                mode,
            },
        )
    }

    fn enqueue(&self, rt: &mut Runtime, command: SurfaceCommand) -> Result<Request, SurfaceError> {
        if !self.active.get() {
            return Err(SurfaceError::Closed);
        }
        let capabilities = self.capabilities.get(rt);
        let supported = match command {
            SurfaceCommand::SetTitle(_) => capabilities.title,
            SurfaceCommand::Navigate { .. } => capabilities.navigation,
        };
        if !supported {
            return Err(SurfaceError::Unsupported);
        }
        if self.requests.get(rt).len() >= REQUEST_CAPACITY {
            return Err(SurfaceError::Full);
        }
        let status = State::new(rt, RequestStatus::Pending);
        let request = Request {
            status: status.read_only(),
        };
        self.requests
            .update(rt, |queue| queue.push(SurfaceRequest { command, status }));
        Ok(request)
    }
}

/// Adapter-owned writer for a surface in the same runtime. Close it before disposal.
pub struct SurfaceDriver {
    surface: Rc<Surface>,
    viewport: State<Option<Viewport>>,
    title: State<String>,
    location: State<Option<Url>>,
    observer: Observer,
}

impl SurfaceDriver {
    /// Install a stable root object without introducing a view wrapper.
    pub fn install(
        app: &mut App,
        title: String,
        location: Option<Url>,
        capabilities: SurfaceCapabilities,
    ) -> Self {
        let rt = app.runtime_mut();
        let viewport = State::new(rt, None);
        let title = State::new(rt, title);
        let location = State::new(rt, location);
        let capabilities = State::new(rt, capabilities);
        let requests = State::new(rt, Vec::new());
        let observer = rt.observer();
        let (_, dependencies) = rt.capture(|rt| {
            requests.get(rt);
        });
        rt.accept(&observer, dependencies);
        let surface = Rc::new(Surface {
            viewport: viewport.read_only(),
            title: title.read_only(),
            location: location.read_only(),
            capabilities: capabilities.read_only(),
            requests,
            active: Rc::new(Cell::new(true)),
        });
        app.set_environment_object(surface.clone());
        Self {
            surface,
            viewport,
            title,
            location,
            observer,
        }
    }

    pub fn surface(&self) -> Rc<Surface> {
        self.surface.clone()
    }

    pub fn publish_viewport(
        &self,
        rt: &mut Runtime,
        viewport: Viewport,
    ) -> Result<(), SurfaceError> {
        if !self.surface.active.get() {
            return Err(SurfaceError::Closed);
        }
        viewport.validate()?;
        self.viewport.set_if_changed(rt, Some(viewport));
        Ok(())
    }

    /// Publish an observation, without creating an outgoing request.
    pub fn publish_title(&self, rt: &mut Runtime, title: String) {
        if self.surface.active.get() {
            self.title.set_if_changed(rt, title);
        }
    }

    pub fn publish_location(&self, rt: &mut Runtime, location: Option<Url>) {
        if self.surface.active.get() {
            self.location.set_if_changed(rt, location);
        }
    }

    /// Drain once per host turn, even when the view tree has no pending patches.
    pub fn take_requests(&self, rt: &mut Runtime) -> Vec<SurfaceRequest> {
        if !self.observer.is_dirty(rt) {
            return Vec::new();
        }
        let mut requests = Vec::new();
        self.surface
            .requests
            .update(rt, |queue| requests = std::mem::take(queue));
        rt.mark_clean(&self.observer);
        requests
    }

    pub fn close(&self, rt: &mut Runtime) {
        if self.surface.active.replace(false) {
            for request in self.take_requests(rt) {
                request.complete(rt, Err(SurfaceError::Closed));
            }
        }
    }
}

impl Drop for SurfaceDriver {
    fn drop(&mut self) {
        self.surface.active.set(false);
    }
}

/// Native/test application history. Entry positions remain distinct for equal URLs.
#[derive(Default)]
pub struct MemoryHistory {
    entries: Vec<Url>,
    position: usize,
}

impl MemoryHistory {
    pub fn new(initial: Option<Url>) -> Self {
        Self {
            entries: initial.into_iter().collect(),
            position: 0,
        }
    }

    pub fn current(&self) -> Option<&Url> {
        self.entries.get(self.position)
    }

    pub fn navigate(&mut self, reference: &str, mode: HistoryMode) -> Result<&Url, SurfaceError> {
        let url = resolve_location(self.current(), reference)?;
        if self.entries.is_empty() {
            self.entries.push(url);
        } else if mode == HistoryMode::Replace {
            self.entries[self.position] = url;
        } else {
            self.entries.truncate(self.position + 1);
            self.entries.push(url);
            self.position += 1;
        }
        Ok(&self.entries[self.position])
    }

    /// Move through existing entries. Out-of-range traversal leaves history unchanged.
    pub fn go(&mut self, delta: isize) -> Option<&Url> {
        let position = self.position.checked_add_signed(delta)?;
        if position >= self.entries.len() {
            return None;
        }
        self.position = position;
        self.current()
    }
}

/// Resolve against the current application URL, not an HTML base element.
pub fn resolve_location(current: Option<&Url>, reference: &str) -> Result<Url, SurfaceError> {
    current
        .map_or_else(|| Url::parse(reference), |base| base.join(reference))
        .map_err(|error| SurfaceError::InvalidUrl(error.to_string()))
}
