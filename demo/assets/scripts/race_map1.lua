-- race_map1.lua
-- Araba Yarışı Oyunu — Harita 1: "Çöl Yarışı"
-- Bu script global olarak yüklenir ve on_update her frame çağrılır.

local started    = false
local intro_done = false
local elapsed    = 0.0
local countdown_shown = { [1]=false, [2]=false, [3]=false }

-- Başlangıçta bir kez çalışır
local function setup_race()
    -- Pist üzerinde 5 checkpoint (id, x, y, z, yakalama_yarıçapı)
    race.add_checkpoint(1,  30,  0,   0,  6)
    race.add_checkpoint(2,  30,  0, -40,  6)
    race.add_checkpoint(3,   0,  0, -60,  6)
    race.add_checkpoint(4, -30,  0, -40,  6)
    race.add_checkpoint(5, -30,  0,   0,  6)

    -- Intro cutscene + geri sayım
    cutscene.play("race_intro")
    dialogue.show("Anons", "Hazır ol! Yarış başlıyor...", 1.2)

    print("Yarış kurulumu tamamlandı, 5 checkpoint eklendi.")
end

-- Her frame çağrılır
function on_update(ctx)
    elapsed = ctx.elapsed

    -- İlk frame kurulumu (sadece bir kez)
    if not intro_done then
        setup_race()
        intro_done = true
    end

    -- Geri sayım göstergesi (3, 2, 1)
    if elapsed > 1.0 and not countdown_shown[3] then
        dialogue.show("Anons", "3...", 0.9)
        countdown_shown[3] = true
    end
    if elapsed > 2.0 and not countdown_shown[2] then
        dialogue.show("Anons", "2...", 0.9)
        countdown_shown[2] = true
    end
    if elapsed > 3.0 and not countdown_shown[1] then
        dialogue.show("Anons", "1...", 0.9)
        countdown_shown[1] = true
    end

    -- Yarış başlangıcı (4. saniyede)
    if elapsed > 4.0 and not started then
        cutscene.stop()
        dialogue.show("Anons", "BAŞLA! 🏁", 2.0)

        -- Araç entity'sini bul ve kamerayı bağla
        local car_id = scene.find_by_name("Araba")
        if car_id then
            camera.follow(car_id)
            camera.set_fov(75)
            print("Kamera araca bağlandı: " .. tostring(car_id))
        else
            print("Uyarı: 'Araba' entity bulunamadı, serbest kamera kalacak.")
        end

        -- Yarışı başlat (Rust tarafında race_status = Running yapar)
        race.start()

        started = true
        print("Yarış başladı! Süre sayıyor...")
    end

    -- Rehber diyaloglar
    if started then
        if elapsed > 6.0 and elapsed < 6.2 then
            dialogue.show("Yardımcı Pilot", "İlk virajı hızlı al! →", 2.5)
        end
        if elapsed > 18.0 and elapsed < 18.2 then
            dialogue.show("Yardımcı Pilot", "Son düzlük! Gaz tam! 🔥", 2.5)
        end
    end
end
