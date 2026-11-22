-- Kırıcı: bir tuğla kırma oyunu, bütün kuralları burada, Lua'da. Rust tarafı yalnız sahneyi
-- (kamera, ışık) kurar, `PlayLoop` ile oynatır ve skoru HUD'a yazar (`skor`, `durum`,
-- `kalan` değişkenlerini okuyarak). Duvarlar, tuğlalar, raket ve top buradan doğar.
--
-- Oyun XY düzleminde: x sağa, y yukarı. Top yerçekimsiz; hızını bu betik sabit tutar.

local GENISLIK = 10.0     -- duvarlar arası
local TAVAN = 12.0
local HIZ = 7.0
local TOP_R = 0.2
local RAKET_Y = 0.6
local RAKET_YARI = 0.9
local SIRA, SUTUN = 4, 8
-- Topun yönü: yatay bileşen en az bu kadar, ki top duvara çözücünün sekme eşiğinin (1 m/s)
-- üstünde çarpsın — altında çarpan top sekmez, duvar boyunca dikey bir döngüye girer. Dikey
-- bileşen en az bu kadar, ki top sonsuza dek yatay sekmesin.
local MIN_YATAY, MIN_DIKEY = 0.25, 0.35
-- Raketin kenarına değen top bu açıyla (dikeyden, radyan) çıkar; ortasına değen dik çıkar.
local RAKET_ACI = 1.05

skor = 0
kalan = 0
durum = "oynuyor"   -- "oynuyor" | "kazandi" | "kaybetti"

local tuglalar = {}  -- id -> { x =, y = }
local top = nil
local raket = nil
local raket_x = 0.0
local kuruldu = false

local function kutu(ad, x, y, hx, hy, r, g, b)
    local id = entity.spawn(ad, x, y, 0.0)
    entity.set_mesh(id, "standard_cube")
    entity.set_scale(id, hx, hy, 0.4)
    entity.set_color(id, r, g, b)
    physics.add_box_collider(id, hx, hy, 0.4)
    physics.set_material(id, 0.0, 1.0)   -- kaygan ve tam sekmeli
    return id
end

local function kur()
    local w = GENISLIK / 2
    kutu("duvar", -w - 0.25, TAVAN / 2, 0.25, TAVAN / 2 + 0.5, 0.3, 0.3, 0.35)
    kutu("duvar", w + 0.25, TAVAN / 2, 0.25, TAVAN / 2 + 0.5, 0.3, 0.3, 0.35)
    kutu("duvar", 0.0, TAVAN + 0.25, w + 0.5, 0.25, 0.3, 0.3, 0.35)
    local tx = GENISLIK / SUTUN
    for s = 0, SIRA - 1 do
        for c = 0, SUTUN - 1 do
            local x = -w + tx * (c + 0.5)
            local y = TAVAN - 1.5 - s * 0.6
            local id = kutu("tugla", x, y, tx / 2 - 0.06, 0.25, 0.9 - 0.15 * s, 0.35 + 0.1 * s, 0.2)
            tuglalar[id] = { x = x, y = y }
            kalan = kalan + 1
        end
    end
    raket = kutu("raket", 0.0, RAKET_Y, RAKET_YARI, 0.15, 0.2, 0.6, 0.9)
    top = entity.spawn("top", 0.0, RAKET_Y + 0.6, 0.0)
    entity.set_mesh(top, "sphere")
    entity.set_scale(top, TOP_R, TOP_R, TOP_R)
    entity.set_color(top, 0.95, 0.95, 0.95)
    physics.add_sphere_collider(top, TOP_R)
    physics.set_material(top, 0.0, 1.0)
    physics.add_rigidbody(top, 1.0, false)
    entity.set_velocity(top, HIZ * 0.45, HIZ * 0.89, 0.0)
    kuruldu = true
end

local function isaret(v, yedek)
    if v > 0 then return 1 elseif v < 0 then return -1 else return yedek end
end

-- Topun hızı sabit, yönü iki sınırın içinde. `yon` verilmezse fizikten okunan hız.
local function hizi_tut(yon)
    local vx, vy
    if yon then
        vx, vy = yon.x, yon.y
    else
        local v = entity.get_velocity(top)
        vx, vy = v.x, v.y
    end
    local n = math.sqrt(vx * vx + vy * vy)
    if n < 1e-3 then vx, vy, n = 0.4, 1.0, math.sqrt(1.16) end
    vx, vy = vx / n, vy / n
    -- Dikey tam sıfırken merkeze doğru, ki duvardan uzaklaşsın.
    local sx = isaret(vx, entity.get_position(top).x > 0 and -1 or 1)
    local sy = isaret(vy, 1)
    if math.abs(vy) < MIN_DIKEY then
        vy = sy * MIN_DIKEY
        vx = sx * math.sqrt(1 - vy * vy)
    elseif math.abs(vx) < MIN_YATAY then
        vx = sx * MIN_YATAY
        vy = sy * math.sqrt(1 - vx * vx)
    end
    entity.set_velocity(top, vx * HIZ, vy * HIZ, 0.0)
end

-- Otomatik pilotun hedefi: kalan en alttaki tuğla, eşitse topa en yakını.
local function hedef_tugla(x)
    local en = nil
    for _, t in pairs(tuglalar) do
        if en == nil or t.y < en.y - 1e-3 or (math.abs(t.y - en.y) < 1e-3 and math.abs(t.x - x) < math.abs(en.x - x)) then
            en = t
        end
    end
    return en
end

local function raketi_sur(dt)
    local hedef = raket_x
    if OTO then
        -- İnen topu raketin öyle bir noktasıyla karşıla ki çıkış açısı hedef tuğlaya baksın.
        local p = entity.get_position(top)
        local v = entity.get_velocity(top)
        hedef = p.x
        local t = hedef_tugla(p.x)
        if t ~= nil and v.y < 0 then
            local aci = math.atan(t.x - p.x, t.y - RAKET_Y)
            local o = math.max(-0.85, math.min(0.85, aci / RAKET_ACI))
            hedef = p.x - o * RAKET_YARI
        end
    else
        if input.is_pressed("left") or input.is_pressed("a") then hedef = raket_x - 9.0 * dt end
        if input.is_pressed("right") or input.is_pressed("d") then hedef = raket_x + 9.0 * dt end
    end
    local sinir = GENISLIK / 2 - RAKET_YARI
    raket_x = math.max(-sinir, math.min(sinir, hedef))
    entity.set_position(raket, raket_x, RAKET_Y, 0.0)
end

function on_update(dt)
    if not kuruldu then
        kur()
        return
    end
    if durum ~= "oynuyor" then return end

    -- Bu kareden önceki her fizik adımının çarpışmaları, her biri bir kez.
    local yon = nil
    for _, ev in ipairs(physics.collisions) do
        if ev.status == "enter" then
            local diger = nil
            if ev.entity_a == top then diger = ev.entity_b elseif ev.entity_b == top then diger = ev.entity_a end
            if diger ~= nil and tuglalar[diger] then
                tuglalar[diger] = nil
                entity.destroy(diger)
                skor = skor + 1
                kalan = kalan - 1
            elseif diger == raket then
                -- Çıkış açısı, topun rakete değdiği yerden.
                local o = (entity.get_position(top).x - raket_x) / RAKET_YARI
                local aci = math.max(-1, math.min(1, o)) * RAKET_ACI
                yon = { x = math.sin(aci), y = math.cos(aci) }
            end
        end
    end

    raketi_sur(dt)
    hizi_tut(yon)

    if entity.get_position(top).y < -1.0 then durum = "kaybetti" end
    if kalan == 0 then durum = "kazandi" end
end
