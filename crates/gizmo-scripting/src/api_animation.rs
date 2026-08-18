//! The animation API — skeletal playback exposed to Lua.
//!
//! Two halves, like the fighter API. The **write** half queues
//! [`ScriptCommand::PlayAnimation`] and [`ScriptCommand::SetAnimationSpeed`]; the **read** half
//! mirrors each entity's [`AnimationPlayer`](gizmo_animation::skeletal::AnimationPlayer) once a
//! frame so a script can ask which clip is running and how far through it is.
//!
//! **Both commands already had working handlers and no producer.** `flush_commands` has applied
//! `PlayAnimation` and `SetAnimationSpeed` to real `AnimationPlayer`s for as long as they have
//! existed, and nothing anywhere pushed either one — no Lua binding, no Rust caller, no test. The
//! engine could animate on request and had no way to be asked. `gizmo-animation`'s own
//! documentation names "the Lua `PlayAnimation` command" while explaining why a failed clip lookup
//! is logged, which is how far the assumption had spread.

use crate::commands::{CommandQueue, ScriptCommand};
use gizmo_core::World;
use mlua::prelude::*;
use std::sync::Arc;

/// Cross-fade used when a script does not name one, in seconds.
///
/// Not zero: a clip switch with no blend is a visible pop, and a script saying `animation.play(id,
/// "run")` is asking for the ordinary thing. Name a blend to override it, `0` included.
const DEFAULT_BLEND: f32 = 0.2;

/// Registers the animation API functions with Lua.
pub fn register_animation_api(lua: &Lua, command_queue: Arc<CommandQueue>) -> Result<(), LuaError> {
    crate::api_table::register_protected(lua, "animation", |animation_table| {
        // The engine writes this every frame (`update_animation_read_api`); Lua reads it through
        // the helpers below. Same shape as `entity._positions`: `table[entity_id] = value`.
        animation_table.raw_set("_state", lua.create_table()?)?;

        // === PLAY ===
        //
        // `blend` and `loop_anim` are optional: `animation.play(id, "run")` cross-fades over
        // DEFAULT_BLEND and loops, which is what a locomotion clip wants. A one-shot is
        // `animation.play(id, "punch", 0.1, false)`.
        {
            let cq = command_queue.clone();
            animation_table.raw_set(
                "play",
                lua.create_function(
                    move |_,
                          (id, name, blend, loop_anim): (
                        u32,
                        String,
                        Option<f32>,
                        Option<bool>,
                    )| {
                        cq.push(ScriptCommand::PlayAnimation {
                            id,
                            name,
                            blend: blend.unwrap_or(DEFAULT_BLEND),
                            loop_anim: loop_anim.unwrap_or(true),
                        });
                        Ok(())
                    },
                )?,
            )?;
        }

        // === SPEED ===
        {
            let cq = command_queue.clone();
            animation_table.raw_set(
                "set_speed",
                lua.create_function(move |_, (id, speed): (u32, f32)| {
                    cq.push(ScriptCommand::SetAnimationSpeed(id, speed));
                    Ok(())
                })?,
            )?;
        }

        lua.load(
            r#"
            function animation.state(id)
                return animation._state[id]
            end

            -- The clip playing right now, or nil if the entity has no player (or its clip index
            -- has been put out of range, which the engine treats as a supported state).
            function animation.clip(id)
                local s = animation._state[id]
                if s == nil then return nil end
                return s.clip
            end

            -- Is `name` the clip currently playing? The question a script asks before switching,
            -- since re-selecting the active clip is deliberately a no-op in the engine.
            function animation.is_playing(id, name)
                local s = animation._state[id]
                return s ~= nil and s.clip == name or false
            end
        "#,
        )
        .exec()?;

        Ok(())
    })
}

