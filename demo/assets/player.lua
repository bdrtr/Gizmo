-- player.lua
-- Serbest Dolaşım (Fly Camera) Scripti

function on_update(ctx)
    local pos = ctx.position
    local vel = ctx.velocity
    local dt = ctx.dt
    local speed = 25.0
    
    local dir_x = 0.0
    local dir_y = 0.0
    local dir_z = 0.0
    
    -- Kamera eksenine göre yerel dolaşım yapmıyor, global WASD olarak çalışıyor.
    -- X ve Z ekseninde hareket
    if ctx.input.w then dir_z = -1.0 end
    if ctx.input.s then dir_z = 1.0 end
    if ctx.input.a then dir_x = -1.0 end
    if ctx.input.d then dir_x = 1.0 end
    if ctx.input.space then dir_y = 1.0 end
    
    -- Pozisyonu doğrudan manuel güncelle (Fizik motorunu bypass et)
    pos.x = pos.x + (dir_x * speed * dt)
    pos.y = pos.y + (dir_y * speed * dt)
    pos.z = pos.z + (dir_z * speed * dt)
    
    return {
        velocity = vel,
        position = pos
    }
end
