//! Run budgets (FR-20, architecture §5.4, PRD §6.3).
//!
//! Two questions, one module:
//!
//! * **How much may one run spend on Offers?** [`BudgetSettings`] holds the
//!   user's per-run cap (default 30, FR-20). The run itself applies it when it
//!   picks the Offers the agent analyses (the daily search's agent pass): the
//!   ones past the cap stay on deterministic features and are marked
//!   `not_yet_analysed` in the Inbox, so the user sees a lead instead of a
//!   judgement. The pick is the run's own ingestion order, so it is
//!   deterministic: same Offers in, same split out.
//! * **How much may one task spend while it runs?** [`default_budget`] gives
//!   every task kind its turn, tool-call and wall-clock limits, and [`Meter`]
//!   counts a live run against them, so the runtime fails a run with
//!   `LimitExceeded` instead of letting it loop.

use std::time::Duration;

use boba_app::ports::agent::{AgentEvent, AgentTaskKind, RunBudget};
use serde::{Deserialize, Serialize};

/// The PRD's per-run offer budget (FR-20).
pub const DEFAULT_OFFERS_PER_RUN: u32 = 30;

/// The most offers one run may be asked to analyse: a sanity cap on a typo,
/// not a product limit.
pub const MAX_OFFERS_PER_RUN: u32 = 500;

/// Settings → AI engine: how many Offers one Search Run may hand to the agent
/// (FR-20). One `setting` row, read by every run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetSettings {
    pub offers_per_run: u32,
}

impl Default for BudgetSettings {
    fn default() -> Self {
        Self {
            offers_per_run: DEFAULT_OFFERS_PER_RUN,
        }
    }
}

impl BudgetSettings {
    /// Clamp into the range the run can honour: zero would mean "never analyse
    /// anything", which is a different feature, and a huge number is a typo.
    pub fn normalized(self) -> Self {
        Self {
            offers_per_run: self.offers_per_run.clamp(1, MAX_OFFERS_PER_RUN),
        }
    }
}

/// The per-task limits every task starts with (architecture §5.4): turns, MCP
/// tool calls and wall-clock time. Cost stays unreported-by-default: the core
/// never guesses what a CLI does not report (§6.3).
pub fn default_budget(kind: AgentTaskKind) -> RunBudget {
    let (turns, tool_calls, wall_clock) = match kind {
        AgentTaskKind::ProfileImport => (20, 40, Duration::from_secs(600)),
        AgentTaskKind::ExtractPostings => (30, 60, Duration::from_secs(600)),
        AgentTaskKind::NarrateScores => (12, 20, Duration::from_secs(300)),
        AgentTaskKind::TailorApplication => (40, 60, Duration::from_secs(900)),
        AgentTaskKind::ReviewApplication => (20, 30, Duration::from_secs(600)),
        AgentTaskKind::AnswerQuestions => (20, 30, Duration::from_secs(600)),
        AgentTaskKind::ClassifyEmails => (20, 20, Duration::from_secs(600)),
        AgentTaskKind::TranslateProfile => (12, 10, Duration::from_secs(300)),
    };
    RunBudget {
        wall_clock: Some(wall_clock),
        max_tool_calls: Some(tool_calls),
        max_turns: Some(turns),
        max_cost_usd: None,
    }
}

/// Fill the limits a caller left unset with [`default_budget`]'s, keeping every
/// explicit value: a call site may shorten the wall clock or raise the turns,
/// but never ends up with no limit at all.
pub fn with_defaults(kind: AgentTaskKind, mut budget: RunBudget) -> RunBudget {
    let defaults = default_budget(kind);
    if budget.wall_clock.is_none() {
        budget.wall_clock = defaults.wall_clock;
    }
    if budget.max_tool_calls.is_none() {
        budget.max_tool_calls = defaults.max_tool_calls;
    }
    if budget.max_turns.is_none() {
        budget.max_turns = defaults.max_turns;
    }
    if budget.max_cost_usd.is_none() {
        budget.max_cost_usd = defaults.max_cost_usd;
    }
    budget
}

/// Which limit a run hit (mapped to `FailureReason::LimitExceeded`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Turns,
    ToolCalls,
    Cost,
}

