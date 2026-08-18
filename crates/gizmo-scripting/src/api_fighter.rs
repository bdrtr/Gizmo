//! The fighter API — the fighting-system functions exposed to Lua.
//!
//! Two halves. The **write** half queues commands: start a move, apply hitstop, apply hitstun.
//! The **read** half is a per-frame mirror of every `FighterController` in the world
//! ([`update_fighter_read_api`]) — health, stance, the move in flight with the frame it is on,
//! and the counters the engine's fight clock spends — plus the input-buffer history combo
//! recognition walks.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::World;
use mlua::prelude::*;
use std::sync::Arc;

/// Registers the fighter API functions with Lua.
pub fn register_fighter_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    crate::api_table::register_protected(lua, "fighter", |fighter_table| {

    // The engine writes these every frame (`update_fighter_read_api`); Lua reads them through the
    // helpers below. Same shape as `entity._positions` and friends: `table[entity_id] = value`.
    fighter_table.raw_set("_buffers", lua.create_table()?)?;
    fighter_table.raw_set("_state", lua.create_table()?)?;

    // === SET FIGHTER MOVE ===
    //
    // `hitstun` and `hitstop` are optional trailing arguments: a move that does not name them
    // keeps the 20/5 every Lua-authored move used to be stuck with, so calls written before they
    // existed behave identically. They are the move's own numbers — what it inflicts when it
    // lands — and the script authoring the move is the only thing that knows them.
    {
        let cq = command_queue.clone();
        fighter_table.raw_set(
            "set_move",
            lua.create_function(
                move |_,
                      (id, name, startup, active, recovery, damage, hitstun, hitstop): (
                    u32,
                    String,
                    u32,
                    u32,
                    u32,
                    f32,
                    Option<u32>,
                    Option<u32>,
                )| {
                    let defaults = gizmo_physics_core::components::fighter::FrameData::default();
                    cq.push(ScriptCommand::SetFighterMove {
                        id,
                        name,
                        startup,
                        active,
                        recovery,
                        damage,
                        hitstun: hitstun.unwrap_or(defaults.hitstun),
                        hitstop: hitstop.unwrap_or(defaults.hitstop),
                    });
                    Ok(())
                },
            )?,
        )?;
    }

    // === APPLY HITSTOP ===
    {
        let cq = command_queue.clone();
        fighter_table.raw_set(
            "apply_hitstop",
            lua.create_function(move |_, (id, frames): (u32, u32)| {
                cq.push(ScriptCommand::ApplyHitstop(id, frames));
                Ok(())
            })?,
        )?;
    }

    // === APPLY HITSTUN ===
    {
        let cq = command_queue.clone();
        fighter_table.raw_set(
            "apply_hitstun",
            lua.create_function(move |_, (id, frames): (u32, u32)| {
                cq.push(ScriptCommand::ApplyHitstun(id, frames));
                Ok(())
            })?,
        )?;
    }


    // Lua tarafında kombo kontrol eden yardımcı fonksiyon
    lua.load(
        r#"
        function fighter.state(id)
            return fighter._state[id]
        end

        function fighter.is_locked(id)
            local s = fighter._state[id]
            return s ~= nil and s.locked or false
        end

        -- Is this fighter's move inside its hitting window right now? The one question frame data
        -- exists to answer, and until the engine had a fight clock it could never be `true`.
        function fighter.is_attacking(id)
            local s = fighter._state[id]
            return s ~= nil and s.move ~= nil and s.move.attacking or false
        end

        -- How far through its move, as `frame, total`; `nil` when the fighter is neutral.
        function fighter.move_frame(id)
            local s = fighter._state[id]
            if s == nil or s.move == nil then return nil end
            return s.move.frame, s.move.total
        end

        function fighter.check_combo(id, combo, max_gap)
            local buffer = fighter._buffers[id]
            if not buffer then return false end

            local combo_idx = #combo
            if combo_idx == 0 then return false end

            local gap_counter = 0
            
            for i = 1, #buffer do
                local frame = buffer[i]
                local target_input = combo[combo_idx]

                if frame.just_pressed[target_input] then
                    combo_idx = combo_idx - 1
                    gap_counter = 0
                    if combo_idx == 0 then
                        return true
                    end
                elseif gap_counter >= max_gap then
                    return false
                else
                    gap_counter = gap_counter + 1
                end
            end
            
            return false
        end
    "#,
    )
    .exec()?;

        Ok(())
    })
}

