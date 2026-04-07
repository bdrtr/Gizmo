use gizmo::prelude::*;
use crate::GameState;

pub fn transform_hierarchy_system(world: &mut World) {
    // 1. Önce herkesin local matrix'ini güncelle (PARALEL!)
    if let Some(mut transforms) = world.borrow_mut::<Transform>() {
        use rayon::prelude::*;
        transforms.dense.par_iter_mut().for_each(|t| {
            t.update_local_matrix();
        });
    }

    // 2. ROOT (Kök) Objelerini bul (Üstünde Parent olmayanlar)
    let mut to_update = Vec::new();
    if let Some(transforms) = world.borrow::<Transform>() {
        let parents = world.borrow::<gizmo::core::component::Parent>();
        for &entity_id in &transforms.entity_dense {
            let has_parent = if let Some(p) = &parents { p.contains(entity_id) } else { false };
            if !has_parent {
                to_update.push((entity_id, Mat4::IDENTITY));
            }
        }
    }

    // 3. BFS ile ağacı aşağıya doğru düzleştirerek Global Matrix hesapla
    let mut head = 0;
    while head < to_update.len() {
        let (entity_id, parent_global) = to_update[head];
        head += 1;

        let mut current_global = Mat4::IDENTITY;
        
        // Bu child'ın global_matrix hesaplaması: Parent Global * Local
        if let Some(mut transforms) = world.borrow_mut::<Transform>() {
            if let Some(t) = transforms.get_mut(entity_id) {
                t.global_matrix = parent_global * t.local_matrix();
                current_global = t.global_matrix;
            }
        }

        // Child node'ları kuyruğa ekle
        if let Some(children_comp) = world.borrow::<gizmo::core::component::Children>() {
            if let Some(children) = children_comp.get(entity_id) {
                for &child_id in &children.0 {
                    to_update.push((child_id, current_global));
                }
            }
        }
    }
}


pub fn particle_update_system(world: &mut World, dt: f32) {
    if dt <= 0.0 { return; }
    
    let mut emitters = match world.borrow_mut::<gizmo::renderer::components::ParticleEmitter>() {
        Some(e) => e,
        None => return,
    };
    
    let transforms = match world.borrow::<gizmo::physics::components::Transform>() {
        Some(t) => t,
        None => return,
    };

    use rand::Rng;
    let mut rng = rand::rng();

    for entity in world.iter_alive_entities() {
        let e_id = entity.id();
        
        if let Some(emitter) = emitters.get_mut(e_id) {
            let base_pos = if let Some(t) = transforms.get(e_id) {
                t.position + t.rotation.mul_vec3(emitter.local_offset)
            } else {
                emitter.local_offset
            };
            
            // 1. Spawning
            if emitter.is_active && emitter.spawn_rate > 0.0 {
                emitter.accumulator += dt;
                let spawn_interval = 1.0 / emitter.spawn_rate;
                
                while emitter.accumulator >= spawn_interval {
                    emitter.accumulator -= spawn_interval;
                    
                    let rand_v_x = rng.random_range(-1.0..=1.0) * emitter.velocity_randomness;
                    let rand_v_y = rng.random_range(-1.0..=1.0) * emitter.velocity_randomness;
                    let rand_v_z = rng.random_range(-1.0..=1.0) * emitter.velocity_randomness;
                    
                    let out_dir = Vec3::new(rand_v_x, rand_v_y, rand_v_z);
                    let vel = emitter.initial_velocity + out_dir;
                    
                    let rand_life = rng.random_range(-1.0..=1.0) * emitter.lifespan_randomness;
                    let max_life = (emitter.lifespan + rand_life).max(0.1);
                    
                    emitter.particles.push(gizmo::renderer::components::Particle {
                        position: base_pos,
                        velocity: vel,
                        life: 0.0,
                        max_life,
                        size_start: emitter.size_start,
                        size_end: emitter.size_end,
                        color: Vec4::new(0.8, 0.8, 0.8, 0.5), // Smoke Default
                    });
                }
            } else {
                emitter.accumulator = 0.0;
            }
            
            // 2. Integration / Physics
            let mut i = 0;
            while i < emitter.particles.len() {
                emitter.particles[i].life += dt;
                if emitter.particles[i].life >= emitter.particles[i].max_life {
                    emitter.particles.swap_remove(i);
                } else {
                    let mut p_vel = emitter.particles[i].velocity;
                    p_vel.y -= emitter.global_gravity * dt;
                    let drag = p_vel * emitter.global_drag * dt;
                    p_vel -= drag;
                    
                    emitter.particles[i].velocity = p_vel;
                    emitter.particles[i].position += p_vel * dt;
                    i += 1;
                }
            }
        }
    }
}


