use std::collections::HashMap;

pub(crate) struct UnionFind {
    pub nodes: HashMap<u32, (u32, u8)>, // (parent, rank)
}

impl UnionFind {
    pub(crate) fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Kökü döndürür (iteratif path compression).
    pub(crate) fn find_root(&mut self, i: u32) -> u32 {
        if !self.nodes.contains_key(&i) {
            self.nodes.insert(i, (i, 0));
        }
        
        // 1. Geçiş: kökü bul
        let mut root = i;
        while self.nodes.get(&root).map(|(p, _)| *p).unwrap_or(root) != root {
            root = self.nodes.get(&root).map(|(p, _)| *p).unwrap_or(root);
        }
        
        // 2. Geçiş: path compression
        let mut cur = i;
        while cur != root {
            let next = self.nodes.get(&cur).map(|(p, _)| *p).unwrap_or(cur);
            if let Some((p, _)) = self.nodes.get_mut(&cur) {
                *p = root;
            }
            cur = next;
        }
        root
    }

    /// İki island'ı birleştirir.
    pub(crate) fn union_nodes(&mut self, i: u32, j: u32) {
        let ri = self.find_root(i);
        let rj = self.find_root(j);
        if ri == rj {
            return;
        }
        
        #[cfg(debug_assertions)]
        let rank_i = self.nodes.get(&ri).map(|(_, r)| *r).expect("rank map'te i kökü bulunamadı — root oluşturulmamış");
        #[cfg(debug_assertions)]
        let rank_j = self.nodes.get(&rj).map(|(_, r)| *r).expect("rank map'te j kökü bulunamadı — root oluşturulmamış");

        #[cfg(not(debug_assertions))]
        let rank_i = self.nodes.get(&ri).map(|(_, r)| *r).unwrap_or(0);
        #[cfg(not(debug_assertions))]
        let rank_j = self.nodes.get(&rj).map(|(_, r)| *r).unwrap_or(0);
        
        match rank_i.cmp(&rank_j) {
            std::cmp::Ordering::Less => {
                if let Some((p, _)) = self.nodes.get_mut(&ri) { *p = rj; }
            }
            std::cmp::Ordering::Greater => {
                if let Some((p, _)) = self.nodes.get_mut(&rj) { *p = ri; }
            }
            std::cmp::Ordering::Equal => {
                if let Some((p, _)) = self.nodes.get_mut(&rj) { *p = ri; }
                if let Some((_, r)) = self.nodes.get_mut(&ri) { *r = r.saturating_add(1); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_union_find_symmetry() {
        let mut uf1 = UnionFind::new();
        uf1.union_nodes(1, 2);
        uf1.union_nodes(2, 3);
        let root1 = uf1.find_root(1);
        let root3_1 = uf1.find_root(3);
        assert_eq!(root1, root3_1);

        let mut uf2 = UnionFind::new();
        // Aynı işlemleri tam ters yönde yapılandıralım
        uf2.union_nodes(2, 1);
        uf2.union_nodes(3, 2);
        let root_inv1 = uf2.find_root(1);
        let root_inv3 = uf2.find_root(3);
        
        assert_eq!(root_inv1, root_inv3);
        // Farklı sıralarla bağlansa bile ortak kökün kendilerine ait ağaç yapısında tutarlı kalması
    }

    #[test]
    fn test_path_compression() {
        let mut uf = UnionFind::new();
        // Uzun bir zincir
        uf.union_nodes(1, 2);
        uf.union_nodes(2, 3);
        uf.union_nodes(3, 4);
        uf.union_nodes(4, 5);

        // find_root çağırmadan önce 5, muhtemelen 4'ü veya 1'i point eder
        let root5 = uf.find_root(5);
        let root1 = uf.find_root(1);
        assert_eq!(root5, root1);

        // İkinci çağrıda doğrudan root'a point etmeli (Path compression çalıştı)
        let root5_direct = uf.find_root(5);
        assert_eq!(root5_direct, root1);
    }
}
