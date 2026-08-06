//! GOAP (Goal-Oriented Action Planning).
//!
//! A F.E.A.R.-style decision layer. The world is described as a set of named
//! boolean symbols ([`GoapState`]); the agent is described by what it can do,
//! with preconditions and effects over those symbols ([`GoapAction`]), and by
//! what it wants to be true ([`GoapGoal`]). [`GoapPlanner::plan`] then searches
//! for the cheapest action sequence reaching the highest-priority reachable goal.
//!
//! The search is a forward uniform-cost (Dijkstra) search over boolean world
//! states; see [`GoapPlanner::plan`] for its optimality conditions and its cost.

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::rc::Rc;

/// A world state as the planner sees it: a set of named boolean symbols.
///
/// A symbol is `true`, `false`, or *absent*, and absent is not the same as
/// `false`. Every condition test ([`GoapState::meets_conditions`]) demands the
/// symbol be present with exactly the value asked for, so an unset symbol fails a
/// `false` requirement just as it fails a `true` one. Seed the starting state
/// with every symbol your preconditions and goals mention.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoapState {
    /// The symbol table, keyed by symbol name.
    ///
    /// An absent key means "unknown" and satisfies nothing (see the type-level
    /// note). The planner identifies already-visited search nodes by the contents
    /// of this map alone, so two states with the same key/value pairs are the same
    /// node no matter which actions produced them.
    pub values: HashMap<String, bool>,
}

impl GoapState {
    /// An empty state in which no symbol is known.
    ///
    /// Since absent symbols satisfy nothing, planning from here can only begin
    /// with actions that have no preconditions at all.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Sets `key` to `value`, inserting the symbol if it was not known before.
    ///
    /// The key is copied into an owned `String` on every call. There is no way to
    /// make a symbol *absent* again through this method; that requires touching
    /// [`GoapState::values`] directly.
    pub fn set(&mut self, key: &str, value: bool) {
        self.values.insert(key.to_string(), value);
    }

    /// Reads a symbol, returning `None` when it was never set.
    ///
    /// `None` and `Some(false)` are deliberately distinct: nothing in this module
    /// treats an unknown symbol as false.
    pub fn get(&self, key: &str) -> Option<bool> {
        self.values.get(key).copied()
    }

    /// Returns `true` when every entry of `conditions` is present in this state
    /// with exactly that value. This is the precondition and goal test.
    ///
    /// An empty `conditions` map is vacuously satisfied. A symbol this state has
    /// never seen counts as unmet, so there is no "don't care" value: conditions
    /// you leave out are ignored, conditions you list must be *known*.
    pub fn meets_conditions(&self, conditions: &HashMap<String, bool>) -> bool {
        for (k, v) in conditions {
            if self.get(k) != Some(*v) {
                return false;
            }
        }
        true
    }

    /// Applies an action's effects to this state.
    ///
    /// Each entry overwrites this state's value for that key, inserting it when
    /// new; symbols not named in `effects` are left untouched. Effects are
    /// absolute assignments rather than deltas, so applying the same map twice
    /// changes nothing the second time and the order of the entries is irrelevant.
    pub fn apply_effects(&mut self, effects: &HashMap<String, bool>) {
        for (k, v) in effects {
            self.set(k, *v);
        }
    }

    /// Counts, as an `f32`, how many entries of `goal` this state does not
    /// currently satisfy. Returns `0.0` when the goal already holds.
    ///
    /// **The planner does not use this.** As an A* heuristic it is inadmissible:
    /// it charges 1.0 per unmet condition, which overestimates the true remaining
    /// cost whenever an action costs less than 1.0 or satisfies several conditions
    /// at once, and that broke the optimality of the returned plan.
    /// [`GoapPlanner`] therefore searches with `h = 0`. What remains here is a
    /// plain "how many symbols are off" count, unweighted by action cost.
    pub fn distance_to(&self, goal: &HashMap<String, bool>) -> f32 {
        let mut dist = 0.0;
        for (k, v) in goal {
            if self.get(k) != Some(*v) {
                dist += 1.0;
            }
        }
        dist
    }
}

