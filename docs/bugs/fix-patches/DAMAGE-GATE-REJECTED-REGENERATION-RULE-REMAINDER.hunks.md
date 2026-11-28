# Damage fixture regeneration rule control

`b7020b427` introduced the rejected `gamerule natural_regeneration false`
command. `23f4e4b11` replaced that one line with
`gamerule natural_health_regeneration false`. The patch carries just that
historical correction; the command now runs in `_damage` before the first
hit rather than partway through `_warden_damage`.

The canonical query and chat fence remain unchanged with the patch reversed.
`damage_regeneration_disabled_on_server` requires both the set response and
`false` from the subsequent query. A missing or rejected set is not converted
into a timeout or an artificial negative result. This control does not
reverse the acknowledgement wait already covered by the parent row or any
other damage fixture step.
