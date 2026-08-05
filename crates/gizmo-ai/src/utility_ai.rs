//! Utility AI Sistemi
//!
//! AAA kalitesinde esnek karar verme sistemi. Ajanın durumunu analiz eder ve
//! çeşitli eylemlerin faydasını (utility) matematiksel eğriler (curves) ile hesaplayarak
//! en yüksek skora sahip eylemi seçer.

use std::sync::Arc;

/// Değerlendirme fonksiyonu tipi (Örn: Ajanın canını 0.0 - 1.0 aralığına normalize eder)
pub type ContextScorer<T> = Arc<dyn Fn(&T) -> f32 + Send + Sync>;

/// Eğri değerlendirme arayüzü (Normalize edilmiş 0-1 değerini, 0-1 arası fayda skoruna dönüştürür)
///
/// This trait is a deliberate **extension point**: users are expected to
/// implement their own response curves in addition to the built-in
/// [`LinearCurve`] and [`LogisticCurve`]. It is therefore intentionally
/// **not** sealed.
pub trait UtilityCurve: Send + Sync {
    /// Maps a normalized input `x` (0..=1) to a utility score (0..=1).
    fn evaluate(&self, x: f32) -> f32;
}

/// Basit Doğrusal Eğri (y = m*x + b)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearCurve {
    /// Slope, in utility per unit of normalized input. A negative slope inverts
    /// the response, so the lowest input scores highest; `0.0` makes the curve
    /// constant at `b`.
    pub m: f32,
    /// Utility at `x == 0`, before clamping. Together with `m` it decides where
    /// the line leaves the 0..=1 band and the output saturates.
    pub b: f32,
}

impl LinearCurve {
    /// Builds the line `y = m * x + b`. `m = 1.0, b = 0.0` is the identity
    /// response.
    ///
    /// Neither argument is validated: any slope/intercept is accepted, and
    /// out-of-range results are folded back by the clamp in this type's
    /// [`UtilityCurve::evaluate`]. That clamp applies to the *output* only —
    /// the input `x` is passed through unclamped, so feeding this curve
    /// directly (rather than through [`UtilityConsideration::score`], which
    /// clamps first) extrapolates the line beyond 0..=1.
    pub fn new(m: f32, b: f32) -> Self {
        Self { m, b }
    }
}

impl UtilityCurve for LinearCurve {
    fn evaluate(&self, x: f32) -> f32 {
        (self.m * x + self.b).clamp(0.0, 1.0)
    }
}

/// Lojistik (Sigmoid) Eğri — S şeklinde geçişler için (örn: can %50'nin altına inince aciliyetin hızla artması)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogisticCurve {
    /// Steepness of the transition, per unit of normalized input. Positive
    /// values rise with `x`, negative values fall, and `0.0` degenerates the
    /// curve to a constant `0.5` everywhere.
    ///
    /// Magnitude sets how abrupt the S is: with f32 arithmetic the upper tail
    /// saturates to exactly `1.0` once `k * (x - c)` exceeds roughly `17`, so
    /// with `c` mid-domain an `|k|` beyond ~35 acts as a step function.
    pub k: f32, // Eğimi (dikliği) belirler
    /// Midpoint on the `x` axis: `evaluate(c)` is exactly `0.5` for any `k`.
    /// Keep it inside 0..=1 for the transition to fall within the normalized
    /// input domain; outside that range only one tail of the S is visible.
    pub c: f32, // Orta noktayı (x eksenindeki kayma) belirler
}

impl LogisticCurve {
    /// Builds `y = 1 / (1 + exp(-k * (x - c)))`.
    ///
    /// Arguments are not validated. Note the lower tail approaches zero without
    /// reaching it — in f32 it stays strictly positive until `k * (x - c)` is
    /// around `-89`, where `exp` overflows. A consideration built on this curve
    /// therefore *starves* an action (drives its score toward zero) instead of
    /// vetoing it the way an exact `0.0` would in
    /// [`UtilityAction::evaluate`].
    pub fn new(k: f32, c: f32) -> Self {
        Self { k, c }
    }
}

impl UtilityCurve for LogisticCurve {
    fn evaluate(&self, x: f32) -> f32 {
        let val = 1.0 / (1.0 + (-self.k * (x - self.c)).exp());
        val.clamp(0.0, 1.0)
    }
}

