-- car_controller.lua
-- Oyuncunun YÖN TUŞLARI (Ok Tuşları) ile arabayı hareket ettirmesini sağlar.
-- WASD artık serbest kamera/karakter için boştur.

function car_update(ctx)
    local vel = ctx.velocity
    local speed = 30.0

    -- Yarış başlamadan (İlk 4 saniye) aracı kitle!
    if ctx.elapsed < 4.0 then
        vel.x = 0.0
        vel.y = 0.0
        vel.z = 0.0
        return { velocity = vel }
    end

    -- İleri / Geri (Up / Down ok tuşları)
    if ctx.input.up then
        vel.z = -speed
    elseif ctx.input.down then
        vel.z = speed
    else
        vel.z = 0.0
    end
    
    -- Sağa / Sola (Right / Left ok tuşları)
    if ctx.input.left then
        vel.x = -speed
    elseif ctx.input.right then
        vel.x = speed
    else
        vel.x = 0.0
    end
    
    return {
        velocity = vel
    }
end
