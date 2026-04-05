-- player.lua
-- Basit bir hareket scripti

function on_update(ctx)
    -- dt = delta time
    -- pos = {x, y, z}
    -- vel = {x, y, z}
    -- input = {w, a, s, d, space}
    
    local pos = ctx.position
    local vel = ctx.velocity
    local dt = ctx.dt
    local speed = 10.0
    
    local dir_x = 0.0
    local dir_z = 0.0
    
    if ctx.input.w then dir_z = 1.0 end
    if ctx.input.s then dir_z = -1.0 end
    if ctx.input.a then dir_x = -1.0 end
    if ctx.input.d then dir_x = 1.0 end
    if ctx.input.space then 
        vel.y = 5.0 
    end
    
    vel.x = dir_x * speed
    vel.z = dir_z * speed

    -- İsterseniz konsola bilgi yazdırabilirsiniz
    -- print_engine("Lua: Obje " .. tostring(ctx.entity_id) .. " Guncellendi. X: " .. tostring(vel.x))
    
    return {
        velocity = vel,
        position = pos
    }
end
