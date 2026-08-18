//! Fighting-game input recording/playback: `FrameRecord`/`PlaybackData` (deterministic replay)
//! and the `FighterInputBuffer` motion/command buffer. Extracted verbatim from input.rs.

use super::*;
/// One frame of a recorded session: the input that was in force and how long the frame lasted.
///
/// A record is self-contained — the [`Input`] is cloned whole rather than stored as a diff
/// against the previous frame — so records can be inspected or spliced individually. They are
/// only *meaningful* in sequence, though, since the held-key state of frame N is what makes
/// frame N+1's edges make sense.
#[derive(Serialize, Deserialize, Clone)]
pub struct FrameRecord {
    /// Frame duration in seconds, exactly as the recorder stored it.
    ///
    /// Replaying feeds this value back to the simulation instead of the replaying machine's
    /// measured frame time, so playback reproduces the original stepping regardless of how
    /// fast the replay runs. Nothing here bounds or checks the number: whatever the recorder
    /// decided to use as that frame's delta — already clamped, or not — is what a replay will
    /// see, so the field is only as faithful as the recording side was.
    pub dt: f32,
    /// Complete input snapshot for the frame, edge sets (`just_pressed` / `just_released`)
    /// included, so a replay reproduces one-shot triggers and not just held state.
    pub input: Input,
}

/// A whole recorded session — an ordered list of per-frame input snapshots, and nothing else.
///
/// This is the entire replay format: no world state, no random seeds, no checksums. The
/// engine's windowed loop replays it by overwriting its live frame delta and its whole live
/// [`Input`] from one record per frame and letting the simulation re-derive everything else,
/// so a replay only reproduces the original run on a build whose simulation behaves
/// identically.
#[derive(Serialize, Deserialize, Clone)]
pub struct PlaybackData {
    /// Frames in chronological order, index 0 being the first recorded frame.
    ///
    /// That ordering is a convention this type does not enforce: it is a plain `Vec`, and
    /// [`PlaybackData::save`] / [`PlaybackData::load`] round-trip whatever order it holds.
    pub frames: Vec<FrameRecord>,
}

impl PlaybackData {
    /// Serialises the replay to `path` as pretty-printed RON, truncating any existing file.
    ///
    /// Errors are flattened into a human-readable `String`: a serialisation failure and a
    /// filesystem failure are not distinguishable by type, only by the message text, so do
    /// not match on them. Missing parent directories are not created, and the write is not
    /// atomic — an interrupted write leaves a truncated, unloadable file where the old
    /// recording used to be.
    pub fn save(&self, path: &str) -> Result<(), String> {
        let string_data = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("Serilestirme hatasi: {}", e))?;
        std::fs::write(path, string_data).map_err(|e| format!("Dosya yazma hatasi: {}", e))?;
        Ok(())
    }

    /// Reads and parses a RON replay previously written by [`PlaybackData::save`].
    ///
    /// The whole file is read into memory before parsing, and the two failure modes —
    /// unreadable file and malformed RON — collapse into one `String`; there is no
    /// programmatic way to tell them apart. Parsing is all-or-nothing: a file truncated
    /// mid-write yields an error rather than the frames that did make it to disk.
    pub fn load(path: &str) -> Result<Self, String> {
        let string_data =
            std::fs::read_to_string(path).map_err(|e| format!("Dosya okuma hatasi: {}", e))?;
        ron::from_str(&string_data).map_err(|e| format!("Deserilestirme hatasi: {}", e))
    }
}

// ==================== FIGHTER INPUT BUFFER ====================

/// The struct that holds the key states for each frame.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameActions {
    /// Action names held during this frame (level, not edge).
    ///
    /// In a frame built by [`FighterInputBuffer::update`], only names passed as
    /// `actions_to_track` can appear — an action nobody asked about is simply absent, which is
    /// indistinguishable from it not being held. The fields are public, so a hand-built
    /// `FrameActions` is under no such restriction.
    pub pressed: HashSet<String>,
    /// Actions whose press edge landed on this frame — the one-shot set, normally also
    /// present in `pressed`.
    pub just_pressed: HashSet<String>,
    /// Actions whose release edge landed on this frame.
    ///
    /// Normally disjoint from `pressed`, with one deliberate exception: a button pressed and
    /// released inside a single frame is reported by [`Input`] as held *and* just-released
    /// *and* just-pressed, so all three sets contain it. That is the fast-tap guarantee, not
    /// a bug — it is what lets a one-frame tap be seen at all.
    pub just_released: HashSet<String>,
}