pub(crate) fn audio_update_system(world: &mut World, state: &mut GameState) {
    let mut cam_pos = Vec3::ZERO;
    let mut cam_right = Vec3::new(1.0, 0.0, 0.0);
    
    if let (Some(cameras), Some(transforms)) = (world.borrow::<Camera>(), world.borrow::<Transform>()) {
        if let Some(t) = transforms.get(state.player_id) {
            cam_pos = t.position;
        }
        if let Some(cam) = cameras.get(state.player_id) {
            cam_right = cam.get_right();
        }
    }
    
    let left_ear = cam_pos - cam_right * 0.5;
    let right_ear = cam_pos + cam_right * 0.5;
    
    if let Some(ref mut am) = state.audio {
        am.clean_dead_sinks();
        
        let audio_entities = if let Some(audio_sources) = world.borrow::<gizmo::audio::AudioSource>() {
            audio_sources.entity_dense.clone()
        } else { Vec::new() };

        if let (Some(mut audio_sources), Some(transforms)) = (world.borrow_mut::<gizmo::audio::AudioSource>(), world.borrow::<Transform>()) {
            for e in audio_entities {
                if let Some(audio_src) = audio_sources.get_mut(e) {
                    let curr_pos = transforms.get(e).map_or(Vec3::ZERO, |t| t.position);
                    
                    // Henüz ses başlamamışsa (ilk kare veya yeniden tetikleme)
                    if audio_src._internal_sink_id.is_none() {
                        let emitter = [curr_pos.x, curr_pos.y, curr_pos.z];
                        let r_ear = [right_ear.x, right_ear.y, right_ear.z];
                        let l_ear = [left_ear.x, left_ear.y, left_ear.z];
                        
                        let id = if audio_src.is_3d {
                            if audio_src.loop_sound {
                                am.play_3d_looped(&audio_src.sound_name, emitter, r_ear, l_ear)
                            } else {
                                am.play_3d(&audio_src.sound_name, emitter, r_ear, l_ear)
                            }
                        } else {
                            if audio_src.loop_sound {
                                am.play_looped(&audio_src.sound_name)
                            } else {
                                am.play(&audio_src.sound_name)
                            }
                        };
                        audio_src._internal_sink_id = id;
                    }
                    
                    // Aktif sink varsa parametrelerini güncelle
                    if let Some(sid) = audio_src._internal_sink_id {
                        if !am.is_playing(sid, audio_src.is_3d) {
                            // Ses bitti — tek seferlik ise temizle, döngüsel ise yeniden başlat
                            audio_src._internal_sink_id = None; 
                        } else {
                            if audio_src.is_3d {
                                am.update_spatial_sink(sid, [curr_pos.x, curr_pos.y, curr_pos.z], 
                                        [right_ear.x, right_ear.y, right_ear.z], 
                                        [left_ear.x, left_ear.y, left_ear.z]);
                            }
                            am.set_sink_volume(sid, audio_src.volume, audio_src.is_3d);
                            am.set_sink_pitch(sid, audio_src.pitch, audio_src.is_3d);
                        }
                    }
                }
            }
        }
    }
}





pub fn character_update_system(world: &World, input: &Input, _dt: f32) {
    let mut move_dir = Vec3::ZERO;
    // Arrow keys for character motion
    if input.is_key_pressed(KeyCode::ArrowUp as u32) { move_dir.z -= 1.0; }
    if input.is_key_pressed(KeyCode::ArrowDown as u32) { move_dir.z += 1.0; }
    if input.is_key_pressed(KeyCode::ArrowLeft as u32) { move_dir.x -= 1.0; }
    if input.is_key_pressed(KeyCode::ArrowRight as u32) { move_dir.x += 1.0; }
    
    // Normalize directions so diagonal speed isn't faster
    if move_dir.length_squared() > 0.001 {
        move_dir = move_dir.normalize();
    }
    
    if let Some(mut chars) = world.borrow_mut::<gizmo::physics::character::CharacterController>() {
        for &e in &chars.entity_dense.clone() {
            if let Some(cc) = chars.get_mut(e) {
                // Apply input velocity
                cc.desired_velocity = move_dir * 10.0;
                
                // Allow jump if grounded
                if input.is_key_pressed(KeyCode::Space as u32) && cc.is_grounded {
                    cc.jump(5.0); 
                }
            }
        }
    }
}



