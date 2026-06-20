//! Node references for TUI elements.

use crate::reactive::{Signal, create_signal};

use crate::node::NodeId;

/// A reference to a TUI node identity.
#[derive(Debug, Clone, Copy)]
pub struct NodeRef(Signal<Option<NodeId>>);

impl NodeRef {
    pub fn new() -> Self {
        Self(create_signal(None))
    }

    pub(crate) fn set(&self, id: NodeId) {
        self.0.set(Some(id));
    }

    pub fn get(&self) -> Option<NodeId> {
        self.0.get()
    }

    pub fn is_set(&self) -> bool {
        self.get().is_some()
    }
}

impl Default for NodeRef {
    fn default() -> Self {
        Self::new()
    }
}