/// Bir aksiyonun skorlanmasında kullanılan bir girdi faktörü
pub struct UtilityConsideration<T> {
    /// Reads one fact out of the context and normalizes it to 0..=1 (e.g.
    /// `hp / max_hp`, `1 - distance / range`). Returns outside that range are
    /// clamped by [`score`](Self::score) before the curve sees them, so the
    /// closure may be sloppy at the edges but must not return NaN — NaN is not
    /// clamped and propagates all the way into the action score.
    ///
    /// Held in an `Arc` so one closure can back several considerations and be
    /// cloned cheaply.
    pub scorer: ContextScorer<T>,
    /// Response curve mapping the normalized reading to a utility in 0..=1 —
    /// this is what turns "hp is at 30%" into "healing is worth 0.9".
    pub curve: Box<dyn UtilityCurve>,
    /// Multiplier applied after the curve, scaling a 0..=1 curve output into
    /// 0..=`weight`. It is a raw factor, not a share: weights across the
    /// considerations of one action are never renormalized to sum to 1.
    ///
    /// A weight of `0.0` (or negative) is **not** a way to switch a
    /// consideration off — it forces [`score`](Self::score) to `<= 0.0`, which
    /// [`UtilityAction::evaluate`] treats as a veto that zeroes the whole
    /// action. Drop the consideration from the action instead.
    pub weight: f32,
}

impl<T> UtilityConsideration<T> {
    /// Assembles the scorer → curve → weight chain evaluated by
    /// [`score`](Self::score). Nothing is validated here; see the field docs for
    /// the ranges each part is expected to honour.
    pub fn new(scorer: ContextScorer<T>, curve: Box<dyn UtilityCurve>, weight: f32) -> Self {
        Self {
            scorer,
            curve,
            weight,
        }
    }

    /// Evaluates this consideration: `curve(clamp(scorer(context), 0, 1)) * weight`.
    ///
    /// The result lies in 0..=`weight` as long as the curve honours the
    /// [`UtilityCurve`] contract; a custom curve that returns out-of-range
    /// values is not policed here. A result of `0.0` or less vetoes the owning
    /// action in [`UtilityAction::evaluate`].
    ///
    /// The scorer is invoked on every call — this method does no caching, so a
    /// heavyweight scorer is re-run once per action evaluation.
    pub fn score(&self, context: &T) -> f32 {
        let raw_val = (self.scorer)(context).clamp(0.0, 1.0);
        self.curve.evaluate(raw_val) * self.weight
    }
}

/// Ajanın seçebileceği bir eylem ve onun skorlama kuralları
pub struct UtilityAction<T> {
    /// Free-form label handed back by [`UtilityBrain::decide`]; the caller
    /// dispatches on it. Uniqueness is not enforced, and a duplicate name makes
    /// the winner indistinguishable from its namesake, since the decision
    /// carries nothing else.
    pub name: String,
    /// Factors multiplied into the score, evaluated in insertion order.
    /// Evaluation short-circuits on the first veto, so scorers later in the
    /// vector may not run at all — do not rely on them for side effects.
    ///
    /// While this is empty the action scores exactly `base_score`.
    pub considerations: Vec<UtilityConsideration<T>>,
    /// Starting value of the multiplicative chain, i.e. the action's standing
    /// priority before context is taken into account. Expected in 0..=1: as
    /// soon as one consideration is present the product is clamped into 0..=1,
    /// so a larger `base_score` buys no extra headroom.
    ///
    /// `0.0` makes the action permanently unselectable: the product stays zero,
    /// and [`UtilityBrain::decide`] only ever returns a strictly positive score.
    pub base_score: f32,
}

impl<T> UtilityAction<T> {
    /// Creates an action with no considerations, which means it scores a flat
    /// `base_score` until you attach some with
    /// [`add_consideration`](Self::add_consideration).
    ///
    /// `name` is copied into an owned `String`.
    pub fn new(name: &str, base_score: f32) -> Self {
        Self {
            name: name.to_string(),
            considerations: Vec::new(),
            base_score,
        }
    }

    /// Appends a consideration and returns the action, for builder-style
    /// chaining.
    ///
    /// Order is preserved and does matter: it fixes both the short-circuit
    /// order of the veto check in [`evaluate`](Self::evaluate) and the order the
    /// f32 factors are multiplied in, so reordering the same set of
    /// considerations can move the score by an ulp. There is no removal API;
    /// edit the [`considerations`](Self::considerations) vector directly.
    pub fn add_consideration(mut self, consideration: UtilityConsideration<T>) -> Self {
        self.considerations.push(consideration);
        self
    }