/// A run's consumption against its [`RunBudget`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meter {
    /// Assistant messages seen: one turn each, however many blocks it carried.
    pub turns: u32,
    pub tool_calls: u32,
    /// Reported cost, and whether the CLI reported any at all (§6.3).
    pub cost_usd: f64,
    pub cost_reported: bool,
}

impl Meter {
    /// Count one stdout line's normalised events, then answer which limit (if
    /// any) the run has now crossed.
    ///
    /// A turn is one assistant message — the batch a line carries — not one
    /// block: a message with prose and two tool calls is still one turn, and a
    /// tool-return line is not a turn at all.
    pub fn observe(&mut self, events: &[AgentEvent], budget: &RunBudget) -> Option<Limit> {
        let assistant_work = events
            .iter()
            .any(|event| matches!(event, AgentEvent::Text { .. } | AgentEvent::ToolCall { .. }));
        if assistant_work {
            self.turns += 1;
        }
        self.tool_calls += events
            .iter()
            .filter(|event| matches!(event, AgentEvent::ToolCall { .. }))
            .count() as u32;
        for event in events {
            if let AgentEvent::Usage {
                cost_usd: Some(cost),
                ..
            } = event
            {
                self.cost_usd += cost;
                self.cost_reported = true;
            }
        }
        self.breach(budget)
    }

    /// The limit `budget` puts this consumption over, if any.
    pub fn breach(&self, budget: &RunBudget) -> Option<Limit> {
        if budget.max_turns.is_some_and(|cap| self.turns > cap) {
            return Some(Limit::Turns);
        }
        if budget
            .max_tool_calls
            .is_some_and(|cap| self.tool_calls > cap)
        {
            return Some(Limit::ToolCalls);
        }
        if self.cost_reported && budget.max_cost_usd.is_some_and(|cap| self.cost_usd > cap) {
            return Some(Limit::Cost);
        }
        None
    }
}

/// Token usage as a run reports it (§6.3 cost). The core never guesses: a CLI
/// that reports nothing leaves every field `None` and the UI says so.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub cost_usd: Option<f64>,
}

impl Usage {
    /// Fold one reported `Usage` event in; absent values are not zero.
    pub fn absorb(
        &mut self,
        tokens_in: Option<i64>,
        tokens_out: Option<i64>,
        cost_usd: Option<f64>,
    ) {
        if let Some(value) = tokens_in {
            *self.tokens_in.get_or_insert(0) += value;
        }
        if let Some(value) = tokens_out {
            *self.tokens_out.get_or_insert(0) += value;
        }
        if let Some(value) = cost_usd {
            *self.cost_usd.get_or_insert(0.0) += value;
        }
    }

