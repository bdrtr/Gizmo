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
