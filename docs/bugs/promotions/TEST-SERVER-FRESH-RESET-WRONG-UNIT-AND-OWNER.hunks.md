# TEST-SERVER-FRESH-RESET-WRONG-UNIT-AND-OWNER — patch mode, one script

`fix_commit = patch:1f7d397bcbac9651195df445ec4a7431785425bf853585ece6c041db1925dccb` pins `docs/bugs/fix-patches/TEST-SERVER-FRESH-RESET-WRONG-UNIT-AND-OWNER.patch`
(the base→HEAD diff of `tools/dev/test-server/deploy-reset.sh`, rebuilt over `b00a8bdaad` and the
later script's comments). The oracle is `tools/dev/test_deploy_reset.py`: it reads the script's
text and never launches it, because the owner prohibits executing this helper in any form.

The four proving refs and their red lines are in `promotions/TEST-SERVER-FRESH-RESET-WRONG-UNIT-AND-OWNER.json`;
the receipt `83c66ef99f` reds are at `test_deploy_reset.py:57` (unit name and the exact
`is-active` boundary), `:68` (the whole archive listing) and `:75` (the `User=` default chown, backup
collision included); the order test is registered as a ref and reds at its own assert in the control.

Omitted: nothing — the patch is the whole script change.
