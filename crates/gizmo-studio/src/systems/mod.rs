/// Exporting a game: the build requests the toolbar raises, and what packages a runtime.
pub mod build;
/// The editor camera: fly controls, focusing, and the orbit the viewport drives.
pub mod camera;
/// Garbage collection of soft-deleted entities, and the auto-save timer beside it.
pub mod gc;
/// Feeding viewport input into picking, and the scene view's own interactions.
pub mod input;
/// Scene operations raised by the UI: spawn, delete, duplicate, save, load.
pub mod scene_ops;
/// Keyboard shortcuts — undo/redo, delete, duplicate, focus, gizmo modes.
pub mod shortcuts;
/// Driving play mode: stepping the game loop, and the snapshot play/stop rests on.
pub mod simulation;
/// The editor's own gizmos: the transform handles and the selection outline.
pub mod gizmos;
