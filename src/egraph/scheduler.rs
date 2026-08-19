//! Rule scheduling for equality saturation.
//!
//! A [`RewriteScheduler`] decides, each iteration, which rules may fire and
//! observes how many matches each produced. This is how equality saturation
//! stays tractable: rather than letting every rule fire on every iteration, a
//! scheduler can throttle rules that match explosively so cheaper, more
//! targeted rules get a turn.

use std::collections::HashMap;

use crate::debug::*;

use super::egraph::Analysis;
use super::lang::*;

/// Decides, each iteration, which rules may fire and observes how many matches
/// each produced.
///
/// The two hooks are called by [`crate::egraph::EGraph::run_with_scheduler`]:
/// - [`is_enabled`](RewriteScheduler::is_enabled) — before matching a rule.
/// - [`on_matches`](RewriteScheduler::on_matches) — after matching it, with the
/// number of matches found this iteration.
pub trait RewriteScheduler<L: Language, N: Analysis<L>> {
    /// Return `false` to skip (ban) rule `rule_idx` on this `iter`.
    fn is_enabled(&mut self, _iter: usize, _rule_idx: usize, _name: &str) -> bool {
        true
    }

    /// Observe how many matches rule `rule_idx` produced this `iter`. The
    /// scheduler may use this to ban the rule on future iterations.
    fn on_matches(&mut self, _iter: usize, _rule_idx: usize, _name: &str, _n_matches: usize) {}
}

/// The trivial scheduler: every rule is always enabled. This reproduces the
/// naive "fire everything every iteration" behavior.
#[derive(Default)]
pub struct SimpleScheduler;

impl<L: Language, N: Analysis<L>> RewriteScheduler<L, N> for SimpleScheduler {}

/// Per-rule bookkeeping for [`BackoffScheduler`].
struct RuleStats {
    /// The rule is banned for every iteration strictly before this one.
    banned_until: usize,
    /// How many times this rule has been banned so far (drives the exponential
    /// back-off: each ban lasts longer than the last).
    times_banned: usize,
    /// If a rule produces more than this many matches in one iteration itgets
    /// banned. The threshold grows each time so the rule is eventually allowed
    /// to do more work.
    match_limit: usize,
    /// Base ban duration in iterations; the actual ban is this shifted left by
    /// `times_banned`.
    ban_length: usize,
}

/// egg-style back-off scheduler. A rule that matches more than its current
/// `match_limit` in a single iteration is banned for an exponentially growing
/// number of iterations, and its threshold is doubled. This tames rules like
/// commutativity/associativity that would otherwise match combinatorially and
/// starve the rest of the rule set.
pub struct BackoffScheduler {
    stats: HashMap<usize, RuleStats>,
    default_match_limit: usize,
    default_ban_length: usize,
}

impl Default for BackoffScheduler {
    fn default() -> Self {
        BackoffScheduler {
            stats: HashMap::new(),
            default_match_limit: 1_000,
            default_ban_length: 2,
        }
    }
}

impl BackoffScheduler {
    /// Set the default match threshold applied to every rule.
    pub fn with_match_limit(mut self, limit: usize) -> Self {
        self.default_match_limit = limit;
        self
    }

    /// Set the base ban length (in iterations) applied to every rule.
    pub fn with_ban_length(mut self, length: usize) -> Self {
        self.default_ban_length = length;
        self
    }

    fn stats_for(&mut self, idx: usize) -> &mut RuleStats {
        let (limit, ban) = (self.default_match_limit, self.default_ban_length);
        self.stats.entry(idx).or_insert(RuleStats {
            banned_until: 0,
            times_banned: 0,
            match_limit: limit,
            ban_length: ban,
        })
    }
}

impl<L: Language, N: Analysis<L>> RewriteScheduler<L, N> for BackoffScheduler {
    fn is_enabled(&mut self, iter: usize, rule_idx: usize, _name: &str) -> bool {
        iter >= self.stats_for(rule_idx).banned_until
    }

    fn on_matches(&mut self, iter: usize, rule_idx: usize, name: &str, n_matches: usize) {
        let stats = self.stats_for(rule_idx);
        if n_matches > stats.match_limit {
            stats.times_banned += 1;
            let ban = stats.ban_length << stats.times_banned;
            stats.banned_until = iter + 1 + ban;
            // Raise the bar so the rule is allowed to match more next time.
            stats.match_limit *= 2;
            debug_string(format!(
                "Banned rule '{name}' for {ban} iteration(s) ({n_matches} matches > limit); new limit {}.",
                stats.match_limit
            ));
        }
    }
}
