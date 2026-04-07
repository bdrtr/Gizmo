-- race_map1.lua
-- Araba Yarışı Oyunu — Harita 1: "Çöl Yarışı"
--
-- KULLANIM:
--   Entity'e Script component ekle ve bu dosyayı işaret et.
--   VEYA ScriptEngine:load_script("demo/assets/scripts/race_map1.lua") ile global yükle.

local started    = false
local intro_done = false
local elapsed    = 0.0
local car_id     = nil   -- Lua'dan takip edilecek araç entity id

-- Başlangıçta bir kez çalışır (on_init varsa engine çağırır)
local function setup_race()
    -- Checkpoint'leri tanımla (id, x, y, z, yakalama_yarıçapı)
    race.add_checkpoint(1,   30,  0,  0,   6)
    race.add_checkpoint(2,   30,  0, -40,  6)
    race.add_checkpoint(3,    0,  0, -60,  6)
    race.add_checkpoint(4,  -30,  0, -40,  6)
    race.add_checkpoint(5,  -30,  0,   0,  6)

    -- Intro ara sahnesi
    cutscene.play("race_intro")

    -- 3 saniyelik geri sayım diyaloğu
    dialogue.show("Anons", "3...", 1.0)
end

-- Her frame çağrılır
function on_update(ctx)
    elapsed = ctx.elapsed

    -- İlk frame kurulumu
    if not intro_done then
        setup_race()
        intro_done = true
    end

    -- Cutscene 3 saniye sonra biter, yarış başlar
    if elapsed > 3.0 and not started then
        cutscene.stop()
        dialogue.show("Anons", "BAŞLA! 🏁", 2.0)
        race.reset()   -- checkpoint'leri ve timer'ı sıfırla

        -- Araç entity'sini bul ve kamerayı bağla
        car_id = scene.find_by_name("Araba")
        if car_id then
            camera.follow(car_id)
            camera.set_fov(75)
        end

        started = true
        print("Yarış başladı!")
    end

    -- Dinamik diyalog: ilk checkpoint'e yaklaşınca ipucu ver
    if started and elapsed > 5.0 and elapsed < 5.1 then
        dialogue.show("Yardımcı", "İlk virajı hızlı al!", 2.5)
    end

    -- Yarış bitince kutlama
    if started and scene.entity_count() > 0 then
        -- Bu kontrol process_game_commands Rust tarafında yapılıyor,
        -- buradan sadece ekstra efekt tetikleyebiliriz
    end
end
