//! GOAP (Goal-Oriented Action Planning) Implementasyonu
//!
//! AAA kalitesinde (F.E.A.R benzeri) karar verme mekanizması.
//! AI ajanlarının mevcut duruma göre hedeflerine ulaşmak için
//! aksiyonları dinamik olarak planlamasını sağlar.

use std::collections::{BinaryHeap, HashMap, HashSet};

/// Dünya durumunu temsil eder. Basitlik için string key ve boolean value kullanıyoruz.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoapState {
    pub values: HashMap<String, bool>,
}

impl GoapState {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: bool) {
        self.values.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> Option<bool> {
        self.values.get(key).copied()
    }

    /// Başka bir durumun bu durumu karşılayıp karşılamadığını kontrol eder (Precondition check)
    pub fn meets_conditions(&self, conditions: &HashMap<String, bool>) -> bool {
        for (k, v) in conditions {
            if self.get(k) != Some(*v) {
                return false;
            }
        }
        true
    }

    /// Bir aksiyonun etkilerini bu duruma uygular
    pub fn apply_effects(&mut self, effects: &HashMap<String, bool>) {
        for (k, v) in effects {
            self.set(k, *v);
        }
    }

    /// İki durum arasındaki farklılık sayısını heuristik olarak döner
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

/// Ajanın yapabileceği tek bir aksiyon
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GoapAction {
    pub name: String,
    pub cost: f32,
    pub preconditions: HashMap<String, bool>,
    pub effects: HashMap<String, bool>,
}

impl GoapAction {
    pub fn new(name: &str, cost: f32) -> Self {
        Self {
            name: name.to_string(),
            cost,
            preconditions: HashMap::new(),
            effects: HashMap::new(),
        }
    }

    pub fn add_precondition(mut self, key: &str, value: bool) -> Self {
        self.preconditions.insert(key.to_string(), value);
        self
    }

    pub fn add_effect(mut self, key: &str, value: bool) -> Self {
        self.effects.insert(key.to_string(), value);
        self
    }
}

/// Ajanın ulaşmaya çalıştığı bir hedef
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GoapGoal {
    pub name: String,
    pub priority: f32,
    pub desired_state: HashMap<String, bool>,
}

impl GoapGoal {
    pub fn new(name: &str, priority: f32) -> Self {
        Self {
            name: name.to_string(),
            priority,
            desired_state: HashMap::new(),
        }
    }

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
    parent: Option<Box<PlanNode>>,
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

pub struct GoapPlanner;

impl GoapPlanner {
    /// Mevcut durum ve hedeflere göre en iyi aksiyon planını (sırasını) oluşturur.
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
                return Some(plan);
            }
        }
        None
    }

    fn build_plan(
        start_state: &GoapState,
        actions: &[GoapAction],
        goal_state: &HashMap<String, bool>,
    ) -> Option<Vec<GoapAction>> {
        let mut open_list = BinaryHeap::new();
        // Ziyaret edilen durumların hash'i (state hashable olmalı, basitlik için String representasyonu)
        let mut closed_list = HashSet::new();

        let start_node = PlanNode {
            state: start_state.clone(),
            g_cost: 0.0,
            h_cost: start_state.distance_to(goal_state),
            action: None,
            parent: None,
        };

        open_list.push(start_node);

        while let Some(current) = open_list.pop() {
            // Hedefe ulaşıldı mı?
            if current.state.meets_conditions(goal_state) {
                let mut plan = Vec::new();
                let mut node = &current;
                while let Some(action) = &node.action {
                    plan.push(action.clone());
                    if let Some(parent) = &node.parent {
                        node = parent;
                    } else {
                        break;
                    }
                }
                plan.reverse();
                return Some(plan);
            }

            // Durumu serialize edip set'e ekle (aynı state loop'a girmemek için)
            let mut state_keys: Vec<_> = current.state.values.iter().collect();
            state_keys.sort_by_key(|(k, _)| *k);
            let state_hash = format!("{:?}", state_keys);

            if closed_list.contains(&state_hash) {
                continue;
            }
            closed_list.insert(state_hash);

            // Uygulanabilir aksiyonları bul
            for action in actions {
                if current.state.meets_conditions(&action.preconditions) {
                    let mut new_state = current.state.clone();
                    new_state.apply_effects(&action.effects);

                    let next_node = PlanNode {
                        state: new_state.clone(),
                        g_cost: current.g_cost + action.cost,
                        h_cost: new_state.distance_to(goal_state),
                        action: Some(action.clone()),
                        parent: Some(Box::new(current.clone())),
                    };

                    open_list.push(next_node);
                }
            }
        }

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
