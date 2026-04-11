-- Yelbegen Engine — Örnek Lua Scripti
-- Bu script bir entity'nin her frame güncellenmesini kontrol eder

local time = 0

function on_update(ctx)
    time = time + ctx.dt
    
    local pos = ctx.position
    local vel = ctx.velocity
    
    -- WASD girişlerine göre hareket
    local speed = 5.0
    
    if ctx.input.w then vel.z = vel.z - speed * ctx.dt end
    if ctx.input.s then vel.z = vel.z + speed * ctx.dt end
    if ctx.input.a then vel.x = vel.x - speed * ctx.dt end
    if ctx.input.d then vel.x = vel.x + speed * ctx.dt end
    
    -- Zıplama
    if ctx.input.space and pos.y <= 0.5 then
        vel.y = 5.0
    end
    
    -- Objeyi sinüs dalgasıyla yukarı aşağı hareket ettir (Idle animasyonu)
    pos.y = pos.y + math.sin(time * 2.0) * 0.5 * ctx.dt
    
    return {
        position = pos,
        velocity = vel
    }
end