/// One action the agent can perform, as the planner sees it.
///
/// This is a planning-time description only: it carries no executable code. The
/// planner reasons about the symbols in `preconditions`/`effects` and hands back
/// the chosen actions; mapping [`GoapAction::name`] onto actual behaviour is the
/// caller's job.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GoapAction {
    /// Free-form label identifying the action to the caller and to tracing output.
    ///
    /// The planner never compares names, so duplicates are permitted, and the same
    /// action may legitimately appear more than once in one plan.
    pub name: String,

    /// What performing this action costs, in caller-defined units — the planner
    /// only ever sums costs and compares the sums, so any consistent scale works
    /// (seconds, metres, "risk points").
    ///
    /// Must be finite and non-negative. The search is uniform-cost with a
    /// visited-set and no decrease-key step, which assumes non-negative edge
    /// weights: a negative cost can yield a non-optimal plan. A NaN cost poisons
    /// every running total derived from it and parks those nodes at one end of the
    /// queue. Zero is fine.
    pub cost: f32,

    /// Symbols that must already hold for this action to be applicable, tested
    /// with [`GoapState::meets_conditions`].
    ///
    /// A symbol the state has never seen counts as unmet, so a precondition must
    /// be established explicitly — by the starting state or by an earlier action's
    /// effect. An empty map means the action is always applicable.
    pub preconditions: HashMap<String, bool>,

    /// Symbols this action sets when performed, applied with
    /// [`GoapState::apply_effects`].
    ///
    /// The planner assumes the effects are exact and always achieved; symbols not
    /// listed are carried into the next state unchanged. An empty map makes the
    /// action inert (see [`GoapAction::new`]).
    pub effects: HashMap<String, bool>,
}

impl GoapAction {
    /// A new action with no preconditions (hence always applicable) and no
    /// effects.
    ///
    /// An action left without effects can never contribute to a plan: the state it
    /// produces equals the state it was applied to, which the planner has already
    /// marked visited, so it only consumes search time. `cost` is stored as given
    /// — see [`GoapAction::cost`] for the range it must lie in.
    pub fn new(name: &str, cost: f32) -> Self {
        Self {
            name: name.to_string(),
            cost,
            preconditions: HashMap::new(),
            effects: HashMap::new(),
        }
    }

    /// Builder step: require `key` to equal `value` before this action may run.
    ///
    /// Repeating the same key overwrites the previous requirement — the last call
    /// wins, an action cannot demand a symbol be both `true` and `false`.
    pub fn add_precondition(mut self, key: &str, value: bool) -> Self {
        self.preconditions.insert(key.to_string(), value);
        self
    }

    /// Builder step: make this action set `key` to `value` when performed.
    ///
    /// Repeating the same key overwrites the previous effect. All effects land
    /// simultaneously, so one effect of an action can never serve as the
    /// precondition of another effect of the same action.
    pub fn add_effect(mut self, key: &str, value: bool) -> Self {
        self.effects.insert(key.to_string(), value);
        self
    }
}

/// A state of the world the agent would like to bring about, together with how
/// strongly it wants it relative to its other goals.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GoapGoal {
    /// Free-form label. Used only for tracing output; the planner never compares
    /// goals by name and does not require them to be unique.
    pub name: String,

    /// Selection order only, in caller-defined units — only the relative order
    /// matters.
    ///
    /// [`GoapPlanner::plan`] sorts goals by descending priority and returns the
    /// plan for the first one it can reach. The sort is stable, so goals of equal
    /// priority are tried in the order they appear in the slice handed to the
    /// planner. Priority does not enter the plan's cost: a reachable
    /// high-priority goal is chosen even when its plan is far more expensive than
    /// a lower-priority goal's.
    pub priority: f32,

    /// The symbols that must hold for this goal to count as reached, tested with
    /// [`GoapState::meets_conditions`] — a symbol the state has never seen counts
    /// as unmet.
    ///
    /// An empty map is satisfied by every state, so such a goal always "succeeds"
    /// with an empty plan and, at the highest priority, shadows every other goal.
    pub desired_state: HashMap<String, bool>,
}

