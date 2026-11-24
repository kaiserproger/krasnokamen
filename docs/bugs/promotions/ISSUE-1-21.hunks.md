# ISSUE-1-21 — hunk map

Row: `ISSUE-1-21` (client, major) — the first-person tool clips through a block.
Ref: `harness:scene-shot:issue1-21-tool-clip` (`tool_wall_visible`).

## The hunk

`src/render/frame.rs`, the first-person depth attachment of the hand pass, one line:

```rust
load: wgpu::LoadOp::Clear(1.),
```

The historical control `d4c5a326243c` ("revert(ISSUE-1-21 control): hand pass loads world depth
instead of clearing it") uses `wgpu::LoadOp::Load` there: the hand pass inherits the world depth, so a
tool held over a wall is depth-tested behind it. That commit is a child of `34a85940` and predates the
26.3 cutover, so the pre-image is rebuilt as a committed reversal at HEAD:

- reversal `3ad35fe51984d1c426d0d925f759d3e8c7a9b41b` — `Load` (scene red);
- re-land `e7c8a46a5dc148c4ead5037fbfeddcb52069ae5e` — `Clear(1.)`; row `fix_commit`.
- `git diff 3ad35fe519 e7c8a46a5d` is that one line.

## Gate

`scene-shot` scenario `issue1-21-tool-clip` (`tools/harness/tool_clipping_scene.json`, now registered):
a diamond pickaxe against an obsidian wall at 1000,161,1001, far and close held frames.
`tool_wall_visible` checks the received target, selected tool and eye distance, the dark wall behind the
turquoise tool pixels, and the tool pixel count. The control fails it; measured counts are in the
receipt's `tool_wall_pixels`.