#[tracing::instrument(skip_all, name = "script_animation_read")]
/// Mirrors every [`AnimationPlayer`](gizmo_animation::skeletal::AnimationPlayer) into Lua, once a
/// frame.
///
/// ```lua
/// local s = animation.state(id)   -- nil if that entity has no player
/// s.clip        -- name of the clip being sampled, nil if the index is out of range
/// s.time        -- playhead in seconds
/// s.duration    -- the clip's length in seconds, nil with no clip
/// s.speed       -- playback multiplier; 0 is frozen, negative runs backwards
/// s.looping     -- whether the active clip wraps
/// s.clips       -- every clip name this player can select, in order
/// ```
///
/// `s.clips` is what makes a script able to *choose*: a clip name that does not exist is ignored by
/// the engine with a warning, so a script that wants to be robust checks the list rather than
/// hoping.
pub fn update_animation_read_api(lua: &Lua, world: &World) -> Result<(), LuaError> {
    // The real table, not the global: the global is a read-only proxy so a script cannot rewrite
    // the API (see `api_table`), and the engine's per-frame writes go behind it.
    let animation_table = crate::api_table::raw(lua, "animation")?;

    let states = lua.create_table()?;
    let players = world.borrow::<gizmo_animation::skeletal::AnimationPlayer>();
    for (eid, _) in players.iter() {
        if let Some(player) = players.get(eid) {
            let state = lua.create_table()?;
            state.set("time", player.current_time)?;
            state.set("speed", player.speed)?;
            state.set("looping", player.loop_anim)?;
            if let Some(clip) = player.current_clip() {
                state.set("clip", clip.name.clone())?;
                state.set("duration", clip.duration)?;
            }

            let clips = lua.create_table()?;
            for (i, clip) in player.animations.iter().enumerate() {
                clips.set(i + 1, clip.name.clone())?;
            }
            state.set("clips", clips)?;

            states.set(eid, state)?;
        }
    }

    animation_table.raw_set("_state", states)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandQueue;

    /// `animation.play` fills in the ordinary case — a cross-fade and a loop — and takes an
    /// override for either. A one-shot attack clip is the reason the override exists.
    #[test]
    fn play_defaults_to_a_looping_cross_fade_and_takes_overrides() {
        let lua = Lua::new();
        let cq = Arc::new(CommandQueue::new());
        register_animation_api(&lua, cq.clone()).unwrap();

        lua.load(
            r#"
            animation.play(1, "run")
            animation.play(2, "punch", 0.05, false)
            animation.set_speed(1, 1.5)
            "#,
        )
        .exec()
        .unwrap();

        let cmds = cq.drain();
        assert_eq!(cmds.len(), 3);
        match &cmds[0] {
            ScriptCommand::PlayAnimation { id, name, blend, loop_anim } => {
                assert_eq!((*id, name.as_str()), (1, "run"));
                assert!((blend - DEFAULT_BLEND).abs() < 1e-6, "unnamed blend is the default");
                assert!(*loop_anim, "a clip a script just names is a looping one");
            }
            other => panic!("beklenen PlayAnimation, gelen {other:?}"),
        }
        match &cmds[1] {
            ScriptCommand::PlayAnimation { id, name, blend, loop_anim } => {
                assert_eq!((*id, name.as_str()), (2, "punch"));
                assert!((blend - 0.05).abs() < 1e-6);
                assert!(!*loop_anim, "a one-shot must stay a one-shot");
            }
            other => panic!("beklenen PlayAnimation, gelen {other:?}"),
        }
        assert!(matches!(cmds[2], ScriptCommand::SetAnimationSpeed(1, s) if (s - 1.5).abs() < 1e-6));
    }

    /// The read helpers answer safely for an entity with no player: `nil`, `nil`, `false`.
    #[test]
    fn the_read_helpers_have_safe_answers_for_a_missing_player() {
        let lua = Lua::new();
        register_animation_api(&lua, Arc::new(CommandQueue::new())).unwrap();
        lua.load(
            r#"
            assert(animation.state(1) == nil, "oynatıcı yoksa state nil")
            assert(animation.clip(1) == nil, "klip nil")
            assert(animation.is_playing(1, "run") == false, "hiçbir şey oynamıyor")
            "#,
        )
        .exec()
        .unwrap();
    }
}