    /// Whether the CLI reported anything at all: `false` means the UI shows
    /// "not reported" rather than a fabricated zero.
    pub fn reported(&self) -> bool {
        self.tokens_in.is_some() || self.tokens_out.is_some() || self.cost_usd.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_budget_is_thirty_offers() {
        assert_eq!(BudgetSettings::default().offers_per_run, 30);
        assert_eq!(DEFAULT_OFFERS_PER_RUN, 30);
    }

    #[test]
    fn the_budget_is_clamped_to_a_workable_range() {
        let zero = BudgetSettings { offers_per_run: 0 }.normalized();
        assert_eq!(zero.offers_per_run, 1, "zero is never a budget");
        let huge = BudgetSettings {
            offers_per_run: u32::MAX,
        }
        .normalized();
        assert_eq!(huge.offers_per_run, MAX_OFFERS_PER_RUN);
        let fine = BudgetSettings { offers_per_run: 12 }.normalized();
        assert_eq!(fine.offers_per_run, 12);
    }

    #[test]
    fn per_task_limits_cover_turns_tool_calls_and_wall_clock() {
        for kind in AgentTaskKind::ALL {
            let budget = default_budget(kind);
            assert!(budget.max_turns.is_some(), "{kind:?} has no turn limit");
            assert!(
                budget.max_tool_calls.is_some(),
                "{kind:?} has no tool-call limit"
            );
            assert!(budget.wall_clock.is_some(), "{kind:?} has no wall clock");
            assert!(
                budget.max_cost_usd.is_none(),
                "{kind:?} must not invent a cost limit"
            );
        }
    }

    #[test]
    fn explicit_limits_survive_the_defaults() {
        let explicit = RunBudget {
            wall_clock: Some(Duration::from_secs(5)),
            max_turns: Some(3),
            ..RunBudget::default()
        };
        let filled = with_defaults(AgentTaskKind::ExtractPostings, explicit);
        assert_eq!(filled.wall_clock, Some(Duration::from_secs(5)));
        assert_eq!(filled.max_turns, Some(3));
        assert_eq!(
            filled.max_tool_calls,
            default_budget(AgentTaskKind::ExtractPostings).max_tool_calls,
            "an unset limit is filled with the task's default"
        );
    }

    #[test]
    fn a_turn_is_one_assistant_message_not_one_block() {
        let budget = RunBudget::default();
        let mut meter = Meter::default();
        assert_eq!(
            meter.observe(
                &[
                    AgentEvent::Text {
                        text: "thinking".to_string()
                    },
                    AgentEvent::ToolCall {
                        name: "mcp__boba__get_task_input".to_string(),
                        input: serde_json::Value::Null,
                    },
                ],
                &budget,
            ),
            None
        );
        assert_eq!(meter.turns, 1, "one message is one turn");
        assert_eq!(meter.tool_calls, 1);

        // A tool-return line is not a turn.
        assert_eq!(
            meter.observe(
                &[AgentEvent::ToolResult {
                    name: "mcp__boba__get_task_input".to_string(),
                    output: "{}".to_string(),
                }],
                &budget,
            ),
            None
        );
        assert_eq!(meter.turns, 1);
    }

    #[test]
    fn the_meter_trips_the_turn_limit() {
        let budget = RunBudget {
            max_turns: Some(2),
            ..RunBudget::default()
        };
        let mut meter = Meter::default();
        let turn = [AgentEvent::Text {
            text: "hello".to_string(),
        }];
        assert_eq!(meter.observe(&turn, &budget), None);
        assert_eq!(meter.observe(&turn, &budget), None);
        assert_eq!(meter.observe(&turn, &budget), Some(Limit::Turns));
    }

    #[test]
    fn the_meter_trips_the_tool_call_limit() {
        let budget = RunBudget {
            max_tool_calls: Some(1),
            ..RunBudget::default()
        };
        let mut meter = Meter::default();
        let call = [AgentEvent::ToolCall {
            name: "mcp__boba__propose_posting_features".to_string(),
            input: serde_json::Value::Null,
        }];
        assert_eq!(meter.observe(&call, &budget), None);
        assert_eq!(meter.observe(&call, &budget), Some(Limit::ToolCalls));
    }

    #[test]
    fn the_meter_trips_the_cost_limit_only_when_reported() {
        let budget = RunBudget {
            max_cost_usd: Some(0.05),
            ..RunBudget::default()
        };
        let mut meter = Meter::default();
        // A usage event with no cost tells the meter nothing.
        assert_eq!(
            meter.observe(
                &[AgentEvent::Usage {
                    tokens_in: Some(100),
                    tokens_out: Some(50),
                    cost_usd: None,
                }],
                &budget,
            ),
            None
        );
        assert!(!meter.cost_reported);
        assert_eq!(
            meter.observe(
                &[AgentEvent::Usage {
                    tokens_in: None,
                    tokens_out: None,
                    cost_usd: Some(0.06),
                }],
                &budget,
            ),
            Some(Limit::Cost)
        );
    }

    #[test]
    fn usage_accumulates_reported_values_only() {
        let mut usage = Usage::default();
        assert!(!usage.reported(), "nothing reported is not a zero");
        usage.absorb(Some(10), None, Some(0.5));
        usage.absorb(Some(5), Some(3), None);
        assert_eq!(usage.tokens_in, Some(15));
        assert_eq!(usage.tokens_out, Some(3));
        assert_eq!(usage.cost_usd, Some(0.5));
        assert!(usage.reported());
    }
}