/// An Input Buffer designed specifically for fighting games (Gizmo Fight).
/// By holding all the key movements of the last N frames in memory, it enables combo
/// (Hadouken etc.) detection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FighterInputBuffer {
    /// Recorded frames, **newest first**: `frames[0]` is the frame most recently pushed by
    /// [`FighterInputBuffer::update`], `frames[1]` the one before it, and the back of the
    /// deque is the oldest surviving frame.
    ///
    /// [`FighterInputBuffer::check_combo_strict`] depends on this direction — it walks the
    /// deque front to back and matches the requested sequence in reverse. The field is public
    /// so tests and tools can seed history directly; seed it with `push_front`, so that index
    /// 0 stays the newest frame and the deque reads backwards in time.
    pub frames: std::collections::VecDeque<FrameActions>,
    /// How many frames of history to keep — that is, how far back in time a combo may reach.
    /// 60 is one second at 60 FPS.
    ///
    /// [`FighterInputBuffer::update`] pushes first and then drops ONE oldest frame if the
    /// buffer is over the limit — an `if`, not a `while`. So the length never decreases:
    /// lowering `max_frames` from 60 to 10 on a full buffer leaves it sitting at 60
    /// indefinitely. It caps growth, it does not truncate. Set it before filling the buffer,
    /// or clear the buffer after changing it.
    ///
    /// `0` is legal and leaves the buffer permanently empty (each update pushes one frame and
    /// drops it again), which makes every combo query fail.
    pub max_frames: usize,
}

impl FighterInputBuffer {
    /// 60 frames (1 second), a standard buffer size, is ideal for fighting games.
    pub fn new(max_frames: usize) -> Self {
        Self {
            frames: std::collections::VecDeque::with_capacity(max_frames),
            max_frames,
        }
    }

    /// Called on every game frame and updates the buffer.
    pub fn update(&mut self, input: &Input, action_map: &ActionMap, actions_to_track: &[&str]) {
        let mut frame = FrameActions {
            pressed: HashSet::new(),
            just_pressed: HashSet::new(),
            just_released: HashSet::new(),
        };

        for &action in actions_to_track {
            if action_map.is_action_pressed(input, action) {
                frame.pressed.insert(action.to_string());
            }
            if action_map.is_action_just_pressed(input, action) {
                frame.just_pressed.insert(action.to_string());
            }
            if action_map.is_action_just_released(input, action) {
                frame.just_released.insert(action.to_string());
            }
        }

        self.frames.push_front(frame);
        if self.frames.len() > self.max_frames {
            self.frames.pop_back();
        }
    }

    /// Checks whether the given combo sequence took place in the last frames or not.
    ///
    /// `sequence`: the actions that must be pressed in order, e.g. `["Down", "Right", "Punch"]`.
    /// `max_gap`: how many frames may pass between two of them (the tolerance; 10–15 is usual).
    ///
    /// **A step matches only on its press edge (`just_pressed`), never on a held button.** This
    /// used to accept `just_pressed || pressed`, and that is not a stricter reading of a motion —
    /// it is the end of order checking. A player merely *holding* Down, Right and Punch at the
    /// same time has all three in `pressed` on every frame of the buffer, so any sequence of them
    /// matches within three consecutive frames: a simultaneous press read as a quarter-circle, in
    /// either direction. `max_gap` stops meaning anything too, since a held button matches on
    /// every frame it is held. This function's own comment already said the safe rule was
    /// `just_pressed`; the code did the other thing.
    ///
    /// Real motions are unaffected, because they are made of press edges: pressing Down, then
    /// Right, then Punch gives each of them an edge on the frame it went down. What is lost is a
    /// step whose edge fell outside the buffer's window — a button already held when recording
    /// began — which is a motion this buffer never saw the start of.
    ///
    /// The Lua `fighter.check_combo` in `gizmo-scripting` implements the same rule over the
    /// mirrored buffer, and a cross-check test there drives both against the same scenarios.
    pub fn check_combo_strict(&self, sequence: &[&str], max_gap: usize) -> bool {
        if sequence.is_empty() || self.frames.is_empty() {
            return false;
        }

        // Aramaya dizilimin SON tuşundan (en yakın zamandaki) başlıyoruz.
        // Çünkü `self.frames[0]` mevcut frame'i (şimdi) temsil eder.
        let mut seq_idx = sequence.len() as isize - 1;
        let mut frames_since_last_match = 0;

        for frame in &self.frames {
            if frames_since_last_match > max_gap {
                // Kombodaki iki tuş arasına çok fazla zaman girmiş, kombo bozuldu.
                return false;
            }

            let required_action = sequence[seq_idx as usize];

            // Press edge only — see this function's docs for why `|| pressed` was not a tolerance
            // but a hole: a held button matches on every frame it is held, which makes a
            // simultaneous press indistinguishable from a motion and makes `max_gap` meaningless.
            if frame.just_pressed.contains(required_action) {
                // Eşleşme bulundu, komboda bir önceki adıma geç
                seq_idx -= 1;
                frames_since_last_match = 0;

                if seq_idx < 0 {
                    // Dizilimin en başına (ilk tuşa) başarıyla ulaştık! Kombo yapıldı!
                    return true;
                }
            } else {
                frames_since_last_match += 1;
            }
        }

        false
    }
}

