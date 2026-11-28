# NATIVE-FROZEN-FALLING-DRAWN-LATE

Disposition: open pending graphical red/green pair. Core adds `frozen_falling_presented_current_tick` to `native-v2-input-sand`, comparing the falling block's own presented Y with its predicted Y after the existing verified two-tick, no-position-packet window. The old descent assertion admitted an intermediate previous-tick value. This is not yet a claim that either graphical side ran.

Client control commit `1f6faef59865f34f6510339424c319fa3c0a68e4` reverses the production hunk of client `c6a56f1d` on stand source `529ed2dfe4c1ec9cbcf5f43b7489f09625dc5d5a`; restore commit `51fda1e25803436b85bfe7c68c59355ec2652888` re-lands exactly that hunk. The control binary was pinned as `pins/1f6faef59865-client-x86_64-unknown-linux-gnu-incr` (`../frozen-witness/control-pin-2.log`). The captured gate must fail specifically at the new assertion, then restore must pass on the same core tree/server. Earlier pin/test attempts were cancelled when a different witness approach proved unnecessary; their partial logs are under `../frozen-witness/`, not red controls.

Control binary SHA-256 `0d6344fd0ccb48085f580efb9172685650c60398d39e7331c0d1fbf76e9691c4`; the stand source binary SHA-256 is `31cf654585586062a7024fccae05e6c2ca37bfda4414084c8d472db78be5fbb5`.

Restore pin `pins/51fda1e25803-client-x86_64-unknown-linux-gnu-incr`, SHA-256 `9a18728d3081ba26efc46beb5b132ea5f5b8d8d0f2dfb7464f139f2427dbc4c6`; build log `../frozen-witness/restore-pin.log`. This is a separately built re-land of the exact control hunk, not the stand binary.
