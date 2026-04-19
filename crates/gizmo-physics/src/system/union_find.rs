use std::collections::HashMap;

/// Node'u haritaya ekler (idempotent — zaten varsa değişmez).
pub fn ensure_node(parent: &mut HashMap<u32, u32>, rank: &mut HashMap<u32, u8>, i: u32) {
    parent.entry(i).or_insert(i);
    rank.entry(i).or_insert(0);
}

/// Kökü döndürür — **tam path compression** (iteratif, ek `Vec` yok).
///
/// 1. Geçiş: `i`'den üst zinciri izleyerek kökü bul.
/// 2. Geçiş: aynı zinciri yeniden yürüyüp her düğümün `parent`'ını doğrudan köke yaz.
///
/// Böylece uzun constraint zincirlerinde tekrarlı `find_root` ortalama ~α(N) kalır.
pub fn find_root(parent: &mut HashMap<u32, u32>, i: u32) -> u32 {
    let root_val = *parent.entry(i).or_insert(i);
    if root_val == i {
        return i;
    }
    let root = find_root(parent, root_val);
    parent.insert(i, root);
    root
}

/// İki island'ı birleştirir; rank'ı düşük olan, yüksek olanın altına girer.
pub fn union_nodes(
    parent: &mut HashMap<u32, u32>,
    rank: &mut HashMap<u32, u8>,
    i: u32,
    j: u32,
) {
    let ri = find_root(parent, i);
    let rj = find_root(parent, j);
    if ri == rj {
        return;
    }
    let rank_i = *rank.get(&ri).unwrap_or(&0);
    let rank_j = *rank.get(&rj).unwrap_or(&0);
    match rank_i.cmp(&rank_j) {
        std::cmp::Ordering::Less => {
            parent.insert(ri, rj);
        }
        std::cmp::Ordering::Greater => {
            parent.insert(rj, ri);
        }
        std::cmp::Ordering::Equal => {
            parent.insert(rj, ri);
            *rank.entry(ri).or_insert(0) += 1;
        }
    }
}
