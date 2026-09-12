//! Contract between the integrated client compositor and an optional presentation destination.
//!
//! The contract is intentionally ephemeral: it carries the semantic organization operations
//! produced by the current shell render together with the complete composed frame. It does not
//! own selection, focus, endpoint routing, or terminal input.

use std::fmt;

use ratatui::layout::Rect;

use super::shell::ClientComposedSurfacePatch;
use crate::api::schema::AgentStatus;
use crate::protocol::FrameData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntegratedOfficeNodeKind {
    Root,
    Coordinator,
    Manager,
    Worker,
    Group,
    Unbound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntegratedOfficeStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl From<AgentStatus> for IntegratedOfficeStatus {
    fn from(status: AgentStatus) -> Self {
        match status {
            AgentStatus::Idle => Self::Idle,
            AgentStatus::Working => Self::Working,
            AgentStatus::Blocked => Self::Blocked,
            AgentStatus::Done => Self::Done,
            AgentStatus::Unknown => Self::Unknown,
        }
    }
}

/// One semantic organization item emitted beside the existing row geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntegratedOfficeNode {
    /// Stable organization key used by the existing shell navigation.
    pub(crate) key: String,
    pub(crate) parent_key: Option<String>,
    pub(crate) kind: IntegratedOfficeNodeKind,
    pub(crate) name: String,
    pub(crate) role: Option<String>,
    pub(crate) status: IntegratedOfficeStatus,
    pub(crate) attention: usize,
    pub(crate) pane_id: Option<String>,
    pub(crate) rect: Rect,
    pub(crate) selected: bool,
    pub(crate) focused: bool,
    pub(crate) focusable: bool,
    pub(crate) meta: String,
    pub(crate) box_start: bool,
    pub(crate) box_end: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IntegratedOfficeOperation {
    Group(IntegratedOfficeNode),
    Person(IntegratedOfficeNode),
}

/// Organization hit data that is derived from the same row rects as the semantic operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntegratedOrganizationHit {
    pub(crate) key: String,
    pub(crate) pane_id: Option<String>,
    pub(crate) rect: Rect,
    pub(crate) focusable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntegratedPresentationIdentity {
    pub(crate) endpoint_id: String,
    pub(crate) generation: u64,
    pub(crate) boot_id: String,
    pub(crate) projection_revision: u64,
    pub(crate) surface_revision: u64,
}

pub(crate) struct IntegratedPresentationCollector {
    pub(crate) organization: Vec<IntegratedOfficeOperation>,
    pub(crate) organization_hits: Vec<IntegratedOrganizationHit>,
    /// Complete visual rectangles occupied by common client overlays in the final compositor
    /// pass. An integrated destination must keep these above office operations. An empty list
    /// means no complete overlay rectangle was exported; `office_operations_allowed` separately
    /// rejects office painting when an open overlay is not covered by this contract.
    pub(crate) overlay_occlusions: Vec<Rect>,
    /// False when an open common overlay has no complete visual rectangle in the compositor.
    /// A destination must preserve the complete frame and skip office painting in that case.
    pub(crate) office_operations_allowed: bool,
}

impl IntegratedPresentationCollector {
    pub(crate) fn with_overlay_occlusions(
        mut self,
        regions: Vec<Rect>,
        complete_coverage: bool,
    ) -> Self {
        self.overlay_occlusions = regions;
        self.office_operations_allowed = complete_coverage;
        self
    }

    pub(crate) fn finish(
        self,
        identity: IntegratedPresentationIdentity,
    ) -> IntegratedPresentationPayload {
        IntegratedPresentationPayload {
            identity,
            organization: self.organization,
            organization_hits: self.organization_hits,
            overlay_occlusions: self.overlay_occlusions,
            office_operations_allowed: self.office_operations_allowed,
        }
    }
}

impl Default for IntegratedPresentationCollector {
    fn default() -> Self {
        Self {
            organization: Vec::new(),
            organization_hits: Vec::new(),
            overlay_occlusions: Vec::new(),
            office_operations_allowed: true,
        }
    }
}

pub(crate) struct IntegratedPresentationPayload {
    pub(crate) identity: IntegratedPresentationIdentity,
    pub(crate) organization: Vec<IntegratedOfficeOperation>,
    pub(crate) organization_hits: Vec<IntegratedOrganizationHit>,
    pub(crate) overlay_occlusions: Vec<Rect>,
    pub(crate) office_operations_allowed: bool,
}

impl IntegratedPresentationPayload {
    pub(crate) fn with_frame(self, frame: FrameData) -> IntegratedPresentationFrame {
        IntegratedPresentationFrame {
            identity: self.identity,
            organization: self.organization,
            organization_hits: self.organization_hits,
            overlay_occlusions: self.overlay_occlusions,
            office_operations_allowed: self.office_operations_allowed,
            frame,
        }
    }
}

/// The full composed client presentation consumed by ANSI and by an opt-in destination.
pub(crate) struct IntegratedPresentationFrame {
    pub(crate) identity: IntegratedPresentationIdentity,
    pub(crate) organization: Vec<IntegratedOfficeOperation>,
    pub(crate) organization_hits: Vec<IntegratedOrganizationHit>,
    pub(crate) overlay_occlusions: Vec<Rect>,
    pub(crate) office_operations_allowed: bool,
    pub(crate) frame: FrameData,
}

/// Explicit update kinds keep freeze/full/patch behavior visible to a destination.
pub(crate) enum IntegratedPresentationUpdate<'a> {
    Frozen,
    /// Resume after a source handoff. The next update is a complete frame, never a patch
    /// against a frame the destination may not have received.
    Thaw,
    Full(&'a IntegratedPresentationFrame),
    Patch {
        frame: &'a IntegratedPresentationFrame,
        patch: &'a ClientComposedSurfacePatch,
    },
    Clear(IntegratedPresentationIdentity),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntegratedPresentationError {
    pub(crate) message: String,
}

impl IntegratedPresentationError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for IntegratedPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for IntegratedPresentationError {}

/// Optional presentation destination. It must not become an owner of client state or input.
pub(crate) trait IntegratedPresentationSink {
    fn present(
        &mut self,
        update: IntegratedPresentationUpdate<'_>,
    ) -> Result<(), IntegratedPresentationError>;
}