impl GoapGoal {
    /// A new goal with an empty desired state.
    ///
    /// Handed to the planner as-is it is satisfied immediately and plans to an
    /// empty action list, so add at least one
    /// [`GoapGoal::add_desired_state`] entry first.
    pub fn new(name: &str, priority: f32) -> Self {
        Self {
            name: name.to_string(),
            priority,
            desired_state: HashMap::new(),
        }
    }

    /// Builder step: require `key` to equal `value` for this goal to be reached.
    ///
    /// Repeating the same key overwrites the previous requirement — the last call
    /// wins.
    pub fn add_desired_state(mut self, key: &str, value: bool) -> Self {
        self.desired_state.insert(key.to_string(), value);
        self
    }
}

#[derive(Clone)]
struct PlanNode {
    state: GoapState,
    g_cost: f32, // Path cost
    h_cost: f32, // Heuristic cost
    action: Option<GoapAction>,
    /// Shared, not owned. With `Box<PlanNode>` here the derived `Clone` recursed
    /// down the whole ancestor chain, so `build_plan` copied every ancestor's
    /// `GoapState` (a `HashMap<String, bool>`, one allocation per key) into every
    /// successor it generated — see the expansion loop for the cost shape. `Rc`
    /// makes a successor share its parent instead; siblings then point at one
    /// copy, and the chain is freed when the last node referencing it goes.
    ///
    /// `Rc` (not `Arc`) is deliberate: `PlanNode` is private to this module, never
    /// escapes `build_plan` (which returns `Vec<GoapAction>`), and the planner is
    /// stateless, so nothing here crosses a thread boundary.
    parent: Option<Rc<PlanNode>>,
}

impl PartialEq for PlanNode {
    fn eq(&self, other: &Self) -> bool {
        self.f_cost() == other.f_cost()
    }
}

impl Eq for PlanNode {}

impl Ord for PlanNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Ters çeviriyoruz ki BinaryHeap Min-Heap gibi davransın
        other.f_cost().total_cmp(&self.f_cost())
    }
}

impl PartialOrd for PlanNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PlanNode {
    fn f_cost(&self) -> f32 {
        self.g_cost + self.h_cost
    }
}

/// Stateless entry point for planning.
///
/// The type carries no data and is never instantiated; [`GoapPlanner::plan`] is
/// an associated function and allocates everything it needs per call, so
/// concurrent plans do not interfere.
pub struct GoapPlanner;

impl GoapPlanner {
    /// Chooses a goal and returns the cheapest action sequence that reaches it, in
    /// execution order (element 0 is performed first).
    ///
    /// Goals are tried by descending [`GoapGoal::priority`] and the first
    /// *reachable* one wins; the planner never falls back to a cheaper
    /// lower-priority goal once a higher-priority one can be reached. `None` is
    /// returned only when no goal at all is reachable from `current_state` with
    /// `actions` — in practice that usually means a missing or mis-specified
    /// action rather than a genuine dead end. An empty `goals` slice likewise
    /// gives `None`, while a goal that already holds gives `Some(vec![])`.
    ///
    /// The search is a forward uniform-cost search (Dijkstra, `h = 0`) over
    /// boolean world states, so the plan is cost-optimal for the chosen goal
    /// **provided every [`GoapAction::cost`] is non-negative**. An action may
    /// appear several times in one plan.
    ///
    /// Cost and limits: worst case exponential in the number of distinct symbols,
    /// with no node, depth or wall-clock budget — the call runs until it finds a
    /// plan or exhausts the reachable state space. Every expanded state and every
    /// returned action is cloned, and the `goals` slice is copied to be sorted.
    #[tracing::instrument(skip_all, name = "goap_plan")]
    pub fn plan(
        current_state: &GoapState,
        actions: &[GoapAction],
        goals: &[GoapGoal],
    ) -> Option<Vec<GoapAction>> {
        // Hedefleri önceliğe göre sırala (en yüksek öncelikli önce)
        let mut sorted_goals = goals.to_vec();
        sorted_goals.sort_by(|a, b| b.priority.total_cmp(&a.priority));

        for goal in sorted_goals {
            if let Some(plan) = Self::build_plan(current_state, actions, &goal.desired_state) {
                // Hedef seçimi — hangi hedef planlanabildi ve kaç adımda.
                tracing::debug!(
                    goal = %goal.name,
                    priority = goal.priority,
                    plan_len = plan.len(),
                    "[AI] GOAP hedefi seçildi, plan bulundu"
                );
                return Some(plan);
            }
            tracing::trace!(
                goal = %goal.name,
                priority = goal.priority,
                "[AI] GOAP hedefi için ulaşılabilir plan yok, sıradaki hedefe geçiliyor"
            );
        }
        // Hiçbir hedef karşılanamadı — ajan kararsız kalır, bu genelde eksik/yanlış
        // aksiyon tanımına işaret eder.
        tracing::warn!(
            goal_count = goals.len(),
            action_count = actions.len(),
            "[AI] GOAP planlaması başarısız — ulaşılabilir hedef yok"
        );
        None
    }

