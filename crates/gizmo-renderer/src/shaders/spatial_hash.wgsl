// ---- Helpers ----
fn get_cell_coord(pos: vec3<f32>) -> vec3<i32> {
    let local_pos = pos - params.bounds_min;
    return vec3<i32>(
        i32(local_pos.x / params.cell_size),
        i32(local_pos.y / params.cell_size),
        i32(local_pos.z / params.cell_size)
    );
}

fn get_cell_index(coord: vec3<i32>) -> i32 {
    if (coord.x < 0 || coord.y < 0 || coord.z < 0) { return -1; }
    if (coord.x >= i32(params.grid_size_x) || coord.y >= i32(params.grid_size_y) || coord.z >= i32(params.grid_size_z)) { return -1; }
    return coord.x + coord.y * i32(params.grid_size_x) + coord.z * i32(params.grid_size_x * params.grid_size_y);
}

// ---- PASS 0: CLEAR GRID ---- //
@compute @workgroup_size(64)
fn clear_grid(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    let total_cells = params.grid_size_x * params.grid_size_y * params.grid_size_z;
    if (index >= total_cells) { return; }
    
    grid[index * 2u] = 0xFFFFFFFFu; // start_index
    grid[index * 2u + 1u] = 0xFFFFFFFFu; // end_index
}

// ---- PASS 1: HASH PARTICLES ---- //
@compute @workgroup_size(64)
fn hash_pass(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) {
        // MUST set padding elements to max hash so bitonic sort pushes them to the end
        sort_buffer[index].hash = 0xFFFFFFFFu;
        sort_buffer[index].index = index;
        return;
    }
    
    let pos = particles[index].predicted_position; // PBF uses predicted position
    let cell_coord = get_cell_coord(pos);
    let cell_idx = get_cell_index(cell_coord);
    
    var hash: u32 = 0xFFFFFFFFu;
    if (cell_idx != -1) {
        hash = u32(cell_idx);
    }
    
    sort_buffer[index].hash = hash;
    sort_buffer[index].index = index;
}

// ---- PASS 2: BITONIC SORT ---- //
@compute @workgroup_size(64)
fn bitonic_sort_pass(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;
    let j = sort_params.j;
    let k = sort_params.k;
    
    let ixj = i ^ j;
    if (ixj > i) {
        let p1 = sort_buffer[i];
        let p2 = sort_buffer[ixj];
        
        let up = (i & k) == 0u;
        
        if ((up && p1.hash > p2.hash) || (!up && p1.hash < p2.hash)) {
            sort_buffer[i] = p2;
            sort_buffer[ixj] = p1;
        }
    }
}

// ---- PASS 3: GRID OFFSETS ---- //
@compute @workgroup_size(64)
fn grid_offsets_pass(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let hash = sort_buffer[index].hash;
    if (hash == 0xFFFFFFFFu) { return; }
    
    let prev_hash = select(0xFFFFFFFFu, sort_buffer[index - 1u].hash, index > 0u);
    if (hash != prev_hash) {
        grid[hash * 2u] = index; // start_index
    }
    
    let next_hash = select(0xFFFFFFFFu, sort_buffer[index + 1u].hash, index < params.num_particles - 1u);
    if (hash != next_hash) {
        grid[hash * 2u + 1u] = index; // end_index
    }
}
