use super::World;

impl World {
    /// Belirli bir Archetype içindeki iki satırı güvenli bir şekilde takaslar ve entity lokasyonlarını günceller.
    pub fn swap_archetype_rows(&mut self, arch_id: u32, row_a: usize, row_b: usize) {
        if row_a == row_b {
            return;
        }

        let arch = &self.archetype_index.archetypes[arch_id as usize];
        if row_a >= arch.len() || row_b >= arch.len() {
            return;
        }

        let entity_a = arch.entities()[row_a];
        let entity_b = arch.entities()[row_b];

        unsafe {
            let mut_arch = &mut self.archetype_index.archetypes[arch_id as usize];
            mut_arch.swap_rows(row_a, row_b);
        }

        self.entity_locations[entity_a as usize].row = row_b as u32;
        self.entity_locations[entity_b as usize].row = row_a as u32;
    }

    /// Aynı archetype'da bulunan ebeveyn ve çocuk düğümleri bellekte sırt sırta verecek şekilde kümelendirir. O(N) cache swap.
    pub fn sort_archetype_hierarchy(&mut self) {
        let type_id = std::any::TypeId::of::<crate::component::Children>();
        let mut arches_to_sort: Vec<usize> = Vec::new();

        for (idx, arch) in self.archetype_index.archetypes.iter().enumerate() {
            if arch.has_component(type_id) {
                arches_to_sort.push(idx);
            }
        }

        for arch_idx in arches_to_sort {
            let arch_len = self.archetype_index.archetypes[arch_idx].len();
            if arch_len <= 1 {
                continue;
            }

            let mut visited = std::collections::HashSet::new();

            for row in 0..arch_len {
                let parent_entity_id = self.archetype_index.archetypes[arch_idx].entities()[row];

                if visited.contains(&parent_entity_id) {
                    continue;
                }
                visited.insert(parent_entity_id);

                let children_opt = {
                    let fetch = unsafe {
                        <&crate::component::Children as crate::query::FetchComponent>::fetch_raw(self, &self.archetype_index.archetypes[arch_idx], self.tick)
                    };
                    fetch.map(|f| unsafe {
                        <&crate::component::Children as crate::query::FetchComponent>::get_item(f, row, parent_entity_id)
                    })
                };

                let children_list = match children_opt {
                    Some(c) => c.0.clone(),
                    None => continue,
                };

                let mut current_insert_row = row + 1;
                for child_id in children_list {
                    let loc = self.entity_location(child_id);
                    if loc.is_valid() && loc.archetype_id == arch_idx as u32 {
                        let child_row = loc.row as usize;
                        if child_row > current_insert_row {
                            self.swap_archetype_rows(
                                arch_idx as u32,
                                current_insert_row,
                                child_row,
                            );
                            visited.insert(child_id);
                            current_insert_row += 1;
                        } else if child_row == current_insert_row {
                            visited.insert(child_id);
                            current_insert_row += 1;
                        }
                    }
                }
            }
        }
    }
}
