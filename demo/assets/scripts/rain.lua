-- Yağmur (Rain) Efekti Serbest Scripti
-- Bu script bir entity'nin hızlıca düşen yağmur damlası gibi davranmasını sağlar.

local reset_height = 40.0
local fall_speed = 35.0
local ground_y = -5.0

function on_update(ctx)
    local pos = ctx.position
    local vel = ctx.velocity
    local dt = ctx.dt
    
    -- Hızlandırılmış yer çekimi (Yağmur hızlı düşer)
    vel.y = vel.y - (30.0 * dt)
    
    if vel.y < -fall_speed then
        vel.y = -fall_speed
    end
    
    -- Fizik motoru (RigidBody) olmadığı için biz hareket ettiriyoruz
    pos.x = pos.x + vel.x * dt
    pos.y = pos.y + vel.y * dt
    pos.z = pos.z + vel.z * dt
    
    -- Yere çapma anında yukarıdan, kameranın merkezine yakın bir spread at
    if pos.y < ground_y then
        pos.y = reset_height + (math.random() * 20.0)
        
        local spread = 60.0
        pos.x = ((math.random() - 0.5) * spread)
        pos.z = ((math.random() - 0.5) * spread)
        
        vel.y = -15.0 - (math.random() * 10.0)
        vel.x = 0.0
        vel.z = 0.0
    end
    
    return {
        position = pos,
        velocity = vel
    }
end
