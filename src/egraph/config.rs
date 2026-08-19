//! Run configuration: stop conditions for equality saturation.

/// Stop conditions for [`crate::egraph::EGraph::run`]. Both limits are
/// independently configurable; saturation always stops early regardless.
#[derive(Clone, Copy, Debug)]
pub struct RunConfig {
    /// Maximum number of saturation iterations.
    pub max_iters: usize,
    /// Optional cap on the total e-node count; `None` disables the cap.
    pub node_limit: Option<usize>,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            max_iters: 30,
            node_limit: None,
        }
    }
}

impl RunConfig {
    /// Config with the given iteration limit and no node cap.
    pub fn with_iters(max_iters: usize) -> Self {
        RunConfig {
            max_iters,
            node_limit: None,
        }
    }

    /// Set the node-count cap.
    pub fn node_limit(mut self, limit: usize) -> Self {
        self.node_limit = Some(limit);
        self
    }
}
