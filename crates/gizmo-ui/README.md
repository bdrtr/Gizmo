# gizmo-ui — experimental

Flexbox **layout** and pointer **hit-testing** for the [Gizmo engine](https://github.com/bdrtr/Gizmo)'s ECS.

**This crate still draws nothing itself** — it has no renderer dependency and no
graphics code — but since 2026-08-24 what it computes *is* drawn. The engine's
facade (`gizmo::systems::render::record_text`) reads `Node` and
`BackgroundColor` and paints them, and a `Text` on the same entity is placed in
that node's box. The bridge is in the facade because this crate sits **above**
`gizmo-app`: the renderer cannot see a `Node` and never will.

If you are here looking for a full UI toolkit, read "What does not work" before
you spend an afternoon on it — the list is still long.

## What works

Two systems, registered by `gizmo_ui::register(&mut world, &mut schedule)` or by
the `UiPlugin` (`app` feature):

| System | Does |
| --- | --- |
| `ui_layout_system` | Mirrors every entity with a `Style` into the layout tree held by `UiContext`, computes layout for each root against the current window size, writes back `Node { position, size }` in **absolute** window pixels. Reclaims layout nodes when `Style` is removed. |
| `ui_interaction_system` | Hit-tests the mouse against each `Node`'s half-open `[pos, pos + size)` box and sets `Interaction` to `None` / `Hovered` / `Pressed`. |

27 unit tests cover exactly that: layout write-back (including ancestor-offset
accumulation), the POD-`Style` → engine-style conversion, node lifecycle, the
hit-test predicate, and the interaction state machine.

```rust,ignore
use gizmo_ui::prelude::*;

// Layout + interaction on a bare World/Schedule — no gizmo-app required.
// `register` also registers the component types and inserts `UiContext`.
gizmo_ui::register(&mut world, &mut schedule);

let button = world.spawn_bundle(ButtonBundle {
    style: Style {
        width: Val::Px(120.0),
        height: Val::Percent(10.0),   // CSS scale: 10% of the parent
        padding: UiRect::all(Val::Px(8.0)),
        ..Default::default()
    },
    ..Default::default()
});

schedule.run(&mut world, dt);
// `button`'s `Node` now holds the computed box and `Interaction` the pointer
// state. Under the engine's renderer that box is painted (`BackgroundColor`)
// and a `Text` on the same entity is laid into it; standalone, turning them
// into pixels is still your job.
```

## `Style` is our own type

[`taffy`](https://crates.io/crates/taffy) does the layout maths, but it is an
implementation detail: **no taffy type is reachable through this crate's API.**
`Style` is plain old data — `Val` lengths (`Auto` / `Px` / `Percent`, on the CSS
`0..=100` percent scale), `UiRect`s, and a flexbox subset — and it is converted
to a taffy style in exactly one function, inside `UiContext`.

Before 0.9 `Style` was a newtype that deref'd to `taffy::style::Style`, and the
prelude glob-re-exported taffy's `style` and `geometry` modules. That made a
third-party type part of the public API, and it forced two `unsafe impl Send/Sync`
on the component, because `taffy::Style` is structurally `!Send`. Both are gone.

The properties `Style` does **not** model — CSS Grid, `overflow`, `box_sizing`,
`direction`, `text_align`, `float`/`clear`, intrinsic sizing keywords and
`calc()` — are listed in the type's rustdoc.

## What does not work

- **No drawing in *this* crate.** Still no vertices and no draw calls here; what
  changed on 2026-08-24 is that something else reads the output. `gizmo-renderer`
  gained fonts, a glyph atlas and a `Text` component, and the facade's text pass
  paints `BackgroundColor` and places a `Text` in its `Node`. Used standalone —
  `register` on a bare `World`, no engine renderer — this crate is what it always
  was: geometry and hover state, and the drawing is yours.
- **No rich text, wrapping, shaping, bidi or font fallback.** What the engine
  draws is one font per `Text`, breaking only on `\n`. A label longer than its box
  overflows it, because there is no clipping either (below).
- **No CSS Grid.** `Style` covers flexbox and block layout only. taffy's grid
  algorithm is compiled in but unreachable — there is no way to say
  `display: grid` or to describe a track template.
- **No z-order or occlusion.** The hit-test is a flat loop, so overlapping
  elements all report `Hovered` — and the drawing has no per-element order
  either: the engine paints every background and then every glyph, so a label is
  always above every panel. That is right for a button and wrong for two
  overlapping windows, and there is no `z` on `Node` to sort by.
- **No click/focus events, keyboard handling, scrolling, clipping or text
  input.** `Interaction` is recomputed each frame; it is state, not an event
  stream.
- A UI entity whose `Parent` is not itself a styled UI entity gets no layout
  pass at all. (Read from the code; no test covers it.)

## Use this if / use something else if

Use `gizmo-ui` if you want the engine to solve box geometry and hover/press state
and you will do the drawing yourself — read `Node` and `BackgroundColor` in your
own pass.

If you want a HUD that is visible on screen today, use the `egui` integration in
`gizmo-engine` (`egui` feature, and `editor` on top of it). It renders, text
included, and it does not go through this crate.

Note: `gizmo-engine` enables its `ui` feature **by default**, so these types show
up in `gizmo::prelude::*` unasked. That is not evidence anything is being drawn.

## Özet (TR)

Bu crate **hiçbir şey çizmez**. Yaptığı iki şey var: taffy ile flexbox
yerleşimi hesaplayıp `Node`'a mutlak pencere koordinatı olarak yazmak, ve fare
konumunu `Node` kutularıyla test edip `Interaction` durumunu güncellemek. İkisi
de 27 birim testiyle kaplı. **Metin render'ı yok** — ne burada, ne
`gizmo-renderer`'da; "deneysel" etiketinin sebebi budur. Ekranda görünen bir HUD
istiyorsan `gizmo-engine`'in `egui` özelliğini kullan.

`Style` artık taffy'nin tipi değil, kendi POD tipimiz (`Val` / `UiRect`); taffy
public API'de hiç görünmüyor ve component'teki iki `unsafe impl Send/Sync`
kalktı. Yüzde değerleri CSS ölçeğinde (`Val::Percent(50.0)` = %50). CSS Grid
modellenmiyor.

## Status

Experimental in the 0.x sense. Nothing here is deprecated or scheduled for
removal; the label describes how much of a UI toolkit this is, not its lifespan.
Expect the component set to change when rendering lands.

## License

MIT OR Apache-2.0, same as the rest of the workspace.