    /// Eylemin toplam fayda skorunu hesaplar (Çarpımsal - biri 0 ise tüm eylem 0 olur)
    ///
    /// Computes the action's total utility as a product: `base_score` times one
    /// factor per consideration, taken in insertion order.
    ///
    /// Semantics worth knowing:
    ///
    /// - **Veto.** The first consideration scoring `<= 0.0` returns `0.0`
    ///   immediately; the remaining scorers are never called.
    /// - **Compensation.** A plain product punishes actions merely for having
    ///   many considerations, so each factor is inflated by the classic
    ///   "make-up value" term: `s + (1 - s) * s * (1 - 1/n)`, with `n` the
    ///   number of considerations. With `n == 1` this is exactly `s`.
    /// - **Empty.** With no considerations the raw `base_score` is returned
    ///   *unclamped* — a `base_score` above `1.0` or below `0.0` escapes here,
    ///   whereas every other path clamps into 0..=1.
    /// - **NaN.** A NaN factor neither trips the veto nor is clamped away; it
    ///   poisons the product, and the action then loses every comparison in
    ///   [`UtilityBrain::decide`], so it can never be selected.
    pub fn evaluate(&self, context: &T) -> f32 {
        if self.considerations.is_empty() {
            return self.base_score;
        }

        // Çarpımsal skorlama sistemi (compensation factor ile)
        let mut final_score = self.base_score;
        let comp_factor = 1.0 - (1.0 / self.considerations.len() as f32);

        for cons in &self.considerations {
            let score = cons.score(context);
            if score <= 0.0 {
                return 0.0; // Veto (Eylem kesinlikle yapılamaz)
            }

            // "Make up" compensation — Çok fazla consideration olan eylemlerin skorunun düşmesini engeller
            let modification = (1.0 - score) * comp_factor;
            final_score *= score + (modification * score);
        }

        final_score.clamp(0.0, 1.0)
    }
}

/// Ajanın eylemleri seçmesini yöneten ana karar verici
pub struct UtilityBrain<T> {
    /// Candidate actions, all of them scored on every [`decide`] call — there is
    /// no pruning, so cost grows linearly with the number of actions times their
    /// considerations.
    ///
    /// Insertion order is the tie-break: on equal scores the earliest action
    /// wins.
    ///
    /// [`decide`]: UtilityBrain::decide
    pub actions: Vec<UtilityAction<T>>,
}

impl<T> Default for UtilityBrain<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> UtilityBrain<T> {
    /// Creates a brain with no actions; [`decide`](Self::decide) returns `None`
    /// until at least one is added.
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    /// Appends an action and returns the brain, for builder-style chaining.
    ///
    /// Earlier actions win ties, so insert the ones that should be preferred at
    /// equal utility first.
    pub fn add_action(mut self, action: UtilityAction<T>) -> Self {
        self.actions.push(action);
        self
    }

    /// Bağlama (context) göre en yüksek skora sahip eylemin adını döner
    ///
    /// Scores every action against `context` and returns the winner's name and
    /// score. The brain is stateless: nothing is remembered between calls, so
    /// there is no hysteresis and an oscillating context produces an
    /// oscillating decision.
    ///
    /// Returns `None` when no action scores **strictly greater than `0.0`** —
    /// an empty brain, every action vetoed, or every action at exactly zero all
    /// look the same to the caller. A tie is resolved toward the earliest
    /// action in [`actions`](Self::actions).
    ///
    /// The returned name is a fresh allocation cloned out of the winning
    /// action, so the result borrows nothing from the brain.
    pub fn decide(&self, context: &T) -> Option<(String, f32)> {
        let mut best_action = None;
        let mut best_score = 0.0;

        for action in &self.actions {
            let score = action.evaluate(context);
            if score > best_score {
                best_score = score;
                best_action = Some(action.name.clone());
            }
        }

        // Eylem/hedef seçimi — hangi eylem hangi skorla kazandı.
        match &best_action {
            Some(name) => tracing::debug!(
                action = %name,
                score = best_score,
                action_count = self.actions.len(),
                "[AI] Utility brain eylem seçti"
            ),
            None => tracing::trace!(
                action_count = self.actions.len(),
                "[AI] Utility brain seçilebilir eylem bulamadı (hepsi veto/0 skor?)"
            ),
        }

        best_action.map(|name| (name, best_score))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_brain_decides_nothing() {
        let brain: UtilityBrain<f32> = UtilityBrain::new();
        assert!(brain.decide(&1.0).is_none());
    }

    #[test]
    fn decide_picks_the_highest_scoring_action() {
        // No considerations → an action evaluates to its (clamped) base_score.
        let brain = UtilityBrain::new()
            .add_action(UtilityAction::<f32>::new("low", 0.3))
            .add_action(UtilityAction::<f32>::new("high", 0.8));
        let (name, score) = brain.decide(&0.0).expect("a decision");
        assert_eq!(name, "high");
        assert!((score - 0.8).abs() < 1e-6, "score {score}");
    }

    #[test]
    fn a_vetoing_consideration_zeroes_the_action() {
        // A consideration that scores 0 vetoes the whole (multiplicative) action,
        // so a lone vetoed action leaves the brain with nothing to pick.
        let scorer: ContextScorer<f32> = Arc::new(|_: &f32| 0.0);
        let cons = UtilityConsideration::new(scorer, Box::new(LinearCurve::new(1.0, 0.0)), 1.0);
        let action = UtilityAction::<f32>::new("vetoed", 1.0).add_consideration(cons);
        assert_eq!(action.evaluate(&0.0), 0.0, "veto must zero the action score");
        let brain = UtilityBrain::new().add_action(action);
        assert!(brain.decide(&0.0).is_none(), "a fully-vetoed action must not be chosen");
    }
}