impl Default for FighterInputBuffer {
    fn default() -> Self {
        Self::new(60)
    }
}

#[cfg(test)]
mod fighter_tests {
    use super::*;

    /// Builds one recorded frame. `just_pressed` names go into `pressed` too, which is what
    /// [`FighterInputBuffer::update`] produces for a key that went down this frame — a press edge
    /// is also a hold.
    fn frame(just_pressed: &[&str], held: &[&str]) -> FrameActions {
        let mut pressed: HashSet<String> =
            just_pressed.iter().map(|s| s.to_string()).collect();
        pressed.extend(held.iter().map(|s| s.to_string()));
        FrameActions {
            pressed,
            just_pressed: just_pressed.iter().map(|s| s.to_string()).collect(),
            just_released: HashSet::new(),
        }
    }

    /// A quarter-circle-forward punch, recorded the way the input system records one: Down goes
    /// down, then Right while Down is still held, then Down is released, then Punch.
    #[test]
    fn test_fighter_input_buffer_combo() {
        let mut buffer = FighterInputBuffer::new(60);

        buffer.frames.push_front(frame(&["Down"], &[]));
        buffer.frames.push_front(frame(&["Right"], &["Down"]));
        buffer.frames.push_front(frame(&[], &["Right"])); // Down bırakıldı
        buffer.frames.push_front(frame(&["LightPunch"], &[]));

        let combo = ["Down", "Right", "LightPunch"];
        assert!(buffer.check_combo_strict(&combo, 5), "Kombo basariyla algilanmali");

        let wrong_combo = ["LightPunch", "Right", "Down"];
        assert!(
            !buffer.check_combo_strict(&wrong_combo, 5),
            "Yanlis kombo sirasi algilanmamali"
        );
    }

    /// **Holding the buttons is not performing the motion.**
    ///
    /// This is what matching on `pressed` cost, and it is not a subtle cost: a player who simply
    /// holds Down, Right and Punch at once has all three in `pressed` on every frame of the
    /// buffer, so *any* order of them completed within three consecutive frames — the motion, and
    /// its reverse, and anything else. Order checking stopped meaning anything, and so did
    /// `max_gap`, since a held button matches on every frame it is held.
    #[test]
    fn a_simultaneous_hold_is_not_a_motion() {
        let mut buffer = FighterInputBuffer::new(60);
        // Ten frames of all three held, with the press edges long gone off the back of the window.
        for _ in 0..10 {
            buffer
                .frames
                .push_front(frame(&[], &["Down", "Right", "LightPunch"]));
        }

        assert!(
            !buffer.check_combo_strict(&["Down", "Right", "LightPunch"], 5),
            "üç tuşu birlikte TUTMAK çeyrek daire hareketi değildir"
        );
        assert!(
            !buffer.check_combo_strict(&["LightPunch", "Right", "Down"], 5),
            "ve ters sırası da değildir — tutulan tuş her karede eşleşir, sıra denetimi biter"
        );
    }

    /// A step whose press edge is inside the window still matches while the button stays held —
    /// the edge is what is looked for, not the state, so a motion finished under a held button is
    /// recognised exactly once.
    #[test]
    fn the_press_edge_is_what_matches_not_the_hold() {
        let mut buffer = FighterInputBuffer::new(60);
        buffer.frames.push_front(frame(&["Down"], &[]));
        // Down stays held for three more frames, then Punch.
        for _ in 0..3 {
            buffer.frames.push_front(frame(&[], &["Down"]));
        }
        buffer.frames.push_front(frame(&["LightPunch"], &["Down"]));

        assert!(
            buffer.check_combo_strict(&["Down", "LightPunch"], 5),
            "basma kenarı pencerenin içindeyse hareket tanınmalı"
        );
    }

    #[test]
    fn test_fighter_input_buffer_max_gap() {
        let mut buffer = FighterInputBuffer::new(60);

        buffer.frames.push_front(frame(&["Down"], &[]));

        // Araya 10 boş kare girsin
        for _ in 0..10 {
            buffer.frames.push_front(frame(&[], &[]));
        }

        buffer.frames.push_front(frame(&["LightPunch"], &[]));

        let combo = ["Down", "LightPunch"];
        
        // max_gap = 5 ise başarısız olmalı (10 kare boşluk var)
        assert!(!buffer.check_combo_strict(&combo, 5), "Cok yavas basildi, algilanmamali");
        
        // max_gap = 15 ise başarılı olmalı
        assert!(buffer.check_combo_strict(&combo, 15), "Max gap genis oldugu icin algilanmali");
    }
}

