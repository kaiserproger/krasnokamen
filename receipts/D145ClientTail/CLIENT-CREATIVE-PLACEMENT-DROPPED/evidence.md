# CLIENT-CREATIVE-PLACEMENT-DROPPED

Disposition: open; intermittent original failure did not reproduce, so there is no failing pointer edge to localize and no fix or red control is claimed.

On core tree `83ce9e6eea54effef614267ff708bc78fdb5b3e0`, after sourcing `/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/client-stand`, ran `KRASNOKAMEN_UI_POINTER_TRACE=1 /run/media/kaiserroman/games/solaris-work/receipts/D124/D124PackRepromote/gate.sh /run/media/kaiserroman/games/solaris-work/wt/D145ClientTail /run/media/kaiserroman/games/solaris-work/pins/2f4d802c08eb-server-x86_64-unknown-linux-gnu-incr /run/media/kaiserroman/games/solaris-work/pins/c879f0ae38b7-client-x86_64-unknown-linux-gnu-incr /run/media/kaiserroman/games/solaris-work/solaris-client scenario native-v2-input-menus`. The server pin is core `2f4d802c08eb`, not the tested tree, so this is diagnostic rather than promotion evidence. Command output: `trace-run.log` here; result and client trace: `.analysis/validation/20261128T083614-native-v2-input-menus-vqut3yq1/{result.json,native-client.log}` in the core tree.

The scenario passed (`creative_move_persists=true`). At client-log lines 48–56 the creative pickup used slot 10 and the placement's Down and Up both targeted slot 11; the release issued `target=slot/11 mode=Pickup`. This passing run does not decide why the historical failed run lost the placement.