    #[tracing::instrument(skip_all, name = "goap_build_plan")]
    fn build_plan(
        start_state: &GoapState,
        actions: &[GoapAction],
        goal_state: &HashMap<String, bool>,
    ) -> Option<Vec<GoapAction>> {
        let mut open_list = BinaryHeap::new();
        // Ziyaret edilen durumların hash'i (state hashable olmalı, basitlik için String representasyonu)
        let mut closed_list = HashSet::new();
        // A* genişletme sayacı — iç döngüde per-düğüm log yerine çıkışta AGGREGATE.
        let mut nodes_expanded = 0u32;

        // Use h = 0 (uniform-cost / Dijkstra). The "unsatisfied-condition count"
        // heuristic (`distance_to`) is INADMISSIBLE — it charges 1.0 per unmet goal
        // condition, which overestimates whenever an action costs < 1.0 or satisfies
        // several conditions at once, so A* could commit to a pricier plan and skip
        // the cheaper one. Combined with the no-revisit `closed_list` (no decrease-key),
        // that broke the documented "optimal plan" guarantee. With h = 0 the first pop
        // of a state is always its cheapest, so the closed_list is correct and the plan
        // is optimal regardless of action costs.
        let start_node = PlanNode {
            state: start_state.clone(),
            g_cost: 0.0,
            h_cost: 0.0,
            action: None,
            parent: None,
        };

        open_list.push(start_node);

        while let Some(current) = open_list.pop() {
            // Hedefe ulaşıldı mı?
            if current.state.meets_conditions(goal_state) {
                let mut plan = Vec::new();
                // Walk the (now shared) parent chain back to the start node, which is
                // the only one with `action: None`.
                let mut cursor = Some(&current);
                while let Some(node) = cursor {
                    match &node.action {
                        Some(action) => plan.push(action.clone()),
                        None => break,
                    }
                    cursor = node.parent.as_deref();
                }
                plan.reverse();
                tracing::debug!(
                    plan_len = plan.len(),
                    cost = current.g_cost,
                    nodes_expanded,
                    "[AI] GOAP A* planı bulundu"
                );
                return Some(plan);
            }

            // Durumu serialize edip set'e ekle (aynı state loop'a girmemek için).
            // Scoped so the borrow of `current.state` is provably over before
            // `current` is moved into the `Rc` below.
            let state_hash = {
                let mut state_keys: Vec<_> = current.state.values.iter().collect();
                state_keys.sort_by_key(|(k, _)| *k);
                format!("{:?}", state_keys)
            };

            if closed_list.contains(&state_hash) {
                continue;
            }
            closed_list.insert(state_hash);
            nodes_expanded += 1;

            // Hand `current` to its successors by reference count instead of by deep
            // copy. Cost shape of the old `Some(Box::new(current.clone()))`: cloning a
            // node at depth d cloned d+1 nodes, hence d+1 `GoapState` HashMaps (each
            // key its own `String` allocation), and it happened once per applicable
            // action — so expanding one node cost A*(d+1) map clones, and every entry
            // waiting in the open list privately owned its whole ancestry, making the
            // live heap O(nodes * depth * symbols) instead of O(nodes * symbols).
            //
            // UNMEASURED: no benchmark was run (cargo is not available in this
            // session), and grepping the workspace finds no caller of
            // `GoapPlanner::plan` outside this module's own tests, so the wall-clock
            // saving on a real agent load is unknown — only the asymptotics above are
            // established. The change is allocation-only: the nodes pushed, their
            // costs and the pop order are bit-identical, so the returned plan is too.
            let current = Rc::new(current);
            for action in actions {
                if current.state.meets_conditions(&action.preconditions) {
                    let mut new_state = current.state.clone();
                    new_state.apply_effects(&action.effects);

                    let next_node = PlanNode {
                        // `new_state` moves in; it used to be cloned into the node and
                        // then dropped unused, one extra map copy per successor.
                        state: new_state,
                        g_cost: current.g_cost + action.cost,
                        h_cost: 0.0, // Dijkstra — see the start node above.
                        action: Some(action.clone()),
                        parent: Some(Rc::clone(&current)),
                    };

                    open_list.push(next_node);
                }
            }
        }

        tracing::trace!(
            nodes_expanded,
            "[AI] GOAP A* açık liste tükendi — bu hedef bu aksiyonlarla ulaşılamaz"
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(pairs: &[(&str, bool)]) -> GoapState {
        let mut s = GoapState::new();
        for (k, v) in pairs {
            s.set(k, *v);
        }
        s
    }

    fn single_goal(pairs: &[(&str, bool)]) -> Vec<GoapGoal> {
        let mut g = GoapGoal::new("g", 1.0);
        for (k, v) in pairs {
            g = g.add_desired_state(k, *v);
        }
        vec![g]
    }

    #[test]
    fn plan_returns_cheapest_path_when_actions_cost_below_one() {
        let start = state(&[]);
        let goal = single_goal(&[("g", true)]);
        let actions = vec![
            GoapAction::new("direct", 1.0).add_effect("g", true),
            GoapAction::new("setup", 0.1).add_effect("step", true),
            GoapAction::new("cheap", 0.1)
                .add_precondition("step", true)
                .add_effect("g", true),
        ];
        let plan = GoapPlanner::plan(&start, &actions, &goal).expect("a plan exists");
        let total: f32 = plan.iter().map(|a| a.cost).sum();
        // Optimal is setup+cheap = 0.2, not the single "direct" action = 1.0. The old
        // inadmissible count heuristic overestimated the 2-step path (h = 1 for a state
        // one cheap 0.1 action from the goal) and committed to "direct".
        assert!(
            total < 0.5,
            "expected the cheap ~0.2 plan, got {total}: {:?}",
            plan.iter().map(|a| a.name.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn single_action_reaches_goal() {
        let cur = state(&[("armed", false)]);
        let actions = vec![GoapAction::new("arm", 1.0).add_effect("armed", true)];
        let plan = GoapPlanner::plan(&cur, &actions, &single_goal(&[("armed", true)])).expect("plan");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].name, "arm");
    }

    #[test]
    fn chains_preconditioned_actions() {
        // Goal: enemy_dead. Need a weapon first.
        let cur = state(&[("has_weapon", false), ("enemy_dead", false)]);
        let actions = vec![
            GoapAction::new("pick_up", 1.0).add_effect("has_weapon", true),
            GoapAction::new("attack", 1.0)
                .add_precondition("has_weapon", true)
                .add_effect("enemy_dead", true),
        ];
        let plan = GoapPlanner::plan(&cur, &actions, &single_goal(&[("enemy_dead", true)])).expect("plan");
        assert_eq!(
            plan.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["pick_up", "attack"]
        );
    }

    #[test]
    fn already_satisfied_goal_gives_empty_plan() {
        let cur = state(&[("safe", true)]);
        let actions = vec![GoapAction::new("hide", 1.0).add_effect("safe", true)];
        let plan = GoapPlanner::plan(&cur, &actions, &single_goal(&[("safe", true)])).expect("plan");
        assert!(plan.is_empty(), "no actions needed when the goal already holds");
    }

    #[test]
    fn unachievable_goal_returns_none() {
        let cur = state(&[("has_key", false)]);
        // No action produces `door_open`.
        let actions = vec![GoapAction::new("noop", 1.0).add_effect("has_key", true)];
        assert!(GoapPlanner::plan(&cur, &actions, &single_goal(&[("door_open", true)])).is_none());
    }

    #[test]
    fn picks_the_cheapest_plan() {
        // Two ways to reach the goal; A* must return the cheaper one.
        let cur = state(&[("x", false)]);
        let actions = vec![
            GoapAction::new("expensive", 5.0).add_effect("x", true),
            GoapAction::new("cheap", 1.0).add_effect("x", true),
        ];
        let plan = GoapPlanner::plan(&cur, &actions, &single_goal(&[("x", true)])).expect("plan");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].name, "cheap", "planner must choose the cheaper action");
    }

    #[test]
    fn prefers_two_cheap_over_one_expensive_multi_condition() {
        // Goal {a,b}. One action does both at cost 3; two actions do a then b at
        // total cost 2. Optimal = the pair. (Guards against the multi-condition
        // heuristic over-estimating and closing the goal via the pricier route.)
        let cur = state(&[("a", false), ("b", false)]);
        let actions = vec![
            GoapAction::new("both", 3.0).add_effect("a", true).add_effect("b", true),
            GoapAction::new("do_a", 1.0).add_effect("a", true),
            GoapAction::new("do_b", 1.0).add_precondition("a", true).add_effect("b", true),
        ];
        let plan = GoapPlanner::plan(&cur, &actions, &single_goal(&[("a", true), ("b", true)])).expect("plan");
        let total: f32 = plan.iter().map(|a| a.cost).sum();
        assert_eq!(total, 2.0, "planner returned a suboptimal plan (cost {total}, expected 2.0): {:?}",
            plan.iter().map(|a| a.name.as_str()).collect::<Vec<_>>());
    }

    // NOT a regression test, and deliberately labelled as such: switching the parent
    // link from `Box` to `Rc` is allocation-only, so no assertion can distinguish the
    // two versions by observable behaviour (the only difference — how many
    // `GoapState` maps get copied — is invisible from outside, and a timing or
    // stack-depth oracle would need a chain far deeper than the old quadratic clone
    // could reach in test time). What this DOES cover is the one part of that change
    // that could break: the rewritten walk back up the shared parent chain. Depth 8
    // vs. the depth-2 chain the other tests reach.
    #[test]
    fn long_chain_reconstructs_in_execution_order() {
        const N: usize = 8;
        let mut actions = Vec::new();
        for i in 0..N {
            let mut a = GoapAction::new(&format!("step{i}"), 1.0);
            if i > 0 {
                a = a.add_precondition(&format!("s{}", i - 1), true);
            }
            a = a.add_effect(&format!("s{i}"), true);
            actions.push(a);
        }

        let goal_key = format!("s{}", N - 1);
        let plan = GoapPlanner::plan(&state(&[]), &actions, &single_goal(&[(&goal_key, true)]))
            .expect("the chain is satisfiable");

        let names: Vec<String> = plan.iter().map(|a| a.name.clone()).collect();
        let expected: Vec<String> = (0..N).map(|i| format!("step{i}")).collect();
        assert_eq!(names, expected, "every ancestor must appear exactly once, in order");
    }

    #[test]
    fn higher_priority_achievable_goal_wins() {
        let cur = state(&[("x", false), ("y", false)]);
        let actions = vec![
            GoapAction::new("do_x", 1.0).add_effect("x", true),
            GoapAction::new("do_y", 1.0).add_effect("y", true),
        ];
        let goals = vec![
            GoapGoal::new("low", 1.0).add_desired_state("x", true),
            GoapGoal::new("high", 9.0).add_desired_state("y", true),
        ];
        let plan = GoapPlanner::plan(&cur, &actions, &goals).expect("plan");
        assert_eq!(plan[0].name, "do_y", "highest-priority goal must be planned first");
    }
}
