-- Bekçi: iki nokta arasında, x ekseninde devriye gezer.
--
-- Bütün bekçiler bu tek dosyayı paylaşır: betikler yola göre bir kez yüklenir, ortamları ortaktır.
-- Her bekçinin kendi değerleri (`props`: ax, bx, z, hiz) sahneden gelir; kendi durumu (yönü)
-- kimliğiyle anahtarlanmış bir tabloda durur — ortak bir global, bütün bekçileri birlikte
-- döndürürdü.

bekciler = 0          -- on_init kaç kez çalıştı (Rust okur)
local yon = {}        -- id -> 1 | -1

function on_init(id, props)
    bekciler = bekciler + 1
    yon[id] = 1
    -- Hızlı bekçi daha kırmızı.
    local k = math.min(1, props.hiz / 3)
    entity.set_color(id, 0.5 + 0.5 * k, 0.25 * (1 - k), 0.15)
end

function on_entity_update(id, dt, props)
    local d = yon[id]
    -- on_init çalışmadıysa bekçi kıpırdamaz.
    if d == nil then return end
    local x = entity.get_position(id).x + d * props.hiz * dt
    if x >= props.bx then
        x = props.bx
        yon[id] = -1
    elseif x <= props.ax then
        x = props.ax
        yon[id] = 1
    end
    entity.set_position(id, x, 0.6, props.z)
end
