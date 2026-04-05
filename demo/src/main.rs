use yelbegen::prelude::*;

pub mod scene;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);


// ======================== HİYERARŞİ (SCENE GRAPH) SİSTEMİ ========================

// ======================== HİYERARŞİ (SCENE GRAPH) SİSTEMİ ========================

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
        let parents = world.borrow::<yelbegen::core::component::Parent>();
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
        if let Some(children_comp) = world.borrow::<yelbegen::core::component::Children>() {
            if let Some(children) = children_comp.get(entity_id) {
                for &child_id in &children.0 {
                    to_update.push((child_id, current_global));
                }
            }
        }
    }
}

// ======================== OYUN DURUMU ========================

#[derive(Clone, Copy, PartialEq)]
pub enum DragAxis { X, Y, Z }

#[derive(Clone, Copy, PartialEq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

struct GameState {
    bouncing_box_id: u32,
    player_id: u32,
    skybox_id: u32,
    inspector_selected_entity: Option<u32>,
    #[allow(dead_code)] // AudioManager'ın OutputStream'i canlı tutulmalı (drop edilirse ses durur)
    audio: Option<yelbegen::audio::AudioManager>,
    do_raycast: bool,
    gizmo_x: u32,
    gizmo_y: u32,
    gizmo_z: u32,
    dragging_axis: Option<DragAxis>,
    drag_start_t: f32,
    drag_original_pos: Vec3,
    drag_original_scale: Vec3,
    drag_original_rot: Quat,
    current_fps: f32,
    new_selection_request: std::cell::Cell<Option<u32>>,
    spawn_monkey_requests: std::cell::Cell<u32>,
    spawn_light_requests: std::cell::Cell<u32>,
    texture_load_requests: std::cell::RefCell<Vec<(u32, String)>>,
    asset_manager: std::cell::RefCell<yelbegen::renderer::asset::AssetManager>,
    gizmo_mode: GizmoMode,
    egui_wants_pointer: bool,
    asset_watcher: Option<yelbegen::renderer::hot_reload::AssetWatcher>,
    script_engine: std::cell::RefCell<Option<yelbegen::scripting::ScriptEngine>>,
    physics_accumulator: f32,
}

// ======================== ARABA SİMÜLASYONU ========================
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CarController {
    pub speed: f32,
    pub steering: f32, // direksiyon açısı (radyan)
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Wheel {
    pub is_front: bool,
    pub is_left: bool,
    pub base_rotation: f32, // tekerleğin teker gibi dönme açısı
}

pub fn car_update_system(world: &mut World, input: &Input) {
    let dt = world.get_resource::<Time>().map_or(0.016, |t| t.dt);
    let mut car_transform: Option<(u32, Transform, CarController)> = None;

    // 1. Arabayı (CarController) bul ve klavyeye göre güncelle
    // winit KeyCode değerleri: Up = 81 (ya da winit::keyboard::KeyCode::ArrowUp as u32)
    // Şimdilik winit KeyCode karşılıklarına bakalım veya klavye oklarını hardcode edelim
    // W/S harici i/k veya oklara denk gelir. Biz yelbegen-app üzerinden ArrowUp vs test edebiliriz
    // Standart WASD harflerini direksiyon ve gaza bağlayacağız. Kamera ile çakışacak ama dert değil.
    // Aslında I,J,K,L tuşlarını arabaya verelim!
    // I (73), J (74), K (75), L (76) ASCII winit enum fallback veya standard W=87, A=65...
    // Input modülüne ufak bir ek eklemedikçe winit VirtualKeyCode (KeyI, vs.) bilmek zor ama deneyelim.
    let car_accel = if input.is_key_pressed(KeyCode::KeyI as u32) { 1.0 } else if input.is_key_pressed(KeyCode::KeyK as u32) { -1.0 } else { 0.0 };
    let car_steer = if input.is_key_pressed(KeyCode::KeyJ as u32) { 1.0 } else if input.is_key_pressed(KeyCode::KeyL as u32) { -1.0 } else { 0.0 };

    if let Some(mut q) = world.query_mut_mut::<Transform, CarController>() {
        for (e, t, car) in q.iter_mut() {
            // İvmelenme ve sürtünme
            car.speed += car_accel * 15.0 * dt;
            car.speed *= 0.95; // sürtünme
            
            // Direksiyon
            car.steering += car_steer * 2.0 * dt;
            car.steering *= 0.8; // direksiyon kendini toplasın (yaylanma)
            car.steering = car.steering.clamp(-0.5, 0.5); // max direksiyon açısı limiti
            
            // Arabanın rotasyonunu direksiyona göre güncelle (sadece ilerlerken)
            if car.speed.abs() > 0.1 {
                let turn_factor = car.speed.signum() * car.steering * dt * 2.0;
                t.rotation = t.rotation * Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), turn_factor);
            }
            
            // Arabanın hız vektörünü ileri taşı
            let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, 1.0));
            t.position += forward * car.speed * dt;
            
            car_transform = Some((e, *t, *car));
        }
    }

    // 2. Wheel bileşenli tekerleklerin yerel rotasyonlarını direksiyona ve hıza bağla
    if let Some(mut q) = world.query_mut_mut::<Transform, Wheel>() {
        if let Some((_cid, _ct, car)) = car_transform {
            for (_e, t, wheel) in q.iter_mut() {
                wheel.base_rotation += car.speed * dt * 2.0; // tekerleğin yuvarlanma dönüşü
                
                // Direksiyon açısı (Sadece Y ekseninde ön tekerlekler, yerel eksende)
                let y_rot = if wheel.is_front { car.steering } else { 0.0 };
                
                // Rotasyonu birleştir: Önce direksiyon yönüne dön (Y), sonra teker gibi yuvarlan (X ekseni etrafında)
                t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), y_rot)
                           * Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), wheel.base_rotation);
            }
        }
    }
}

// ======================== GLTF HİYERARŞİ OLUŞTURUCU ========================
pub fn spawn_gltf_hierarchy(
    world: &mut World,
    nodes: &[yelbegen::renderer::GltfNodeData],
    parent_id: Option<u32>,
    default_material: Material,
) -> Vec<yelbegen::core::Entity> {
    let mut spawned_entities = Vec::new();

    for node in nodes {
        let entity = world.spawn();
        let id = entity.id();
        spawned_entities.push(entity);

        let entity_name = node.name.clone().unwrap_or_else(|| "GLTF_Node".to_string());
        world.add_component(entity, EntityName(entity_name.clone()));

        // Transform hesapla
        let t = Transform::new(Vec3::new(node.translation[0], node.translation[1], node.translation[2]))
            .with_rotation(Quat::new(node.rotation[0], node.rotation[1], node.rotation[2], node.rotation[3]))
            .with_scale(Vec3::new(node.scale[0], node.scale[1], node.scale[2]));
        world.add_component(entity, t);

        if let Some(pid) = parent_id {
            world.add_component(entity, Parent(pid));
        }

        let mut immediate_children = Vec::new();

        for (prim_i, (mesh, mat_opt)) in node.primitives.iter().enumerate() {
            let prim_entity = world.spawn();
            world.add_component(prim_entity, Parent(id));
            world.add_component(prim_entity, Transform::new(Vec3::ZERO));
            world.add_component(prim_entity, EntityName(format!("{}_Primitive_{}", entity_name, prim_i)));
            world.add_component(prim_entity, mesh.clone());
            
            if let Some(mat) = mat_opt {
                world.add_component(prim_entity, mat.clone());
            } else {
                world.add_component(prim_entity, default_material.clone()); // Eğer GLTF material okumadıysa default
            }
            world.add_component(prim_entity, yelbegen::renderer::components::MeshRenderer::new());
            immediate_children.push(prim_entity.id());
        }

        // Recursive olarak çocukları in
        if !node.children.is_empty() {
            let child_entities = spawn_gltf_hierarchy(world, &node.children, Some(id), default_material.clone());
            immediate_children.extend(child_entities.iter().map(|e| e.id()));
        }

        if !immediate_children.is_empty() {
            world.add_component(entity, Children(immediate_children));
        }
    }

    spawned_entities
}

// ======================== ANA FONKSİYON ========================

