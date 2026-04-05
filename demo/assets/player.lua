-- player.lua
-- Basit bir hareket scripti — Yelbegen Engine Lua Scripting

frame_count = 0

function on_update(ctx)
    frame_count = frame_count + 1

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

    -- Her 120 frame'de durumu konsola yaz
    if frame_count % 120 == 0 then
        print_engine("Frame #" .. tostring(frame_count) .. " | Entity: " .. tostring(ctx.entity_id) .. " | Pos: (" .. string.format("%.1f", pos.x) .. ", " .. string.format("%.1f", pos.y) .. ", " .. string.format("%.1f", pos.z) .. ")")
    end
    
    return {
        velocity = vel,
        position = pos
    }
end
