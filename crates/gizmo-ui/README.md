# gizmo-ui — experimental

Flexbox **layout** and pointer **hit-testing** for the [Gizmo engine](https://github.com/bdrtr/Gizmo)'s ECS.

**This crate draws nothing.** It has no renderer dependency and no graphics code.
If you are here looking for a UI toolkit, read the next two sections before you
spend an afternoon on it.

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
// state. Turning those into pixels is your job — see "What does not work".
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

- **No text rendering.** No `Text` component, no font loading, no glyph
  rasterisation — not in this crate and not in `gizmo-renderer`. This is the
  single largest missing piece, and the reason for the "experimental" label.
- **No drawing of any kind.** No vertices, no draw calls, no renderer
  integration. `BackgroundColor` is stored on the entity and read by nothing in
  the workspace.
- **No CSS Grid.** `Style` covers flexbox and block layout only. taffy's grid
  algorithm is compiled in but unreachable — there is no way to say
  `display: grid` or to describe a track template.
- **No z-order or occlusion.** The hit-test is a flat loop, so overlapping
  elements all report `Hovered`.
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