fn main() {
    let mut app = App::new("Yelbegen Engine — Rust 3D Motor", 1280, 720);

    // 1. SETUP
    app = app.set_setup(|world, renderer| {
        println!("Yelbegen Engine: Sahne başlatılıyor...");

        let mut audio = yelbegen::audio::AudioManager::new();
        if let Some(ref mut a) = audio {
            a.load_sound("bounce", "demo/assets/bounce.wav");
        }

        let mut asset_manager = AssetManager::new();

        // Varsayılan Kaplama (Texture)
        let tbind = asset_manager.load_material_texture(
             &renderer.device,
             &renderer.queue,
             &renderer.texture_bind_group_layout,
             "demo/assets/stone_tiles.jpg" // brick.jpg yerine stone_tiles var
        ).expect("Varsayilan texture bulunamadi!");

        let stone_tbind = asset_manager.load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.texture_bind_group_layout,
            "demo/assets/stone_tiles.jpg"
        ).expect("Varsayılan taş texture yüklenemedi!");

        let mut bouncing_box_id = 0;
        let loaded = scene::SceneData::load_into(
            "scene.json",
            world,
            &renderer.device,
            &renderer.queue,
            &renderer.texture_bind_group_layout,
            &mut asset_manager,
            tbind.clone(),
            &mut bouncing_box_id,
        );

        let (player_id, skybox_id, g_x, g_y, g_z) = if !loaded {
            // Sphere Mesh'ini bir kez oluşturup paylaşalım (Instancing için şart!)
            let sphere_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);

            // --- FİZİKLİ ZEMİN OLUŞTURMA ---
            let ground_mesh = AssetManager::create_plane(&renderer.device, 50.0);
            let ground_entity = world.spawn();
            world.add_component(ground_entity, ground_mesh);
            world.add_component(ground_entity, Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(50.0, 1.0, 50.0)));
            world.add_component(ground_entity, Material::new(stone_tbind.clone()).with_pbr(Vec4::new(0.3, 0.3, 0.3, 1.0), 0.9, 0.1));
            world.add_component(ground_entity, yelbegen::renderer::components::MeshRenderer::new());
            world.add_component(ground_entity, yelbegen::physics::components::RigidBody::new_static());
            // Y ekseni 'half_extent'ini (kalınlık) 0.05 yapıyoruz ki dümdüz olan visual Plane ile eşleşsin! (1.0 olunca havada çarpışıyordu)
            world.add_component(ground_entity, yelbegen::physics::shape::Collider::new_aabb(25.0, 0.05, 25.0));

            // --- Fiziksel Araç (Raycast Milk Truck) ---
            let gltf_scene = asset_manager.load_gltf_scene(&renderer.device, &renderer.queue, &renderer.texture_bind_group_layout, tbind.clone(), "demo/assets/truck.glb").unwrap();
            
            // Gerçek fizik arabamızın asıl gövdesi (Matematiksel Dünya Koordinatlarına sahip Master Kök)
            let car_root = world.spawn();
            world.add_component(car_root, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
            world.add_component(car_root, yelbegen::physics::components::Velocity::new(Vec3::ZERO));
            
            // GLTF Hiyerarşisini Çıkar (İlk Node genelde yöneltici Yup2Zup düğümüdür)
            let parent_entities = spawn_gltf_hierarchy(world, &gltf_scene.roots, Some(car_root.id()), Material::new(tbind.clone()));
            world.add_component(car_root, yelbegen::core::component::Children(vec![parent_entities[0].id()]));
            
            let mut rb = yelbegen::physics::components::RigidBody::new(1500.0, 0.1, 0.5, true);
            rb.calculate_box_inertia(2.0, 1.5, 3.0); // Kutu eylemsizliği
            world.add_component(car_root, rb);
            
            // Gövde için kaba çarpışma kutusu
            world.add_component(car_root, yelbegen::physics::shape::Collider::new_aabb(1.2, 0.8, 2.0)); 
            
            // Süspansiyon (Vehicle Controller)
            let mut vehicle = yelbegen::physics::vehicle::VehicleController::new();
            
            let w = 1.0;
            let l_f = 1.43; // GLTF'teki (Node) ön dingil Z konumu
            let l_r = -1.35; // GLTF'teki (Node.001) arka dingil Z konumu
            let h = -0.4; // Kasanın altına

            let rest = 0.5;
            let stiff = 25000.0;
            let damp = 1500.0;
            let rad = 0.42;

            use yelbegen::physics::vehicle::Wheel;
            vehicle.add_wheel(Wheel::new(Vec3::new(w, h, l_f), rest, stiff, damp, rad)); // İleri Sol
            vehicle.add_wheel(Wheel::new(Vec3::new(-w, h, l_f), rest, stiff, damp, rad)); // İleri Sağ
            vehicle.add_wheel(Wheel::new(Vec3::new(w, h, l_r), rest, stiff, damp, rad)); // Geri Sol
            vehicle.add_wheel(Wheel::new(Vec3::new(-w, h, l_r), rest, stiff, damp, rad)); // Geri Sağ
            
            world.add_component(car_root, vehicle);
            world.add_component(car_root, EntityName("Süt Kamyonu (GLTF)".into()));
            
            // --- GÜNEŞ (Directional Light / Gerçek Zamanlı Ana Gölgelendirici) ---
            let sun = world.spawn();
            let sun_transform = Transform::new(Vec3::new(0.0, 50.0, 50.0))
                .with_rotation(Quat::from_axis_angle(Vec3::new(1.0, 0.5, 0.0).normalize(), -std::f32::consts::FRAC_PI_4));
            world.add_component(sun, sun_transform);
            world.add_component(sun, yelbegen::renderer::components::DirectionalLight::new(Vec3::new(1.0, 0.98, 0.9), 1.5, true));
            world.add_component(sun, EntityName("Güneş (Directional)".into()));

            // --- Player (Kamera) ---
            let player = world.spawn();
            world.add_component(player, Transform::new(Vec3::new(0.0, 5.0, 15.0)));
            world.add_component(player, Camera::new(
                std::f32::consts::FRAC_PI_4, 0.1, 2000.0,
                -std::f32::consts::FRAC_PI_2, -0.3, true,
            ));
            world.add_component(player, EntityName("Kamera (Göz)".into()));

            // --- Skybox (Sonsuz Gökyüzü) ---
            let skybox = world.spawn();
            let mut sky_transform = Transform::new(Vec3::ZERO);
            // Devasa boyut
            sky_transform.scale = Vec3::new(500.0, 500.0, 500.0); 
            world.add_component(skybox, sky_transform);
            world.add_component(skybox, AssetManager::create_inverted_cube(&renderer.device));
            world.add_component(skybox, Material::new(tbind.clone()).with_skybox());
            world.add_component(skybox, yelbegen::renderer::components::MeshRenderer::new());
            world.add_component(skybox, EntityName("Skybox (Gök Kubbe)".into()));

            // --- GIZMO EKSENLERI (X, Y, Z) ---
            // Görünmez yapmak için y = -1000'de başlat.
            let x_gizmo = world.spawn();
            world.add_component(x_gizmo, Transform::new(Vec3::new(0.0, -1000.0, 0.0)).with_scale(Vec3::new(1.5, 0.08, 0.08)));
            world.add_component(x_gizmo, AssetManager::create_sphere(&renderer.device, 1.0, 6, 6));
            world.add_component(x_gizmo, Material::new(tbind.clone()).with_unlit(Vec4::new(1.0, 0.0, 0.0, 1.0)));
            world.add_component(x_gizmo, Collider::new_aabb(1.5, 0.3, 0.3));
            world.add_component(x_gizmo, yelbegen::renderer::components::MeshRenderer::new());
            world.add_component(x_gizmo, EntityName("Gizmo_X".into()));

            let y_gizmo = world.spawn();
            world.add_component(y_gizmo, Transform::new(Vec3::new(0.0, -1000.0, 0.0)).with_scale(Vec3::new(0.08, 1.5, 0.08)));
            world.add_component(y_gizmo, AssetManager::create_sphere(&renderer.device, 1.0, 6, 6));
            world.add_component(y_gizmo, Material::new(tbind.clone()).with_unlit(Vec4::new(0.0, 1.0, 0.0, 1.0)));
            world.add_component(y_gizmo, Collider::new_aabb(0.3, 1.5, 0.3));
            world.add_component(y_gizmo, yelbegen::renderer::components::MeshRenderer::new());
            world.add_component(y_gizmo, EntityName("Gizmo_Y".into()));

            let z_gizmo = world.spawn();
            world.add_component(z_gizmo, Transform::new(Vec3::new(0.0, -1000.0, 0.0)).with_scale(Vec3::new(0.08, 0.08, 1.5)));
            world.add_component(z_gizmo, AssetManager::create_sphere(&renderer.device, 1.0, 6, 6));
            world.add_component(z_gizmo, Material::new(tbind.clone()).with_unlit(Vec4::new(0.0, 0.0, 1.0, 1.0)));
            world.add_component(z_gizmo, Collider::new_aabb(0.3, 0.3, 1.5));
            world.add_component(z_gizmo, yelbegen::renderer::components::MeshRenderer::new());
            world.add_component(z_gizmo, EntityName("Gizmo_Z".into()));

            (player.id(), skybox.id(), x_gizmo.id(), y_gizmo.id(), z_gizmo.id())
        } else {
            // ECS'den Player ve Skybox ID'sini çek (Kameralara bakarak vb)
            let mut p_id = 0;
            let mut s_id = 0;
            let mut g_x = 0;
            let mut g_y = 0;
            let mut g_z = 0;
            
            if let Some(names) = world.borrow::<EntityName>() {
                for entity in world.iter_alive_entities() {
                    if let Some(n) = names.get(entity.id()) {
                        if n.0 == "Kamera (Göz)" { p_id = entity.id(); }
                        if n.0 == "Skybox (Gök Kubbe)" { s_id = entity.id(); }
                        if n.0 == "Gizmo_X" { g_x = entity.id(); }
                        if n.0 == "Gizmo_Y" { g_y = entity.id(); }
                        if n.0 == "Gizmo_Z" { g_z = entity.id(); }
                    }
                }
            }
            (p_id, s_id, g_x, g_y, g_z)
        };

        GameState {
            bouncing_box_id,
            player_id: player_id,
            skybox_id: skybox_id,
            inspector_selected_entity: None,
            audio,
            do_raycast: false,
            gizmo_x: g_x,
            gizmo_y: g_y,
            gizmo_z: g_z,
            dragging_axis: None,
            drag_start_t: 0.0,
            drag_original_pos: Vec3::ZERO,
            drag_original_scale: Vec3::ONE,
            drag_original_rot: Quat::IDENTITY,
            current_fps: 60.0,
            new_selection_request: std::cell::Cell::new(None),
            spawn_monkey_requests: std::cell::Cell::new(0),
            spawn_light_requests: std::cell::Cell::new(0),
            texture_load_requests: std::cell::RefCell::new(Vec::new()),
            asset_manager: std::cell::RefCell::new(asset_manager),
            gizmo_mode: GizmoMode::Translate,
            egui_wants_pointer: false,
            asset_watcher: yelbegen::renderer::hot_reload::AssetWatcher::new(&["demo/assets"]),
            script_engine: std::cell::RefCell::new(yelbegen::scripting::ScriptEngine::new().ok()),
            physics_accumulator: 0.0,
        }
    });



    // 3. UPDATE HOOK
    app = app.set_update(|world, state, dt, input| {
        let speed = 10.0 * dt;
        state.current_fps = 1.0 / dt;

        // === HOT-RELOAD: Dosya değişikliklerini kontrol et ===
        if let Some(watcher) = &state.asset_watcher {
            let changes = watcher.poll_changes();
            if !changes.is_empty() {
                // Değişen dosyalar arasında texture olanları bul ve runtime'da yeniden yükle
                for changed_path in &changes {
                    let path_str = changed_path.to_string_lossy().to_string();
                    // Sadece resim dosyalarını yeniden yükle
                    let is_image = path_str.ends_with(".jpg") || path_str.ends_with(".png") || path_str.ends_with(".jpeg");
                    if !is_image { continue; }
                    
                    println!("🔥 Hot-Reload: Texture değişti → {}", path_str);
                    
                    // Bu texture'ı kullanan tüm materyalleri bul ve güncelle
                    // (texture_load_requests mekanizmasını kullanarak)
                    if let Some(materials) = world.borrow::<Material>() {
                        let mut targets = Vec::new();
                        for &entity_id in &materials.entity_dense {
                            if let Some(mat) = materials.get(entity_id) {
                                if let Some(src) = &mat.texture_source {
                                    if changed_path.ends_with(src.as_str()) || src.contains(&path_str) || path_str.contains(src.as_str()) {
                                        targets.push(entity_id);
                                    }
                                }
                            }
                        }
                        drop(materials);
                        
                        // Her hedef entity için texture_load_requests kuyruğuna ekle
                        // (Render hook'ta renderer erişimiyle gerçek yükleme yapılacak)
                        for entity_id in targets {
                            state.texture_load_requests.borrow_mut().push((entity_id, path_str.clone()));
                        }
                    }
                }
            }
        }

        if let Some(new_sel) = state.new_selection_request.get() {
            state.inspector_selected_entity = Some(new_sel);
            state.new_selection_request.set(None);
        }

        let mut current_ray = None;
        let (mx, my) = input.mouse_position();
        let (ww, wh) = input.window_size();
        let ndc_x = (2.0 * mx) / ww - 1.0;
        let ndc_y = 1.0 - (2.0 * my) / wh;

        if input.is_mouse_button_just_pressed(mouse::LEFT) {
            state.do_raycast = true;
        }
        if input.is_mouse_button_just_released(mouse::LEFT) {
            state.dragging_axis = None;
        }

        if input.is_mouse_button_pressed(mouse::RIGHT) {
            if let Some(mut cameras) = world.borrow_mut::<Camera>() {
                if let Some(cam) = cameras.get_mut(state.player_id) {
                    let delta = input.mouse_delta();
                    cam.yaw += delta.0 * 0.002;
                    cam.pitch -= delta.1 * 0.002;
                    cam.pitch = cam.pitch.clamp(-1.5, 1.5);
                }
            }
        }

        if let (Some(cameras), Some(transforms)) = (world.borrow::<Camera>(), world.borrow::<Transform>()) {
            if let (Some(cam), Some(cam_t)) = (cameras.get(state.player_id), transforms.get(state.player_id)) {
                let proj = Mat4::perspective(cam.fov, ww / wh, cam.near, cam.far);
                let view = cam.get_view(cam_t.position);
                let view_proj = proj * view;

                if let Some(inv_vp) = view_proj.inverse() {
                    // wgpu NDC: z aralığı [0, 1] (near=0, far=1)
                    let far_pt = inv_vp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
                    let near_pt = inv_vp * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);

                    let world_near = Vec3::new(near_pt.x / near_pt.w, near_pt.y / near_pt.w, near_pt.z / near_pt.w);
                    let world_far = Vec3::new(far_pt.x / far_pt.w, far_pt.y / far_pt.w, far_pt.z / far_pt.w);

                    let ray_dir = (world_far - world_near).normalize();
                    current_ray = Some(yelbegen::math::Ray::new(world_near, ray_dir));
                }
            }
        }

        if let Some(ray) = current_ray {
            // EGUI paneli üzerindeyken raycast'i yutuyoruz
            if state.egui_wants_pointer {
                state.do_raycast = false;
            }

            if state.do_raycast {
                state.do_raycast = false;
                
                let mut closest_t = std::f32::MAX;
                let mut hit_entity = None;

                if let (Some(colliders), Some(transforms)) = (world.borrow::<Collider>(), world.borrow::<Transform>()) {
                    for i in 0..colliders.dense.len() {
                        let id = colliders.entity_dense[i];
                        // Dahili objeleri (Kamera, Skybox) raycast'ten hariç tut
                        if id == state.player_id || id == state.skybox_id {
                            continue;
                        }
                        if let Some(t) = transforms.get(id) {
                            if let yelbegen::physics::ColliderShape::Aabb(aabb) = &colliders.dense[i].shape {
                                // Scale'i collider boyutuna uygula
                                let scaled_half = Vec3::new(
                                    aabb.half_extents.x * t.scale.x,
                                    aabb.half_extents.y * t.scale.y,
                                    aabb.half_extents.z * t.scale.z,
                                );
                                let min = t.position - scaled_half;
                                let max = t.position + scaled_half;
                                if let Some(hitt) = ray.intersect_aabb(min, max) {
                                    // Sadece bize kameranın önündeki objeleri ver! (Z clip)
                                    if hitt > 0.0 && hitt < closest_t {
                                        closest_t = hitt;
                                        hit_entity = Some(id);
                                    }
                                }
                            }
                        }
                    }

                    if let Some(hit) = hit_entity {
                        if hit == state.gizmo_x || hit == state.gizmo_y || hit == state.gizmo_z {
                            if let Some(sel) = state.inspector_selected_entity {
                                if let Some(t) = transforms.get(sel) {
                                    state.drag_original_pos = t.position;
                                    state.drag_original_scale = t.scale;
                                    state.drag_original_rot = t.rotation;
                                    let axis_dir = if hit == state.gizmo_x { Vec3::new(1.0, 0.0, 0.0) }
                                                else if hit == state.gizmo_y { Vec3::new(0.0, 1.0, 0.0) }
                                                else { Vec3::new(0.0, 0.0, 1.0) };
                                    
                                    if state.gizmo_mode == GizmoMode::Rotate {
                                        // Düzlem kesişimi (Plane Intersection)
                                        let denom = ray.direction.dot(axis_dir);
                                        if denom.abs() > 0.0001 {
                                            let plane_t = (t.position - ray.origin).dot(axis_dir) / denom;
                                            if plane_t >= 0.0 {
                                                let intersection = ray.origin + ray.direction * plane_t;
                                                let local_hit = intersection - t.position;
                                                
                                                // Start Angle hesaplama
                                                // Normal'in dikindeki u ve v vektörlerini bulalım
                                                let u = if axis_dir.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0).cross(axis_dir).normalize() } 
                                                        else { Vec3::new(0.0, 1.0, 0.0).cross(axis_dir).normalize() };
                                                let v = axis_dir.cross(u);
                                                
                                                let start_angle = local_hit.dot(v).atan2(local_hit.dot(u));
                                                state.drag_start_t = start_angle; // Açı olarak tutuyoruz
                                                
                                                if hit == state.gizmo_x { state.dragging_axis = Some(DragAxis::X); }
                                                else if hit == state.gizmo_y { state.dragging_axis = Some(DragAxis::Y); }
                                                else { state.dragging_axis = Some(DragAxis::Z); }
                                            }
                                        }
                                    } else {
                                        // Translate ve Scale için En Yakın Çizgi Kesişimi
                                        let w0 = ray.origin - t.position;
                                        let b = ray.direction.dot(axis_dir);
                                        let d = ray.direction.dot(w0);
                                        let e = axis_dir.dot(w0);
                                        let denom = 1.0 - b * b;
                                        
                                        if denom.abs() > 0.0001 {
                                            state.drag_start_t = (e - b * d) / denom; // Çizgi üzerindeki mesafe (t)
                                            if hit == state.gizmo_x { state.dragging_axis = Some(DragAxis::X); }
                                            else if hit == state.gizmo_y { state.dragging_axis = Some(DragAxis::Y); }
                                            else { state.dragging_axis = Some(DragAxis::Z); }
                                        }
                                    }
                                }
                            }
                        } else {
                            state.inspector_selected_entity = Some(hit);
                            let mut name_str = format!("Model {}", hit);
                            if let Some(names) = world.borrow::<EntityName>() {
                                if let Some(n) = names.get(hit) {
                                    name_str = n.0.clone();
                                }
                            }
                            println!("Raycast: {} seçildi!", name_str);
                        }
                    } else {
                        state.inspector_selected_entity = None;
                    }
                } // End immutable borrow
            } else if let Some(axis) = state.dragging_axis {
                if let Some(sel) = state.inspector_selected_entity {
                    let axis_dir = match axis {
                        DragAxis::X => Vec3::new(1.0, 0.0, 0.0),
                        DragAxis::Y => Vec3::new(0.0, 1.0, 0.0),
                        DragAxis::Z => Vec3::new(0.0, 0.0, 1.0),
                    };

                    if state.gizmo_mode == GizmoMode::Rotate {
                        let denom = ray.direction.dot(axis_dir);
                        if denom.abs() > 0.0001 {
                            let plane_t_val = (state.drag_original_pos - ray.origin).dot(axis_dir) / denom;
                            if plane_t_val >= 0.0 {
                                let intersection = ray.origin + ray.direction * plane_t_val;
                                let local_hit = intersection - state.drag_original_pos;
                                
                                let u = if axis_dir.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0).cross(axis_dir).normalize() } 
                                        else { Vec3::new(0.0, 1.0, 0.0).cross(axis_dir).normalize() };
                                let v = axis_dir.cross(u);
                                
                                let current_angle = local_hit.dot(v).atan2(local_hit.dot(u));
                                let delta_angle = current_angle - state.drag_start_t;
                                
                                let rot_delta = yelbegen::math::Quat::from_axis_angle(axis_dir, delta_angle);
                                if let Some(mut trans) = world.borrow_mut::<Transform>() {
                                    if let Some(t) = trans.get_mut(sel) {
                                        t.rotation = rot_delta * state.drag_original_rot;
                                    }
                                }
                            }
                        }
                    } else {
                        let w0 = ray.origin - state.drag_original_pos;
                        let b = ray.direction.dot(axis_dir);
                        let d = ray.direction.dot(w0);
                        let e = axis_dir.dot(w0);
                        let denom = 1.0 - b * b;
                        
                        if denom.abs() > 0.0001 {
                            let current_t = (e - b * d) / denom;
                            let delta_t = current_t - state.drag_start_t;
                            
                            if let Some(mut trans) = world.borrow_mut::<Transform>() {
                                if let Some(t) = trans.get_mut(sel) {
                                    if state.gizmo_mode == GizmoMode::Translate {
                                        t.position = state.drag_original_pos + axis_dir * delta_t;
                                    } else if state.gizmo_mode == GizmoMode::Scale {
                                        let mut new_scale = state.drag_original_scale + axis_dir * delta_t;
                                        if new_scale.x < 0.01 { new_scale.x = 0.01; }
                                        if new_scale.y < 0.01 { new_scale.y = 0.01; }
                                        if new_scale.z < 0.01 { new_scale.z = 0.01; }
                                        t.scale = new_scale;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut f = Vec3::ZERO;
        let mut r = Vec3::ZERO;

        if let Some(cameras) = world.borrow::<Camera>() {
            if let Some(cam) = cameras.get(state.player_id) {
                f = cam.get_front();
                r = cam.get_right();
            }
        }

        // ECS içine render frame'in anlık DT'sini koyalım (scriptler vs için)
        world.insert_resource(Time { dt, elapsed_seconds: 0.0 });

        // Araba Kontrolcüsü Güncellemesi (Render dt ile frame bağımlı çalışsın diye bırakıldı veya bu da fiziğe alınabilir)
        car_update_system(world, input);

        // --- FIXED TIME STEP (SABİT FİZİK ADIMI) ---
        state.physics_accumulator += dt;
        let fixed_dt: f32 = 1.0 / 60.0; // 60 FPS Fizik adımı (0.016666...)

        // Max 5 step sınırı koyalım (Oyuna çok fazla drop girerse Spiral of Death engellemesi)
        let mut steps = 0;
        while state.physics_accumulator >= fixed_dt && steps < 5 {
            // Fiziğe sabit zamanı simüle ettiğimizi söyleyelim
            // Aslında movement_system içindeki dt'yi world.get_resource::<Time> yerine argüman vs almalıyız
            // Fakat sistem zaten "Time" resource'ını okumuyor! İçinde sabit let dt = 0.016; kullanıyordu. Oraya da `fixed_dt` yollamamız lazım.
            
            // Araç Süspansiyonları (Fizik eylemleri hesaplanmadan hemen önce yay torkunu ekle)
            yelbegen::physics::system::physics_vehicle_system(world, fixed_dt);
            
            // HACK: Simdilik physics_movement_system içindeki sabit dt, fixed_dt ile örtüşmeli
            yelbegen::physics::system::physics_movement_system(world, fixed_dt);
            yelbegen::physics::system::physics_collision_system(world);
            
            // Fizik kısıtlamalarını Çöz
            if let Some(joint_world) = world.get_resource::<yelbegen::physics::JointWorld>() {
                yelbegen::physics::solve_constraints(&*joint_world, world, fixed_dt);
            }
            
            state.physics_accumulator -= fixed_dt;
            steps += 1;
        }

        let target_pos = if let Some(selected) = state.inspector_selected_entity {
            if let Some(trans) = world.borrow::<Transform>() {
                trans.get(selected).map(|t| t.position)
            } else { None }
        } else { None };

        let current_mode = state.gizmo_mode;

        if let Some(mut trans) = world.borrow_mut::<Transform>() {
            // Gizmo Senkronizasyonu
            if let Some(pos) = target_pos {
                if let Some(tx) = trans.get_mut(state.gizmo_x) { 
                    tx.position = pos;
                    tx.scale = match current_mode {
                        GizmoMode::Translate | GizmoMode::Scale => Vec3::new(1.5, 0.08, 0.08),
                        GizmoMode::Rotate => Vec3::new(0.05, 1.5, 1.5),
                    };
                }
                if let Some(ty) = trans.get_mut(state.gizmo_y) { 
                    ty.position = pos;
                    ty.scale = match current_mode {
                        GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.08, 1.5, 0.08),
                        GizmoMode::Rotate => Vec3::new(1.5, 0.05, 1.5),
                    };
                }
                if let Some(tz) = trans.get_mut(state.gizmo_z) { 
                    tz.position = pos;
                    tz.scale = match current_mode {
                        GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.08, 0.08, 1.5),
                        GizmoMode::Rotate => Vec3::new(1.5, 1.5, 0.05),
                    };
                }
            } else {
                if let Some(tx) = trans.get_mut(state.gizmo_x) { tx.position = Vec3::new(0.0, -1000.0, 0.0); }
                if let Some(ty) = trans.get_mut(state.gizmo_y) { ty.position = Vec3::new(0.0, -1000.0, 0.0); }
                if let Some(tz) = trans.get_mut(state.gizmo_z) { tz.position = Vec3::new(0.0, -1000.0, 0.0); }
            }

            if let Some(_pos) = target_pos {
                if let Some(mut colls) = world.borrow_mut::<Collider>() {
                    if let Some(cx) = colls.get_mut(state.gizmo_x) {
                        if let ColliderShape::Aabb(ref mut aabb) = cx.shape {
                            aabb.half_extents = match current_mode {
                                GizmoMode::Translate | GizmoMode::Scale => Vec3::new(1.5, 0.3, 0.3),
                                GizmoMode::Rotate => Vec3::new(0.1, 1.5, 1.5),
                            };
                        }
                    }
                    if let Some(cy) = colls.get_mut(state.gizmo_y) {
                        if let ColliderShape::Aabb(ref mut aabb) = cy.shape {
                            aabb.half_extents = match current_mode {
                                GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.3, 1.5, 0.3),
                                GizmoMode::Rotate => Vec3::new(1.5, 0.1, 1.5),
                            };
                        }
                    }
                    if let Some(cz) = colls.get_mut(state.gizmo_z) {
                        if let ColliderShape::Aabb(ref mut aabb) = cz.shape {
                            aabb.half_extents = match current_mode {
                                GizmoMode::Translate | GizmoMode::Scale => Vec3::new(0.3, 0.3, 1.5),
                                GizmoMode::Rotate => Vec3::new(1.5, 1.5, 0.1),
                            };
                        }
                    }
                }
            }

            // 3.4. Kamera Hareketi Güncellemesi (WASD)
            let mut move_dir = Vec3::ZERO;
            
            if input.is_key_pressed(KeyCode::KeyW as u32) { move_dir.z += 1.0; }
            if input.is_key_pressed(KeyCode::KeyS as u32) { move_dir.z -= 1.0; }
            if input.is_key_pressed(KeyCode::KeyA as u32) { move_dir.x -= 1.0; }
            if input.is_key_pressed(KeyCode::KeyD as u32) { move_dir.x += 1.0; }
            if input.is_key_pressed(KeyCode::Space as u32) { move_dir.y += 1.0; }
            if input.is_key_pressed(KeyCode::ShiftLeft as u32) { move_dir.y -= 1.0; }
            
            if let Some(t) = trans.get_mut(state.player_id) {
                t.position += (f * move_dir.z + r * move_dir.x + Vec3::new(0.0, move_dir.y, 0.0)) * speed * 2.0;
            }

            // Suzanne / Car_Root'ı Y ekseni etrafında YALNIZCA input ile döndürmek lazım.
            // Sürekli dönmesini engellemek için bu satırı kapattık.
            /*if let Some(t) = trans.get_mut(state.bouncing_box_id) {
                let rot_delta = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), dt * 2.0); 
                t.rotation = (t.rotation * rot_delta).normalize();
            }*/
        }

        // Işıkları gökyüzünde gezdir
        if let Some(time_res) = world.get_resource::<Time>() {
            let res_dt = time_res.dt;
            if let Some(mut q) = world.query_mut_ref::<Transform, PointLight>() {
                for (_e, t, _l) in q.iter_mut() {
                    // Güneş (Directional Light) gibi gökyüzünde dönmesi için Y ve X/Z ekseninde parabol çiziyoruz
                    let speed = std::f32::consts::PI * 0.2 * res_dt; 
                    let x = t.position.x;
                    let y = t.position.y;
                    // Yatay eksende çok hafif dön, asıl dikeyde (Y) in/çık
                    t.position.x = x * speed.cos() - y * speed.sin();
                    t.position.y = x * speed.sin() + y * speed.cos();
                }
            }
        }

        // Görsel Tekerlekleri Süspansiyon ile Senkronize Et (Visual Wheel Animation)
        if let (Some(vehicles), Some(names)) = (world.borrow::<yelbegen::physics::vehicle::VehicleController>(), world.borrow::<EntityName>()) {
            if let Some(&car_id) = vehicles.entity_dense.first() {
                if let Some(car_v) = vehicles.get(car_id) {
                    let front_comp = (car_v.wheels[0].compression + car_v.wheels[1].compression) / 2.0;
                    let rear_comp = (car_v.wheels[2].compression + car_v.wheels[3].compression) / 2.0;
                    
                    if let Some(mut trans) = world.borrow_mut::<Transform>() {
                        for &e in &names.entity_dense {
                            if let Some(n) = names.get(e) {
                                if n.0 == "Node" { // Ön Dingil (Front Axle GLTF Orijinal Z ekseni hizalaması)
                                    if let Some(t) = trans.get_mut(e) {
                                        t.position.z = -0.9 + front_comp;
                                    }
                                } else if n.0 == "Node.001" { // Arka Dingil
                                    if let Some(t) = trans.get_mut(e) {
                                        t.position.z = -0.9 + rear_comp;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ECS Güncellemeleri Tamamlandı: Hiyerarşiyi Traverse Et
        transform_hierarchy_system(world);

        // Script (Lua) Motorunu Çalıştır
        if let Some(engine) = state.script_engine.borrow_mut().as_mut() {
            if let (Some(mut transforms), Some(mut vels), Some(scripts)) = (world.borrow_mut::<Transform>(), world.borrow_mut::<Velocity>(), world.borrow::<yelbegen::scripting::Script>()) {
                // Sadece scripti olan varlıklarda iterasyon yap (dense array var)
                for &e in &scripts.entity_dense {
                    let script = match scripts.get(e) { Some(s) => s, None => continue };
                    let t = match transforms.get_mut(e) { Some(t) => t, None => continue };
                    let v = match vels.get_mut(e) { Some(v) => v, None => continue };
                    let ctx = yelbegen::scripting::engine::ScriptContext {
                        entity_id: e,
                        dt,
                        position: [t.position.x, t.position.y, t.position.z],
                        velocity: [v.linear.x, v.linear.y, v.linear.z],
                        key_w: input.is_key_pressed(KeyCode::KeyW as u32),
                        key_a: input.is_key_pressed(KeyCode::KeyA as u32),
                        key_s: input.is_key_pressed(KeyCode::KeyS as u32),
                        key_d: input.is_key_pressed(KeyCode::KeyD as u32),
                        key_space: input.is_key_pressed(KeyCode::Space as u32),
                    };

                    // Script yüklenmemişse veya güncellendiyse (Hot Reload)
                    let _ = engine.reload_if_changed(&script.file_path);

                    if let Ok(res) = engine.run_update(&ctx) {
                        if let Some(pos) = res.new_position {
                            t.position = Vec3::new(pos[0], pos[1], pos[2]);
                        }
                        if let Some(vel) = res.new_velocity {
                            v.linear = Vec3::new(vel[0], vel[1], vel[2]);
                        }
                    }
                }
            }
        }
    });

    // 4. ECS SİSTEMLERİ
    // Fizik sistemlerini App::schedule yerine doğrudan Update Hook içerisinde 
    // çağırdık çünkü AudioManager gibi oyun State'ine ihtiyacımız var.

    // 4. UI SEÇİM VE GÖRÜNTÜLEME
    app = app.set_ui(|world, state, ctx| {
        state.egui_wants_pointer = ctx.is_pointer_over_area();
        
        // --- PROFILER OVERLAY ---
        egui::Window::new("Profiler")
            .fixed_pos([10.0, 10.0])
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.style()).fill(egui::Color32::from_black_alpha(200)))
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("📊 BİLGİ EKRANI").color(egui::Color32::WHITE).strong());
                ui.separator();
                
                let fps = state.current_fps;
                let ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };
                
                // Renklendirme mantığı (FPS)
                let fps_color = if fps >= 55.0 { egui::Color32::GREEN }
                                else if fps >= 30.0 { egui::Color32::YELLOW }
                                else { egui::Color32::RED };

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("FPS:").color(egui::Color32::LIGHT_GRAY));
                    ui.label(egui::RichText::new(format!("{:.1}", fps)).color(fps_color).strong());
                    ui.label(egui::RichText::new(format!("({:.2} ms)", ms)).color(egui::Color32::GRAY));
                });
                
                let entity_count = world.entity_count();
                let draw_calls = world.query_ref::<yelbegen::renderer::components::MeshRenderer>()
                                      .map(|q| q.s1.dense.len())
                                      .unwrap_or(0);
                                      
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Varlık (Entity):").color(egui::Color32::LIGHT_GRAY));
                    ui.label(egui::RichText::new(format!("{}", entity_count)).color(egui::Color32::WHITE).strong());
                });

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Çizim Çağrısı (Draw Calls):").color(egui::Color32::LIGHT_GRAY));
                    ui.label(egui::RichText::new(format!("{}", draw_calls + 1)).color(egui::Color32::WHITE).strong()); // +1 Shadow Pass
                });
            });

        // --- ANA INSPECTOR ---
        egui::Window::new("Yelbegen Inspector")
            .default_pos([10.0, 120.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // SOL PANEL: Hiyerarşi
                    ui.vertical(|ui| {
                        ui.set_width(150.0);
                        ui.heading("Sahne (Hiyerarşi)");
                        ui.add_space(5.0);
                        if ui.button("💾 Sahneyi Kaydet").clicked() {
                            scene::SceneData::save(world, "scene.json");
                        }
                        if ui.button("📂 Yeniden Yükle").on_hover_text("Uygulamayı yeniden başlatın").clicked() {
                            println!("Lütfen yüklemek için uygulamayı kapatıp yeniden başlatın.");
                        }
                        ui.separator();
                        
                        if ui.button("➕ Yeni Maymun Ekle").clicked() {
                            state.spawn_monkey_requests.set(state.spawn_monkey_requests.get() + 1);
                        }
                        if ui.button("💡 Yeni Işık Ekle").clicked() {
                            state.spawn_light_requests.set(state.spawn_light_requests.get() + 1);
                        }
                        ui.separator();
                        
                        egui::ScrollArea::vertical().id_source("hierarchy").max_height(200.0).show(ui, |ui| {
                            if let Some(names) = world.borrow::<EntityName>() {
                                for i in 0..names.dense.len() {
                                    let e_id = names.entity_dense[i];
                                    let e_name = &names.dense[i].0;
                                    // Dahili objeleri (Gizmo, Kamera, Skybox) hiyerarşiden gizle
                                    if e_id == state.gizmo_x || e_id == state.gizmo_y || e_id == state.gizmo_z
                                        || e_id == state.player_id || e_id == state.skybox_id {
                                        continue;
                                    }
                                    let is_selected = state.inspector_selected_entity == Some(e_id);
                                    let display_name = format!("{} (ID: {})", e_name, e_id);
                                    if ui.selectable_label(is_selected, display_name).clicked() {
                                        state.inspector_selected_entity = Some(e_id);
                                    }
                                }
                            }
                        });
                    });

                    ui.separator();

                    // SAĞ PANEL: Inspector
                    ui.vertical(|ui| {
                        ui.set_min_width(200.0);
                        ui.heading("Bileşenler (Components)");

                        if let Some(e) = state.inspector_selected_entity {
                            if ui.button(egui::RichText::new("🗑️ Seçili Objeyi Sil").color(egui::Color32::RED)).clicked() {
                                world.despawn_by_id(e);
                                state.inspector_selected_entity = None;
                                state.dragging_axis = None;
                            }
                            
                            ui.add_space(5.0);
                            ui.label("Gizmo Modu:");
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut state.gizmo_mode, GizmoMode::Translate, "✥ Taşı (T)");
                                ui.selectable_value(&mut state.gizmo_mode, GizmoMode::Rotate, "↻ Çevir (R)");
                                ui.selectable_value(&mut state.gizmo_mode, GizmoMode::Scale, "↗ Ölçekle (S)");
                            });
                        }

                        ui.separator();

                        if let Some(e) = state.inspector_selected_entity {
                                // İsim
                                if let Some(names) = world.borrow::<EntityName>() {
                                    if let Some(n) = names.get(e) {
                                        ui.label(egui::RichText::new(&n.0).strong().size(16.0));
                                        ui.add_space(5.0);
                                    }
                                }

                            // Transform
                            if let Some(mut transforms) = world.borrow_mut::<Transform>() {
                                if let Some(t) = transforms.get_mut(e) {
                                    ui.label(egui::RichText::new("Transform").underline());
                                    ui.horizontal(|ui| {
                                        ui.label("P(X): "); ui.add(egui::DragValue::new(&mut t.position.x).speed(0.1));
                                        ui.label("P(Y): "); ui.add(egui::DragValue::new(&mut t.position.y).speed(0.1));
                                        ui.label("P(Z): "); ui.add(egui::DragValue::new(&mut t.position.z).speed(0.1));
                                    });
                                    ui.add_space(5.0);
                                }
                            }

                            // RigidBody
                            if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
                                if let Some(rb) = rbs.get_mut(e) {
                                    ui.label(egui::RichText::new("Fizik (RigidBody)").underline());
                                    ui.horizontal(|ui| {
                                        ui.label("Kütle: ");
                                        ui.add(egui::Slider::new(&mut rb.mass, 0.0..=10.0));
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Sekme: ");
                                        ui.add(egui::Slider::new(&mut rb.restitution, 0.0..=1.0));
                                    });
                                    ui.add_space(5.0);
                                }
                            }

                            // Material
                            if let Some(mut materials) = world.borrow_mut::<Material>() {
                                if let Some(mat) = materials.get_mut(e) {
                                    ui.label(egui::RichText::new("Materyal (PBR)").underline());
                                    let mut edit_albedo = [mat.albedo.x, mat.albedo.y, mat.albedo.z];
                                    ui.horizontal(|ui| {
                                        ui.label("Renk (Albedo): ");
                                        ui.color_edit_button_rgb(&mut edit_albedo);
                                    });
                                    mat.albedo.x = edit_albedo[0];
                                    mat.albedo.y = edit_albedo[1];
                                    mat.albedo.z = edit_albedo[2];

                                    ui.horizontal(|ui| {
                                        ui.label("Pürüzlülük: ");
                                        ui.add(egui::Slider::new(&mut mat.roughness, 0.0..=1.0));
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Metalik: ");
                                        ui.add(egui::Slider::new(&mut mat.metallic, 0.0..=1.0));
                                    });

                                    ui.separator();
                                    ui.label("Kaplama (Dinamik Doku):");
                                    let mut ui_source = mat.texture_source.clone().unwrap_or_else(|| "".to_string());
                                    ui.horizontal(|ui| {
                                        let res = ui.add(egui::TextEdit::singleline(&mut ui_source).hint_text("ör: demo/assets/wood.jpg"));
                                        if res.changed() {
                                            mat.texture_source = Some(ui_source.clone());
                                        }
                                        if ui.button("Yükle").clicked() && !ui_source.is_empty() {
                                            state.texture_load_requests.borrow_mut().push((e, ui_source.clone()));
                                        }
                                    });

                                    ui.add_space(5.0);
                                }
                            }

                            // PointLight
                            if let Some(mut lights) = world.borrow_mut::<PointLight>() {
                                if let Some(l) = lights.get_mut(e) {
                                    ui.label(egui::RichText::new("Işık (PointLight)").underline());
                                    let mut edit_color = [l.color.x, l.color.y, l.color.z];
                                    ui.horizontal(|ui| {
                                        ui.label("Renk: ");
                                        ui.color_edit_button_rgb(&mut edit_color);
                                    });
                                    l.color.x = edit_color[0];
                                    l.color.y = edit_color[1];
                                    l.color.z = edit_color[2];

                                    ui.horizontal(|ui| {
                                        ui.label("Yoğunluk: ");
                                        ui.add(egui::Slider::new(&mut l.intensity, 0.0..=10.0));
                                    });
                                    ui.add_space(5.0);
                                }
                            }

                        } else {
                            ui.label("Lütfen soldaki listeden bir obje seçin.");
                        }
                    });
                });

                ui.separator();
                if ui.button("🔄 Zıplayan Maymunu Başa Sar").clicked() {
                    if let Some(mut trans) = world.borrow_mut::<Transform>() {
                        if let Some(t) = trans.get_mut(state.bouncing_box_id) {
                            t.position = Vec3::new(0.0, 5.0, -8.0);
                        }
                    }
                    if let Some(mut vels) = world.borrow_mut::<Velocity>() {
                        if let Some(v) = vels.get_mut(state.bouncing_box_id) {
                            v.linear = Vec3::new(3.0, 0.0, 0.0);
                        }
                    }
                }
            });
    });

    // 6. RENDER HOOK
    app = app.set_render(|world, state, encoder, view, renderer, _light_time| {
        let aspect = if renderer.size.height > 0 {
            renderer.size.width as f32 / renderer.size.height as f32
        } else {
            1.0
        };

        let mut proj = Mat4::perspective(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
        let mut view_mat = Mat4::translation(Vec3::ZERO);
        let mut cam_pos = Vec3::ZERO;

        if let (Some(cameras), Some(mut transforms)) = (world.borrow::<Camera>(), world.borrow_mut::<Transform>()) {
            if let (Some(cam), Some(trans)) = (cameras.get(state.player_id), transforms.get(state.player_id)) {
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(trans.position);
                cam_pos = trans.position;
            }
            // Skybox her zaman Kamerayla aynı yerde durarak sonsuzluk hissi yaratır.
            if let Some(sky_t) = transforms.get_mut(state.skybox_id) {
                sky_t.position = cam_pos;
            }
        }
        
        let view_proj = proj * view_mat;

        // --- EVENT: YENİ OBJE EKLEME ---
        while state.spawn_monkey_requests.get() > 0 {
            let entity = world.spawn();
            let mut spawn_pos = cam_pos + Vec3::new(0.0, 0.0, -5.0);
            
            if let Some(cameras) = world.borrow::<Camera>() {
                if let Some(cam) = cameras.get(state.player_id) {
                    spawn_pos = cam_pos + cam.get_front() * 5.0; // 5 birim ileriye koy
                }
            }

            world.add_component(entity, Transform::new(spawn_pos));
            world.add_component(entity, Velocity::new(Vec3::ZERO)); 
            world.add_component(entity, Collider::new_aabb(1.0, 1.0, 1.0));
            world.add_component(entity, RigidBody::new(1.0, 0.5, 0.2, true));
            world.add_component(entity, EntityName("Yeni Küre".into()));
            
            // Olan mesh'i kopyala ki Instancing (Batching) devreye girsin!
            let mut mesh_clone = None;
            if let Some(meshes) = world.borrow::<Mesh>() {
                if let Some(m) = meshes.get(state.bouncing_box_id) {
                    mesh_clone = Some(m.clone());
                }
            }
            if let Some(mesh) = mesh_clone {
                world.add_component(entity, mesh);
            } else {
                world.add_component(entity, AssetManager::create_sphere(&renderer.device, 1.0, 16, 16));
            }
            // Rastgele renkli materyal
            let r = ((entity.id() * 73 + 17) % 255) as f32 / 255.0;
            let g = ((entity.id() * 137 + 43) % 255) as f32 / 255.0;
            let b = ((entity.id() * 199 + 7) % 255) as f32 / 255.0;
            {
                let mut bind_group_clone = None;
                if let Some(mats) = world.borrow::<yelbegen::renderer::components::Material>() {
                    if let Some(mat) = mats.get(state.bouncing_box_id) {
                        bind_group_clone = Some(mat.bind_group.clone());
                    }
                }
                if let Some(bg) = bind_group_clone {
                    let new_mat = yelbegen::renderer::components::Material::new(bg)
                        .with_pbr(Vec4::new(r, g, b, 1.0), 0.4, 0.1);
                    world.add_component(entity, new_mat);
                }
            }
            
            world.add_component(entity, yelbegen::renderer::components::MeshRenderer::new());
            
            // Yeni eklenen maymunu seç
            state.new_selection_request.set(Some(entity.id()));
            
            state.spawn_monkey_requests.set(state.spawn_monkey_requests.get() - 1);
        }

        while state.spawn_light_requests.get() > 0 {
            let entity = world.spawn();
            let mut spawn_pos = cam_pos + Vec3::new(0.0, 2.0, -3.0);
            
            if let Some(cameras) = world.borrow::<Camera>() {
                if let Some(cam) = cameras.get(state.player_id) {
                    spawn_pos = cam_pos + cam.get_front() * 3.0; // 3 birim ileriye koy
                }
            }

            world.add_component(entity, Transform::new(spawn_pos));
            world.add_component(entity, PointLight::new(Vec3::new(1.0, 1.0, 1.0), 3.0));
            world.add_component(entity, EntityName("Yeni Işık".into()));
            
            // Olan mesh'i kopyala
            let mut mesh_clone = None;
            if let Some(meshes) = world.borrow::<Mesh>() {
                if let Some(m) = meshes.get(state.bouncing_box_id) {
                    mesh_clone = Some(m.clone());
                }
            }
            if let Some(mesh) = mesh_clone {
                world.add_component(entity, mesh);
            } else {
                world.add_component(entity, AssetManager::create_sphere(&renderer.device, 0.3, 8, 8));
            }
            {
                let mut mat_bg = None;
                if let Some(mats) = world.borrow::<yelbegen::renderer::components::Material>() {
                    if let Some(mat) = mats.get(state.bouncing_box_id) {
                        mat_bg = Some(mat.bind_group.clone());
                    }
                }
                if let Some(bg) = mat_bg {
                    world.add_component(entity, yelbegen::renderer::components::Material::new(bg)
                        .with_unlit(Vec4::new(1.0, 1.0, 1.0, 1.0)));
                }
            }
            
            world.add_component(entity, yelbegen::renderer::components::MeshRenderer::new());
            
            // Yeni eklenen ışığı seç
            state.new_selection_request.set(Some(entity.id()));

            state.spawn_light_requests.set(state.spawn_light_requests.get() - 1);
        }

        // --- EVENT: DOKU (TEXTURE) YÜKLEME ---
        while let Some((e_id, path)) = state.texture_load_requests.borrow_mut().pop() {
            let mut am = state.asset_manager.borrow_mut();
            match am.load_material_texture(&renderer.device, &renderer.queue, &renderer.texture_bind_group_layout, &path) {
                Ok(bg) => {
                    // Query API ile Material component'e güvenli &mut erişim
                    if let Some(mut q) = world.query_mut::<yelbegen::renderer::components::Material>() {
                        for (e, mat) in q.iter_mut() {
                            if e == e_id {
                                mat.bind_group = bg.clone();
                                println!("Texture başarıyla yüklendi: {}", path);
                            }
                        }
                    }
                },
                Err(err) => {
                    println!("Texture yukleme hatasi: {}", err);
                }
            }
        }

        // --- SKELETAL ANIMATION UPDATE ---
        let delta_time = 1.0 / (state.current_fps.max(1.0));
        
        if let Some(mut q) = world.query_mut_mut::<yelbegen::renderer::components::AnimationPlayer, yelbegen::renderer::components::Skeleton>() {
            for (_e, anim_player, skeleton) in q.iter_mut() {
                if anim_player.animations.is_empty() { continue; }
                
                let active_idx = anim_player.active_animation.min(anim_player.animations.len() - 1);
                let anim = &anim_player.animations[active_idx];
                
                // Zamanı ilerlet
                anim_player.current_time += delta_time;
                if anim_player.current_time > anim.duration {
                    if anim_player.loop_anim {
                        anim_player.current_time %= anim.duration.max(0.001); // 0 div fix
                    } else {
                        anim_player.current_time = anim.duration;
                    }
                }
                
                let time = anim_player.current_time;
                
                // 1) Local Poses hesapla (Sadece animasyondan gelenleri ez, geri kalanı orijinal local_bind kalsın)
                let hierarchy = &skeleton.hierarchy;
                let mut local_poses = vec![yelbegen::math::mat4::Mat4::IDENTITY; hierarchy.joints.len()];
                
                for (i, joint) in hierarchy.joints.iter().enumerate() {
                    local_poses[i] = joint.local_bind_transform; // Varsayılan olarak T-Pose bekleme pozu
                }
                
                for track in &anim.translations {
                    if let Some(val) = track.get_interpolated(time, |a, b, t| a.lerp(b, t)) {
                        if let Some(b_idx) = hierarchy.joints.iter().position(|j| j.node_index == track.target_node) {
                            // translation'i degistir
                            // Aslında matrisi TRS şeklinde sıfırdan oluşturmamız daha doğru olur.
                            // Çok kaba bir simülasyon: mevcut local_bind_transform'un translation kısmını ezelim:
                            local_poses[b_idx].cols[3].x = val.x;
                            local_poses[b_idx].cols[3].y = val.y;
                            local_poses[b_idx].cols[3].z = val.z;
                        }
                    }
                }
                
                for track in &anim.rotations {
                    if let Some(val) = track.get_interpolated(time, |a, b, t| a.slerp(b, t)) {
                        if let Some(b_idx) = hierarchy.joints.iter().position(|j| j.node_index == track.target_node) {
                            // Rotation ezmek için mevcut translation ve scale'i koruyup yeniden çarpabiliriz:
                            let tr = yelbegen::math::vec3::Vec3::new(local_poses[b_idx].cols[3].x, local_poses[b_idx].cols[3].y, local_poses[b_idx].cols[3].z);
                            // Scale'i local_bind_transform'dan cikarmak zor, simdilik (1,1,1) varsayip sade donduruyoruz (cok basit yaklasim)
                            // Ileride Mat4::decomposed() metodunu math modulumuze ekleyerek daha iyi cekeriz.
                            local_poses[b_idx] = yelbegen::math::mat4::Mat4::translation(tr) * val.to_mat4();
                        }
                    }
                }
                
                for track in &anim.scales {
                    if let Some(val) = track.get_interpolated(time, |a, b, t| a.lerp(b, t)) {
                        if let Some(b_idx) = hierarchy.joints.iter().position(|j| j.node_index == track.target_node) {
                            local_poses[b_idx] = local_poses[b_idx] * yelbegen::math::mat4::Mat4::scale(val);
                        }
                    }
                }
                
                // 2) Global matrisleri hesapla (Hierarchy)
                let globals = hierarchy.calculate_global_matrices(&local_poses);
                
                // 3) Inverse Bind Matrices ile çarpıp Skeleton'un local_poses alanına yaz (ki shader bilsin)
                skeleton.local_poses.clear();
                for (i, global_mat) in globals.iter().enumerate() {
                    let final_mat = *global_mat * hierarchy.joints[i].inverse_bind_matrix;
                    skeleton.local_poses.push(final_mat);
                }
                
                // 4) GPU'ya gönder! (En faza 64 kemik)
                let mut gpu_data = [[[0.0f32; 4]; 4]; 64];
                for i in 0..skeleton.local_poses.len().min(64) {
                    gpu_data[i] = skeleton.local_poses[i].to_cols_array_2d();
                }
                renderer.queue.write_buffer(&skeleton.buffer, 0, bytemuck::cast_slice(&gpu_data));
            }
        }

        // Işık kaynaklarını topla (Maksimum 10)
        let mut lights_data = [yelbegen::renderer::renderer::LightData { position: [0.0; 4], color: [0.0; 4] }; 10];
        let mut num_lights = 0;
        
        if let Some(q) = world.query_ref_ref::<PointLight, Transform>() {
            for (_e, l, t) in q.iter() {
                if num_lights >= 10 { break; }
                lights_data[num_lights as usize] = yelbegen::renderer::renderer::LightData {
                    position: [t.position.x, t.position.y, t.position.z, l.intensity],
                    color: [l.color.x, l.color.y, l.color.z, 0.0],
                };
                num_lights += 1;
            }
        }

        // --- Directional Light (Güneş) Taraması ---
        let mut sun_dir = [0.0, -1.0, 0.0, 0.0];
        let mut sun_col = [0.0, 0.0, 0.0, 0.0];
        
        if let Some(q) = world.query_ref_ref::<yelbegen::renderer::components::DirectionalLight, Transform>() {
            for (_e, dl, t) in q.iter() {
                if dl.is_sun {
                    // Transform'un rotasyonundan ileri vektörü hesapla (Güneşin baktığı yön)
                    // Standartlara göre ışık '-Z' ye bakar
                    let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, -1.0)).normalize();
                    sun_dir = [forward.x, forward.y, forward.z, 1.0]; // w=1.0: güneş tanımlı
                    sun_col = [dl.color.x, dl.color.y, dl.color.z, dl.intensity];
                    break; // Sadece ilk ana güneşi al
                }
            }
        }

        // Shadow Mapping İçin Dinamik Ana Işık Kamerası Hazırla
        let mut light_view_proj = Mat4::IDENTITY;
        if sun_dir[3] > 0.5 {
            // Dinamik Frustum: Gölge kamerasını oyuncunun (cam_pos) tam üstüne/arkasına kilitleriz.
            let light_direction = Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]).normalize();
            // Güneşi kameranın uzağına yerleştirip, kameranın baktığı yeri aydınlatmasını sağla
            let light_pos = cam_pos - light_direction * 40.0; 
            
            let light_view = Mat4::look_at_rh(light_pos, cam_pos, Vec3::new(0.0, 1.0, 0.0));
            // 40 metre genişliğinde dik açılı yüksek kaliteli gölge kutusu (Orthographic)
            let light_proj = Mat4::orthographic(-40.0, 40.0, -40.0, 40.0, 0.1, 100.0);
            light_view_proj = light_proj * light_view;
        } else if num_lights > 0 {
            // Fallback: PointLight taklidi
            let l_pos = Vec3::new(lights_data[0].position[0], lights_data[0].position[1], lights_data[0].position[2]);
            let light_view = Mat4::look_at_rh(l_pos, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
            let light_proj = Mat4::orthographic(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0);
            light_view_proj = light_proj * light_view;
        }

        // Global Uniforms (Her frame sadece 1 kere gönderilir)
        let scene_uniform_data = yelbegen::renderer::renderer::SceneUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
            sun_direction: sun_dir,
            sun_color: sun_col,
            lights: lights_data,
            light_view_proj: light_view_proj.to_cols_array_2d(),
            num_lights,
            _padding: [0; 3],
        };
        renderer.queue.write_buffer(&renderer.global_uniform_buffer, 0, bytemuck::cast_slice(&[scene_uniform_data]));

        // --- BATCHING (INSTANCING) HAZIRLIĞI VE FRUSTUM CULLING ---
        use yelbegen::renderer::renderer::InstanceRaw;

        let frustum = yelbegen::math::frustum::Frustum::from_matrix(&view_proj);

        struct BatchData {
            vbuf: std::sync::Arc<wgpu::Buffer>,
            vertex_count: u32,
            bind_group: std::sync::Arc<wgpu::BindGroup>,
            skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
            instances: Vec<InstanceRaw>,
        }

        let mut batches: std::collections::HashMap<(*const wgpu::Buffer, *const wgpu::BindGroup, *const wgpu::BindGroup), BatchData> = std::collections::HashMap::new();

        let renderers = world.borrow::<yelbegen::renderer::components::MeshRenderer>();
        let skeletons = world.borrow::<yelbegen::renderer::components::Skeleton>();
        let lod_groups = world.borrow::<yelbegen::renderer::components::LodGroup>();
        
        if let Some(q) = world.query_ref_ref_ref::<Mesh, Transform, Material>() {
            for (e, mesh, trans, mat) in q.iter() {
                // Sadece MeshRenderer tagli olanları çiz:
                if let Some(r) = &renderers {
                    if r.get(e).is_none() { continue; }
                } else { continue; }
                // --- GLOBAL TRANSFORM HESAPLAMA ---
                // transform_hierarchy_system() daha önce tüm hiyerarşiyi t.global_matrix'te çözdüğü için 
                // doğrudan global_matrix'i kullanmamız yeterlidir. Çift çarpım yapmıyoruz!
                let global_model = trans.global_matrix;
                
                let center_mat = yelbegen::math::mat4::Mat4::translation(mesh.center_offset);
                let model = global_model * center_mat;

                // Frustum Culling (Görüş açısı dışındakileri atla)
                if e != state.skybox_id && e != state.gizmo_x && e != state.gizmo_y && e != state.gizmo_z {
                    let world_aabb = mesh.bounds.transform(&model);
                    if !frustum.contains_aabb(&world_aabb) {
                        continue;
                    }
                }

                // --- LOD (Level of Detail) SEÇİMİ ---
                // Eğer entity'de LodGroup varsa, kameraya mesafeye göre düşük/yüksek detay mesh seç
                let active_mesh = if let Some(lods) = &lod_groups {
                    if let Some(lod) = lods.get(e) {
                        let world_pos = Vec3::new(model.cols[3].x, model.cols[3].y, model.cols[3].z);
                        let dist = cam_pos.distance(world_pos);
                        lod.select_mesh(dist).unwrap_or(mesh)
                    } else {
                        mesh
                    }
                } else {
                    mesh
                };

                let instance_data = InstanceRaw {
                    model: model.to_cols_array_2d(),
                    albedo_color: [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                    roughness: mat.roughness,
                    metallic: mat.metallic,
                    unlit: mat.unlit,
                    _padding: 0.0,
                };

                // --- SKELETON (KEMİK) ARAMASI ---
                // Yalnızca child meshin değil, atalarından (Root) herhangi birisinde Skeleton var mı diye tırman:
                let mut skel_bg = renderer.dummy_skeleton_bind_group.clone();
                if let Some(skels) = &skeletons {
                    if let Some(s) = skels.get(e) {
                         skel_bg = s.bind_group.clone();
                    } else if let Some(parents) = world.borrow::<Parent>() {
                         let mut curr = e;
                         while let Some(p) = parents.get(curr) {
                             if let Some(s) = skels.get(p.0) {
                                 skel_bg = s.bind_group.clone();
                                 break;
                             }
                             curr = p.0;
                         }
                    }
                }

                let vbuf_ptr = std::sync::Arc::as_ptr(&active_mesh.vbuf);
                let bg_ptr = std::sync::Arc::as_ptr(&mat.bind_group);
                let skel_ptr = std::sync::Arc::as_ptr(&skel_bg);

                let batch = batches.entry((vbuf_ptr, bg_ptr, skel_ptr)).or_insert_with(|| BatchData {
                    vbuf: active_mesh.vbuf.clone(),
                    vertex_count: active_mesh.vertex_count,
                    bind_group: mat.bind_group.clone(),
                    skeleton_bg: skel_bg,
                    instances: Vec::new(),
                });
                
                batch.instances.push(instance_data);
            }
        }

        // Batch'ler için GPU tarafında geçici instancing buffer'ı oluştur
        let mut gpu_batches = Vec::new();
        use wgpu::util::DeviceExt;
        for (_, batch) in batches {
            let instance_buf = renderer.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&batch.instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
            gpu_batches.push((batch, instance_buf));
        }

        // --- 1. GÖLGE PASS (Shadow Pass) ---
        {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shadow Pass"),
                color_attachments: &[], // Shadow pass sadece Depth'e çizer
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.shadow_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            shadow_pass.set_pipeline(&renderer.shadow_pipeline);

            // Tıpkı main render gibi gruplanmış nesneleri tek draw çağrısıyla bas
            for (batch, instance_buf) in &gpu_batches {
                shadow_pass.set_bind_group(0, &renderer.global_bind_group, &[]);
                shadow_pass.set_bind_group(1, &batch.skeleton_bg, &[]);
                shadow_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                shadow_pass.set_vertex_buffer(1, instance_buf.slice(..));
                shadow_pass.draw(0..batch.vertex_count, 0..batch.instances.len() as u32);
            }
        }

        // --- 2. ANA RENDER PASS (HDR Offscreen Texture'a çiz) ---
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass (HDR)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.hdr_texture_view, // Artık ekran yerine HDR texture'a çiziyoruz!
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.15, b: 0.20, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&renderer.render_pipeline);

            for (batch, instance_buf) in &gpu_batches {
                render_pass.set_bind_group(0, &renderer.global_bind_group, &[]);
                render_pass.set_bind_group(1, &batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &batch.skeleton_bg, &[]);
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.set_vertex_buffer(1, instance_buf.slice(..));
                render_pass.draw(0..batch.vertex_count, 0..batch.instances.len() as u32);
            }
        }

        // --- 3. POST-PROCESSING (Bloom + Tone Mapping → Ekrana Yaz) ---
        renderer.run_post_processing(encoder, view);
    });

    app.run();
}