#[tracing::instrument(skip_all, name = "script_fighter_read")]
/// Mirrors every fighter's state — the whole component, not a boolean — into Lua, once a frame.
///
/// **What this used to hand across, and why that was not enough.** The only fighter fact a script
/// could read was `is_locked`'s boolean. Not the health it was fighting over, not the move in
/// flight, and above all not `current_move_frame` — so a script could start a move and then had
/// no way to learn what frame it was on, whether it was hitting, or when it ended. The whole
/// reason frame data exists is to be *read* on the frame it matters, and the read side of this API
/// was one bit wide.
///
/// The shape follows `entity._positions` and its neighbours: `table[entity_id] = value`, rebuilt
/// each frame from the world, read through the Lua helpers registered next to it.
///
/// ```lua
/// local s = fighter.state(id)          -- nil if that entity is not a fighter
/// s.health, s.max_health               -- numbers
/// s.player_id, s.blocking, s.crouching
/// s.hitstop, s.hitstun, s.locked       -- the counters the fight clock spends, and their sum
/// s.move                               -- nil when neutral, otherwise:
///   s.move.name, s.move.frame, s.move.total
///   s.move.startup, s.move.active, s.move.recovery
///   s.move.attacking                   -- is_in_active_window(): hitting on THIS frame
///   s.move.damage, s.move.hitstun_on_hit, s.move.hitstop_on_hit
/// ```
///
/// **The snapshot is one frame old by the time a per-entity hook runs**, and deliberately so: the
/// scripting pass mirrors, then scripts run, then `PlayLoop` spends the fixed steps that advance
/// these numbers. So `s.move.frame` is where the move stood when this frame began — which is the
/// value a script reacting to it should be looking at anyway.
pub fn update_fighter_read_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    // The real table, not the global: the global is a read-only proxy so a script cannot
    // rewrite the API (see `api_table`), and the engine's per-frame writes go behind it.
    let fighter_table = crate::api_table::raw(lua, "fighter")?;

    let buffers = lua.create_table()?;
    let states = lua.create_table()?;

    let controllers = world.borrow::<gizmo_physics_core::components::FighterController>();
    for (eid, _) in controllers.iter() {
        if let Some(fighter) = controllers.get(eid) {
            let state = lua.create_table()?;
            state.set("player_id", fighter.player_id)?;
            state.set("health", fighter.health)?;
            state.set("max_health", fighter.max_health)?;
            state.set("blocking", fighter.is_blocking)?;
            state.set("crouching", fighter.is_crouching)?;
            state.set("hitstop", fighter.hitstop_frames)?;
            state.set("hitstun", fighter.hitstun_frames)?;
            state.set("locked", fighter.is_locked())?;

            if let Some(active) = &fighter.active_move {
                let fd = &active.frame_data;
                let move_table = lua.create_table()?;
                move_table.set("name", active.name.clone())?;
                move_table.set("frame", fighter.current_move_frame)?;
                move_table.set("total", fd.total_frames())?;
                move_table.set("startup", fd.startup)?;
                move_table.set("active", fd.active)?;
                move_table.set("recovery", fd.recovery)?;
                move_table.set("damage", fd.damage)?;
                // Named apart from `state.hitstun`/`state.hitstop`: these are what the move
                // INFLICTS when it lands, not what this fighter is currently serving.
                move_table.set("hitstun_on_hit", fd.hitstun)?;
                move_table.set("hitstop_on_hit", fd.hitstop)?;
                move_table.set("attacking", fighter.is_in_active_window())?;
                state.set("move", move_table)?;
            }

            states.set(eid, state)?;

            let frames_table = lua.create_table()?;
            for (i, frame) in fighter.input_buffer.frames.iter().enumerate() {
                let frame_table = lua.create_table()?;

                let jp_table = lua.create_table()?;
                for k in &frame.just_pressed {
                    jp_table.set(k.clone(), true)?;
                }

                let p_table = lua.create_table()?;
                for k in &frame.pressed {
                    p_table.set(k.clone(), true)?;
                }

                // The release edge was dropped on the floor here, which made charge moves,
                // negative-edge specials and hold-and-release inputs — a whole class of fighting
                // game move — invisible to Lua even with a buffer someone had filled.
                let jr_table = lua.create_table()?;
                for k in &frame.just_released {
                    jr_table.set(k.clone(), true)?;
                }

                frame_table.set("just_pressed", jp_table)?;
                frame_table.set("pressed", p_table)?;
                frame_table.set("just_released", jr_table)?;

                frames_table.set(i + 1, frame_table)?;
            }
            buffers.set(eid, frames_table)?;
        }
    }

    fighter_table.raw_set("_buffers", buffers)?;
    fighter_table.raw_set("_state", states)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandQueue;

    /// A helper that builds a buffer in Lua marking `input` as just_pressed at the given frame
    /// indices, and returns check_combo's result.
    fn run_combo(setup: &str) -> bool {
        let lua = Lua::new();
        register_fighter_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        lua.load(setup).exec().unwrap();
        lua.load("return fighter.check_combo(1, combo, max_gap)")
            .eval()
            .unwrap()
    }

    /// Regression: max_gap=2 must allow a gap of exactly 2 frames.
    /// After 'b' is found, 2 empty frames + 'a' => accepted.
    /// 3 empty frames => rejected.
    #[test]
    fn combo_gap_boundary_is_exact() {
        // Buffer ileri taranır; önce combo'nun son elemanı ('b') aranır.
        // frame1='b', frame2/3 boş, frame4='a' -> 2 boşluk, kabul edilmeli.
        let accepted = run_combo(
            r#"
            combo = { "a", "b" }
            max_gap = 2
            local function f(k) return { just_pressed = k and { [k] = true } or {} } end
            fighter._buffers[1] = { f("b"), f(nil), f(nil), f("a") }
            "#,
        );
        assert!(accepted, "2 frame boşluk max_gap=2 için kabul edilmeli");

        // frame1='b', frame2/3/4 boş, frame5='a' -> 3 boşluk, reddedilmeli.
        let rejected = run_combo(
            r#"
            combo = { "a", "b" }
            max_gap = 2
            local function f(k) return { just_pressed = k and { [k] = true } or {} } end
            fighter._buffers[1] = { f("b"), f(nil), f(nil), f(nil), f("a") }
            "#,
        );
        assert!(
            !rejected,
            "3 frame boşluk max_gap=2 için REDDEDİLMELİ (off-by-one)"
        );
    }

    /// A complete adjacent (gapless) combo must be recognised; a combo is scanned backwards,
    /// last element first.
    #[test]
    fn adjacent_full_combo_is_recognized() {
        let accepted = run_combo(
            r#"
            combo = { "a", "b", "c" }
            max_gap = 1
            local function f(k) return { just_pressed = { [k] = true } } end
            -- Buffer ileri taranır, önce combo'nun sonu ("c") aranır: c, b, a
            fighter._buffers[1] = { f("c"), f("b"), f("a") }
            "#,
        );
        assert!(accepted, "bitişik c-b-a dizisi a,b,c kombosunu tamamlamalı");
    }

    /// An empty combo list must never match (the combo_idx == 0 early exit).
    #[test]
    fn empty_combo_never_matches() {
        let matched = run_combo(
            r#"
            combo = {}
            max_gap = 5
            local function f(k) return { just_pressed = { [k] = true } } end
            fighter._buffers[1] = { f("a"), f("b") }
            "#,
        );
        assert!(!matched, "boş kombo false dönmeli");
    }

    /// With no buffer for the entity, check_combo must safely return false (the nil-buffer
    /// guard).
    #[test]
    fn missing_buffer_returns_false() {
        let matched = run_combo(
            r#"
            combo = { "a" }
            max_gap = 5
            -- fighter._buffers[1] hiç ayarlanmadı
            "#,
        );
        assert!(!matched, "buffer yoksa false dönmeli");
    }

    /// If the combo is not completed (only the last element is present, not the first) it must
    /// return false.
    #[test]
    fn partially_matched_combo_is_rejected() {
        let matched = run_combo(
            r#"
            combo = { "a", "b" }
            max_gap = 5
            local function f(k) return { just_pressed = { [k] = true } } end
            -- Sadece "b" var, "a" hiç basılmadı → kombo tamamlanmaz.
            fighter._buffers[1] = { f("b"), f("x"), f("y") }
            "#,
        );
        assert!(!matched, "eksik kombo tamamlanmamış sayılmalı");
    }

    /// The read helpers answer safely for an entity that is not a fighter at all — `false`, `nil`,
    /// never an error. A script asking about the wrong id is the common case, not an exception.
    #[test]
    fn the_read_helpers_have_safe_answers_for_a_missing_fighter() {
        let lua = Lua::new();
        register_fighter_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        lua.load(
            r#"
            assert(fighter.state(1) == nil, "dövüşçü yoksa state nil")
            assert(fighter.is_locked(1) == false, "giriş yoksa varsayılan false")
            assert(fighter.is_attacking(1) == false, "giriş yoksa saldırmıyor")
            assert(fighter.move_frame(1) == nil, "giriş yoksa kare nil")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// The helpers read `_state`, and `false` in the table must come back as `false` rather than
    /// being swallowed by Lua's `and`/`or` idiom.
    #[test]
    fn the_read_helpers_read_the_state_table() {
        let lua = Lua::new();
        register_fighter_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        lua.load(
            r#"
            fighter._state[1] = { locked = false, move = { frame = 4, total = 10, attacking = false } }
            assert(fighter.is_locked(1) == false, "locked=false false dönmeli")
            assert(fighter.is_attacking(1) == false, "attacking=false false dönmeli")
            local f, t = fighter.move_frame(1)
            assert(f == 4 and t == 10, "kare ve toplam okunmalı")

            fighter._state[2] = { locked = true, move = { frame = 6, total = 10, attacking = true } }
            assert(fighter.is_locked(2) == true, "locked=true true dönmeli")
            assert(fighter.is_attacking(2) == true, "attacking=true true dönmeli")

            fighter._state[3] = { locked = false }
            assert(fighter.is_attacking(3) == false, "hareketi olmayan saldırmıyor")
            assert(fighter.move_frame(3) == nil, "hareketi olmayanın karesi nil")
            "#,
        )
        .exec()
        .unwrap();
    }

    /// **The mirror carries the whole component, off a real world.**
    ///
    /// The read side used to hand Lua one boolean, so a script could start a move and never learn
    /// what frame it was on — the thing frame data exists for. This drives the same function the
    /// engine calls every frame and reads the numbers back out through the Lua helpers.
    #[test]
    fn the_mirror_carries_the_move_in_flight_and_its_counters() {
        use gizmo_physics_core::components::fighter::{CombatMove, FighterController, FrameData};

        let mut world = World::new();
        let e = world.spawn();

        let mut frame_data = FrameData::default();
        frame_data.startup = 5;
        frame_data.active = 3;
        frame_data.recovery = 2;
        frame_data.damage = 8.0;
        frame_data.hitstun = 20;
        frame_data.hitstop = 5;
        let mut combat_move = CombatMove::default();
        combat_move.name = "Jab".to_string();
        combat_move.frame_data = frame_data;

        let mut fighter = FighterController::new(2);
        fighter.health = 73.5;
        fighter.is_crouching = true;
        fighter.active_move = Some(combat_move);
        fighter.current_move_frame = 6; // inside the 5..8 window
        fighter.apply_hitstop(4);
        world.add_component(e, fighter);

        let lua = Lua::new();
        register_fighter_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        update_fighter_read_api(&lua, &world).unwrap();

        lua.load(format!(
            r#"
            local s = fighter.state({id})
            assert(s ~= nil, "dövüşçü aynada yok")
            assert(s.player_id == 2, "player_id")
            assert(math.abs(s.health - 73.5) < 0.001, "health")
            assert(s.max_health == 100.0, "max_health — HUD barının paydası")
            assert(s.crouching == true and s.blocking == false, "duruş")
            assert(s.hitstop == 4 and s.hitstun == 0, "sayaçlar")
            assert(s.locked == true, "hitstop varken kilitli")
            assert(s.move ~= nil, "uçuşta bir hareket var")
            assert(s.move.name == "Jab", "hareket adı")
            assert(s.move.frame == 6 and s.move.total == 10, "kare 6/10")
            assert(s.move.startup == 5 and s.move.active == 3 and s.move.recovery == 2, "fazlar")
            assert(s.move.attacking == true, "6. kare vuruş penceresinin içinde")
            assert(s.move.hitstun_on_hit == 20 and s.move.hitstop_on_hit == 5,
                   "hareketin İSABET ETTİĞİNDE dayattığı süreler, dövüşçünün şu anki sayaçları değil")
            assert(fighter.is_locked({id}) == true and fighter.is_attacking({id}) == true, "yardımcılar")
            "#,
            id = e.id()
        ))
        .exec()
        .unwrap();
    }

    /// A neutral fighter mirrors with no `move` at all, rather than a stale one.
    #[test]
    fn a_neutral_fighter_mirrors_without_a_move() {
        use gizmo_physics_core::components::fighter::FighterController;

        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, FighterController::new(1));

        let lua = Lua::new();
        register_fighter_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        update_fighter_read_api(&lua, &world).unwrap();

        lua.load(format!(
            r#"
            local s = fighter.state({id})
            assert(s ~= nil and s.move == nil, "nötr dövüşçünün hareketi olmamalı")
            assert(fighter.is_attacking({id}) == false, "ve saldırmıyor")
            assert(fighter.move_frame({id}) == nil, "ve karesi yok")
            "#,
            id = e.id()
        ))
        .exec()
        .unwrap();
    }

    /// The input-buffer mirror carries the RELEASE edge too. Without it a charge move — hold, then
    /// let go — is invisible to Lua however carefully the buffer is filled.
    #[test]
    fn the_buffer_mirror_carries_the_release_edge() {
        use gizmo_core::input::FrameActions;
        use gizmo_physics_core::components::fighter::FighterController;
        use std::collections::HashSet;

        let mut world = World::new();
        let e = world.spawn();
        let mut fighter = FighterController::new(1);
        fighter.input_buffer.frames.push_front(FrameActions {
            pressed: HashSet::new(),
            just_pressed: HashSet::new(),
            just_released: HashSet::from(["Back".to_string()]),
        });
        world.add_component(e, fighter);

        let lua = Lua::new();
        register_fighter_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        update_fighter_read_api(&lua, &world).unwrap();

        lua.load(format!(
            r#"
            local frame = fighter._buffers[{id}][1]
            assert(frame.just_released ~= nil, "bırakma kenarı aynada olmalı")
            assert(frame.just_released["Back"] == true, "bırakılan tuş okunmalı")
            "#,
            id = e.id()
        ))
        .exec()
        .unwrap();
    }

    /// set_move / apply_hitstop / apply_hitstun must queue the right commands, frame data
    /// included.
    #[test]
    fn fighter_write_calls_push_expected_commands() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_fighter_api(&lua, cq.clone()).unwrap();

        lua.load(
            r#"
            fighter.set_move(1, "jab", 3, 2, 8, 5.5)
            fighter.apply_hitstop(1, 6)
            fighter.apply_hitstun(2, 20)
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 3);
        match &cmds[0] {
            ScriptCommand::SetFighterMove { id, name, startup, active, recovery, damage, hitstun, hitstop } => {
                assert_eq!(*id, 1);
                assert_eq!(name, "jab");
                assert_eq!((*startup, *active, *recovery), (3, 2, 8));
                assert!((damage - 5.5).abs() < 1e-6);
                assert_eq!(
                    (*hitstun, *hitstop),
                    (20, 5),
                    "a call that names neither must keep the frame-data defaults"
                );
            }
            other => panic!("beklenen SetFighterMove, gelen {other:?}"),
        }
        assert!(matches!(cmds[1], ScriptCommand::ApplyHitstop(1, 6)));
        assert!(matches!(cmds[2], ScriptCommand::ApplyHitstun(2, 20)));
    }

    // ── One concept, two implementations ────────────────────────────────────────────────────
    //
    // Combo recognition exists twice: `FighterInputBuffer::check_combo_strict` in gizmo-core, and
    // the Lua `fighter.check_combo` above, which walks the mirrored table because a script can
    // fill that table itself (the engine never fills the component's buffer). Two implementations
    // of one named concept is exactly the arrangement that drifts, and it had: the Rust one
    // matched `just_pressed || pressed` while Lua matched only `just_pressed`, and every combo
    // test in the workspace drove one or the other, never both, so the divergence was invisible.
    //
    // These drive the same scenarios through both and assert the same answer.

    /// One recorded frame: what went down this frame, and what is merely still held.
    type Frame<'a> = (&'a [&'a str], &'a [&'a str]);

    /// Runs `scenario` (oldest frame first) through the Rust implementation.
    fn rust_answer(scenario: &[Frame<'_>], combo: &[&str], max_gap: usize) -> bool {
        use gizmo_core::input::{FighterInputBuffer, FrameActions};
        use std::collections::HashSet;

        let mut buffer = FighterInputBuffer::new(60);
        for (just_pressed, held) in scenario {
            let mut pressed: HashSet<String> = just_pressed.iter().map(|s| s.to_string()).collect();
            pressed.extend(held.iter().map(|s| s.to_string()));
            buffer.frames.push_front(FrameActions {
                pressed,
                just_pressed: just_pressed.iter().map(|s| s.to_string()).collect(),
                just_released: HashSet::new(),
            });
        }
        buffer.check_combo_strict(combo, max_gap)
    }

    /// Runs the same scenario through the Lua implementation, via the mirrored table.
    fn lua_answer(scenario: &[Frame<'_>], combo: &[&str], max_gap: usize) -> bool {
        let lua = Lua::new();
        register_fighter_api(&lua, Arc::new(CommandQueue::new())).unwrap();

        // The Lua buffer is newest-first, so the chronological scenario is written backwards.
        let frames: Vec<String> = scenario
            .iter()
            .rev()
            .map(|(just_pressed, held)| {
                let jp: Vec<String> = just_pressed
                    .iter()
                    .map(|k| format!("[\"{k}\"]=true"))
                    .collect();
                let mut p = jp.clone();
                p.extend(held.iter().map(|k| format!("[\"{k}\"]=true")));
                format!(
                    "{{ just_pressed = {{{}}}, pressed = {{{}}} }}",
                    jp.join(","),
                    p.join(",")
                )
            })
            .collect();
        let combo_literal: Vec<String> = combo.iter().map(|c| format!("\"{c}\"")).collect();

        lua.load(format!(
            "fighter._buffers[1] = {{{}}}\nreturn fighter.check_combo(1, {{{}}}, {})",
            frames.join(","),
            combo_literal.join(","),
            max_gap
        ))
        .eval()
        .unwrap()
    }

    /// **Both implementations must answer the same question the same way.**
    ///
    /// The scenarios are the ones a fighting game actually produces: a quarter circle entered as
    /// press edges, the same inputs merely held down, the reverse order, and the gap tolerance at
    /// its exact boundary and one frame past it.
    #[test]
    fn the_rust_and_lua_combo_checks_agree() {
        let qcf: &[Frame<'_>] = &[
            (&["Down"], &[]),
            (&["Right"], &["Down"]),
            (&[], &["Right"]),
            (&["LightPunch"], &[]),
        ];
        let held: &[Frame<'_>] = &[
            (&[], &["Down", "Right", "LightPunch"]),
            (&[], &["Down", "Right", "LightPunch"]),
            (&[], &["Down", "Right", "LightPunch"]),
            (&[], &["Down", "Right", "LightPunch"]),
        ];
        let gap_exact: &[Frame<'_>] = &[
            (&["Down"], &[]),
            (&[], &[]),
            (&[], &[]),
            (&["LightPunch"], &[]),
        ];
        let gap_over: &[Frame<'_>] = &[
            (&["Down"], &[]),
            (&[], &[]),
            (&[], &[]),
            (&[], &[]),
            (&["LightPunch"], &[]),
        ];

        let cases: &[(&str, &[Frame<'_>], &[&str], usize, bool)] = &[
            ("çeyrek daire, basma kenarlarıyla", qcf, &["Down", "Right", "LightPunch"], 5, true),
            ("aynı girdiler ters sırada", qcf, &["LightPunch", "Right", "Down"], 5, false),
            ("üçünü birlikte TUTMAK", held, &["Down", "Right", "LightPunch"], 5, false),
            ("tutulanların ters sırası", held, &["LightPunch", "Right", "Down"], 5, false),
            ("tam sınırda boşluk", gap_exact, &["Down", "LightPunch"], 2, true),
            ("sınırın bir üstünde boşluk", gap_over, &["Down", "LightPunch"], 2, false),
            ("boş kombo", qcf, &[], 5, false),
        ];

        for (name, scenario, combo, max_gap, expected) in cases {
            let rust = rust_answer(scenario, combo, *max_gap);
            let lua = lua_answer(scenario, combo, *max_gap);
            assert_eq!(
                rust, lua,
                "{name}: Rust {rust} dedi, Lua {lua} dedi — bir kavramın iki uygulaması ayrışmış"
            );
            assert_eq!(
                rust, *expected,
                "{name}: beklenen {expected}, ikisi de {rust} dedi"
            );
        }
    }
}
