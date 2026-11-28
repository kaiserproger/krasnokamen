# NATIVE-FREEZE-CAMERA

Disposition: open; no camera-specific negative control was produced. The existing `native-v2-input-camera` assertion `frozen_captured_camera_tracks_teleport` already passed its historical pre-fix client (see the row's original history and `receipts/D123/D123ClientNative/HANDOFF-13.md`). The separate `native-v2-frozen-witness` compares entity presentation rather than the local camera and cannot by itself prove this row.

Source inspection of core `168b5312d3319c4f22c573ba5abae016957a7380` and client `529ed2dfe4c1ec9cbcf5f43b7489f09625dc5d5a` also found that the witness expects an entity `position` array where `minecraft_list_entities` exposes `x`, `y`, and `z`, and the item witness lacks `presented_position`. No graphical run was attempted: load15 22.60 exceeded that scene's 12.0 admission budget. The capture-failure guard's unit red/green controls are in `../frozen-witness/` and are not a control for this camera symptom.
