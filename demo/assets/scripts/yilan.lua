-- Yılan: oyunun bütün kuralları burada, Lua'da. Rust tarafı yalnız sahneyi kurar, betiği
-- yükler ve skoru HUD'a yazar (`skor`, `durum` değişkenlerini okuyarak).
--
-- Izgara 16 × 16, hücre 1 m; yılan her `ADIM` saniyede bir hücre ilerler.

local N = 16
local ADIM = 0.14

-- Okunan durum (Rust HUD'u bunları okur).
skor = 0
durum = "oynuyor"   -- "oynuyor" | "bitti"

local yilan = {}     -- { {x=, z=, id=} ... } baş önde
local yon = { x = 1, z = 0 }
local sonraki_yon = { x = 1, z = 0 }
local yem = nil      -- { x=, z=, id= }
tohum = 12345
local birikim = 0
local kuruldu = false

local function konum(x, z)
    return x - N / 2 + 0.5, 0.5, z - N / 2 + 0.5
end

local function parca(x, z, bas)
    local px, py, pz = konum(x, z)
    local id = entity.spawn("yilan", px, py, pz)
    entity.set_mesh(id, "standard_cube")
    entity.set_scale(id, 0.45, 0.45, 0.45)
    if bas then
        entity.set_color(id, 0.95, 0.75, 0.10)
    else
        entity.set_color(id, 0.20, 0.70, 0.25)
    end
    return { x = x, z = z, id = id }
end

local function dolu(x, z)
    for _, p in ipairs(yilan) do
        if p.x == x and p.z == z then return true end
    end
    return false
end

local function yem_koy()
    -- Rastgele boş bir hücre; deterministik olsun diye kendi üretecimiz.
    for _ = 1, 400 do
        tohum = (tohum * 1103515245 + 12345) % 2147483648
        local x = tohum % N
        tohum = (tohum * 1103515245 + 12345) % 2147483648
        local z = tohum % N
        if not dolu(x, z) then
            local px, py, pz = konum(x, z)
            local id = entity.spawn("yem", px, py, pz)
            entity.set_mesh(id, "sphere")
            entity.set_scale(id, 0.35, 0.35, 0.35)
            entity.set_color(id, 0.90, 0.15, 0.15)
            yem = { x = x, z = z, id = id }
            return
        end
    end
end

local function temizle()
    for _, p in ipairs(yilan) do entity.destroy(p.id) end
    yilan = {}
    if yem then entity.destroy(yem.id) end
    yem = nil
end

local function basla()
    temizle()
    tohum = 12345
    skor = 0
    durum = "oynuyor"
    yon = { x = 1, z = 0 }
    sonraki_yon = { x = 1, z = 0 }
    for i = 0, 2 do
        table.insert(yilan, parca(5 - i, 8, i == 0))
    end
    yem_koy()
end

-- Girdi: ok tuşları / WASD; ters yöne dönülmez. `oto_yon` dışarıdan (sınama) verilebilir.
local function girdi()
    local istek = nil
    if input.is_just_pressed("up") or input.is_just_pressed("w") then istek = { x = 0, z = -1 } end
    if input.is_just_pressed("down") or input.is_just_pressed("s") then istek = { x = 0, z = 1 } end
    if input.is_just_pressed("left") or input.is_just_pressed("a") then istek = { x = -1, z = 0 } end
    if input.is_just_pressed("right") or input.is_just_pressed("d") then istek = { x = 1, z = 0 } end
    if istek and not (istek.x == -yon.x and istek.z == -yon.z) then
        sonraki_yon = istek
    end
end

-- Sınama kipi: yemin olduğu yöne, kendine çarpmadan dön.
local function oto()
    local bas = yilan[1]
    local adaylar = { { x = 1, z = 0 }, { x = -1, z = 0 }, { x = 0, z = 1 }, { x = 0, z = -1 } }
    local en_iyi, en_iyi_d = nil, 1e9
    for _, a in ipairs(adaylar) do
        local nx, nz = bas.x + a.x, bas.z + a.z
        local ters = (a.x == -yon.x and a.z == -yon.z)
        if not ters and nx >= 0 and nx < N and nz >= 0 and nz < N and not dolu(nx, nz) then
            local d = math.abs(nx - yem.x) + math.abs(nz - yem.z)
            if d < en_iyi_d then en_iyi, en_iyi_d = a, d end
        end
    end
    if en_iyi then sonraki_yon = en_iyi end
end

local function adim()
    yon = sonraki_yon
    local bas = yilan[1]
    local nx, nz = bas.x + yon.x, bas.z + yon.z
    if nx < 0 or nx >= N or nz < 0 or nz >= N or dolu(nx, nz) then
        durum = "bitti"
        return
    end
    -- Eski baş gövde rengine döner, yeni baş önde doğar.
    entity.set_color(bas.id, 0.20, 0.70, 0.25)
    table.insert(yilan, 1, parca(nx, nz, true))
    if yem and nx == yem.x and nz == yem.z then
        skor = skor + 1
        entity.destroy(yem.id)
        yem = nil
        yem_koy()
    else
        local kuyruk = table.remove(yilan)
        entity.destroy(kuyruk.id)
    end
end

function on_update(ctx)
    if not kuruldu then
        basla()
        kuruldu = true
        return
    end
    if durum == "bitti" then
        if input.is_just_pressed("r") or input.is_just_pressed("space") then basla() end
        return
    end
    girdi()
    if OTO then oto() end
    birikim = birikim + ctx.dt
    while birikim >= ADIM and durum == "oynuyor" do
        birikim = birikim - ADIM
        if OTO then oto() end
        adim()
    end
end
