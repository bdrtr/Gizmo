//! Utility AI Sistemi
//!
//! AAA kalitesinde esnek karar verme sistemi. Ajanın durumunu analiz eder ve 
//! çeşitli eylemlerin faydasını (utility) matematiksel eğriler (curves) ile hesaplayarak 
//! en yüksek skora sahip eylemi seçer.

use std::sync::Arc;

/// Değerlendirme fonksiyonu tipi (Örn: Ajanın canını 0.0 - 1.0 aralığına normalize eder)
pub type ContextScorer<T> = Arc<dyn Fn(&T) -> f32 + Send + Sync>;

/// Eğri değerlendirme arayüzü (Normalize edilmiş 0-1 değerini, 0-1 arası fayda skoruna dönüştürür)
pub trait UtilityCurve: Send + Sync {
    fn evaluate(&self, x: f32) -> f32;
}

/// Basit Doğrusal Eğri (y = m*x + b)
pub struct LinearCurve {
    pub m: f32,
    pub b: f32,
}

impl LinearCurve {
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
pub struct LogisticCurve {
    pub k: f32, // Eğimi (dikliği) belirler
    pub c: f32, // Orta noktayı (x eksenindeki kayma) belirler
}

impl LogisticCurve {
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
    pub scorer: ContextScorer<T>,
    pub curve: Box<dyn UtilityCurve>,
    pub weight: f32,
}

impl<T> UtilityConsideration<T> {
    pub fn new(scorer: ContextScorer<T>, curve: Box<dyn UtilityCurve>, weight: f32) -> Self {
        Self { scorer, curve, weight }
    }

    pub fn score(&self, context: &T) -> f32 {
        let raw_val = (self.scorer)(context).clamp(0.0, 1.0);
        self.curve.evaluate(raw_val) * self.weight
    }
}

/// Ajanın seçebileceği bir eylem ve onun skorlama kuralları
pub struct UtilityAction<T> {
    pub name: String,
    pub considerations: Vec<UtilityConsideration<T>>,
    pub base_score: f32,
}

impl<T> UtilityAction<T> {
    pub fn new(name: &str, base_score: f32) -> Self {
        Self {
            name: name.to_string(),
            considerations: Vec::new(),
            base_score,
        }
    }

    pub fn add_consideration(mut self, consideration: UtilityConsideration<T>) -> Self {
        self.considerations.push(consideration);
        self
    }

    /// Eylemin toplam fayda skorunu hesaplar (Çarpımsal - biri 0 ise tüm eylem 0 olur)
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
    pub actions: Vec<UtilityAction<T>>,
}

impl<T> UtilityBrain<T> {
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    pub fn add_action(mut self, action: UtilityAction<T>) -> Self {
        self.actions.push(action);
        self
    }

    /// Bağlama (context) göre en yüksek skora sahip eylemin adını döner
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

        best_action.map(|name| (name, best_score))
    }
}
