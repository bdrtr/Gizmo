-- Yağmur (Rain) Efekti Serbest Scripti
-- Bu script bir entity'nin yağmur damlası gibi davranmasını sağlar.
-- physics motorundan bağımsız pozisyon bazlı düşüş yapar.

local reset_height = 25.0
local fall_speed = 20.0
local ground_y = 0.0

function on_update(ctx)
    local pos = ctx.position
    local vel = ctx.velocity
    local dt = ctx.dt
    
    -- Yer çekimi etkisi (sabit bir ivme ile aşağı düşüş)
    vel.y = vel.y - (9.8 * 2.0 * dt)
    
    -- Limit hızı (terminal velocity)
    if vel.y < -fall_speed then
        vel.y = -fall_speed
    end
    
    -- Pozisyon güncelleniyor
    pos.x = pos.x + vel.x * dt
    pos.y = pos.y + vel.y * dt
    pos.z = pos.z + vel.z * dt
    
    -- Eğer yağmur damlası yere (veya belirli bir Y eksenine) çarparsa
    -- Tekrardan yukarıda rastgele bir konuma taşı (Pooling/Looping)
    if pos.y < ground_y then
        pos.y = reset_height + (math.random() * 10.0) -- 25-35 arası bir yüksekliğe taşı
        
        -- Damlanın düşeceği alanı belirle (örneğin -20 ile 20 arası X/Z düzlemi)
        local spread = 40.0
        pos.x = pos.x + ((math.random() - 0.5) * spread)
        pos.z = pos.z + ((math.random() - 0.5) * spread)
        
        -- Hızı sıfırla, tekrar yavaş yavaş hızlansın
        vel.y = -5.0
        vel.x = 0.0
        vel.z = 0.0
    end
    
    return {
        position = pos,
        velocity = vel
    }
end
