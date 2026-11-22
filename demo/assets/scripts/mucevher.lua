-- Mücevher: hırsız tetikleyicisine girince toplanır ve kendini yok eder.
--
-- Bütün mücevherler bu dosyayı paylaşır; `toplanan` hepsinin ortak sayacıdır (Rust okur).

toplanan = 0
local alindi = {}     -- id -> true

function on_entity_update(id, dt, props)
    if alindi[id] then return end
    local hirsiz = scene.find_by_name("hirsiz")
    -- Bu kareden önceki her fizik adımının tetikleyici olayları, her biri bir kez.
    for _, ev in ipairs(physics.triggers) do
        if ev.status == "enter" and ev.trigger_id == id and ev.other_id == hirsiz then
            alindi[id] = true
            toplanan = toplanan + 1
            entity.destroy(id)
            return
        end
    end
end
