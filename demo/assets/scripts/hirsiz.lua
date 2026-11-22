-- Hırsız: avlunun güney ucundan çıkar, üç devriye hattını geçer, kuzeydeki mücevherleri toplar.
-- Bir bekçiye `YAKALAMA` metreden fazla yaklaşırsa yakalanır.
--
-- `props.oto`: otomatik pilot. "dikkatli" bir hattı yalnız oradaki bekçi geçiş boyunca uzak
-- kalacaksa geçer; "dalgin" hiç beklemez. Yoksa elle: ok tuşları ya da WASD.

durum = "oynuyor"   -- "oynuyor" | "yakalandi" | "kazandi"  (Rust okur)
yakalayan = ""
bekleme = 0         -- bir hattın önünde beklenen toplam süre, saniye (Rust okur)

local HIZ = 4.0
local YAKALAMA = 1.0
local HATLAR = { -4.0, 0.0, 4.0 }          -- bekçilerin z'si, güneyden kuzeye
local BANT = 1.0                            -- hattın iki yanındaki tehlikeli şerit
local SIRALAR = { -6.0, -2.0, 2.0, 6.5 }   -- hatların arasındaki bekleme sıraları; sonuncusu mücevherlerin
local UC = 6.0                              -- bekçilerin x uçları
local PAY = 0.7                             -- tahminin hatasına bırakılan pay

local sira = 1          -- gidilen sıra
local geciyor = false   -- bir hattı geçiyor mu (karar verildi, dönüş yok)
local supurme = -6.5    -- son sırada gidilen x
local onceki = {}       -- bekçi id -> önceki x (hız tahmini için)
local hizlar = {}       -- bekçi id -> tahmini x hızı

local function bekciler()
    local l = {}
    for i = 1, #HATLAR do
        local id = scene.find_by_name("bekci" .. i)
        if id then l[i] = id end
    end
    return l
end

local function mucevher_kaldi()
    for _, id in ipairs(scene.get_all_entities()) do
        if entity.get_name(id) == "mucevher" then return true end
    end
    return false
end

-- Bir bekçinin `t` saniye sonraki x'i: uçlarda geri dönerek.
local function ileri_x(x, v, t)
    local u = (x + v * t + UC) % (4 * UC)
    if u > 2 * UC then u = 4 * UC - u end
    return u - UC
end

-- `z0`'dan `z1`'e, `x`'te, hat `zl`'yi geçmek güvenli mi?
local function guvenli(bekci, x, z0, zl)
    local v = hizlar[bekci]
    if v == nil then return false end
    local gx = entity.get_position(bekci).x
    local t0 = (zl - BANT - z0) / HIZ - 0.15
    local t1 = (zl + BANT - z0) / HIZ + 0.15
    local t = math.max(0, t0)
    while t <= t1 do
        if math.abs(ileri_x(gx, v, t) - x) < YAKALAMA + PAY then return false end
        t = t + 0.05
    end
    return true
end

local function otomatik(id, p, dt, props)
    local hedef_z = SIRALAR[sira]
    if sira == #SIRALAR and math.abs(p.z - hedef_z) < 0.1 then
        -- Son sıra: önce sola, sonra sağa süpür.
        if math.abs(p.x - supurme) < 0.1 then supurme = 6.5 end
        return (supurme > p.x and HIZ or -HIZ), 0
    end
    if math.abs(p.z - hedef_z) < 0.05 and not geciyor then
        -- Bir sıradayız: sıradaki hattı geçmeye karar ver.
        local hat = sira          -- SIRALAR[sira] ile SIRALAR[sira+1] arasındaki hat
        local b = bekciler()[hat]
        if props.oto == "dalgin" or b == nil or guvenli(b, p.x, p.z, HATLAR[hat]) then
            sira = sira + 1
            geciyor = true
        else
            bekleme = bekleme + dt
        end
        return 0, 0
    end
    -- Hedef sıraya yürü.
    local dz = hedef_z - p.z
    if math.abs(dz) <= HIZ * dt then
        geciyor = false
        return 0, dz / dt
    end
    return 0, (dz > 0 and HIZ or -HIZ)
end

local function elle()
    local vx, vz = 0, 0
    if input.is_pressed("left") or input.is_pressed("a") then vx = vx - HIZ end
    if input.is_pressed("right") or input.is_pressed("d") then vx = vx + HIZ end
    if input.is_pressed("up") or input.is_pressed("w") then vz = vz + HIZ end
    if input.is_pressed("down") or input.is_pressed("s") then vz = vz - HIZ end
    return vx, vz
end

function on_entity_update(id, dt, props)
    -- Bekçilerin hızı, iki karenin konum farkından.
    for _, b in pairs(bekciler()) do
        local x = entity.get_position(b).x
        if onceki[b] ~= nil and dt > 0 then hizlar[b] = (x - onceki[b]) / dt end
        onceki[b] = x
    end
    local v = entity.get_velocity(id)
    if durum ~= "oynuyor" then
        entity.set_velocity(id, 0, v.y, 0)
        return
    end
    local p = entity.get_position(id)
    for i, b in pairs(bekciler()) do
        local q = entity.get_position(b)
        if (q.x - p.x) ^ 2 + (q.z - p.z) ^ 2 < YAKALAMA * YAKALAMA then
            durum = "yakalandi"
            yakalayan = "bekci" .. i
            entity.set_velocity(id, 0, v.y, 0)
            return
        end
    end
    if not mucevher_kaldi() then
        durum = "kazandi"
        entity.set_velocity(id, 0, v.y, 0)
        return
    end
    local vx, vz
    if props.oto then vx, vz = otomatik(id, p, dt, props) else vx, vz = elle() end
    entity.set_velocity(id, vx, v.y, vz)
end
