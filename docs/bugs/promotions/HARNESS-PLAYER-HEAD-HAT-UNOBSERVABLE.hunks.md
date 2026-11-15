# HARNESS-PLAYER-HEAD-HAT-UNOBSERVABLE: forward-ported hunk map

Base: `5315128eeb4cfcd19a6158db6498307ac674cad4`. The adjacent patch is identical to `../fix-patches/HARNESS-PLAYER-HEAD-HAT-UNOBSERVABLE.patch`; digest uses `ledger._split_non_test`. Tests/docs stay in the control. No invented pre-image or a/-side behavior adaptation.

f645ca39a: scene_shot.py hat assertion and runtime skin-pack install/restore are retained, as are both player_head_scene.json name hunks. Original new player_head_skin_pack API stays (b): the retained authored-skin test calls it. Pixel measurement/report fields stay (b): retained tests consume these reports; magenta is also shared by later worn/back oracles. The corrected head ROI stays (a), HARNESS-PLAYER-HEAD-ROI: reverting it would fail before checking hats. Gait/clock hunks and all other files listed below stay (a), their independent rows. fad1cef1e completes actual slim-default fixture coverage: its first skin-writing hunk is retained, plus the worn_hat condition in its second hunk (HEAD scene_shot.py:1250–1255,1302). Worn measurement/report API stays (b); clock changes in that and later hunks stay (a). Proving refs: tools/harness/test_scene_shot.py:1150–1151 requires missing placed hats to be rejected; :1251 requires the actual slim fallback texture. Neither relies on the ROI assertion. Individual controls: f645ca39a-only and fad1cef1e-only logs in receipts/HarnessFix4; combined patch carries both.

## Every original non-test hunk

The dispositions above apply to each listed hunk, including mixed hunks at sub-hunk granularity.

| Commit | Original path/hunk |
| --- | --- |
| `f645ca39a` | `tools/harness/audio.py` `@@ -42,12 +42,13 @@ class SoundCapture:` |
| `f645ca39a` | `tools/harness/backends/scene_shot.py` `@@ -720,7 +720,7 @@ def execute(probe: ProbeSteps, artifact: Path, steps: list[dict[str, Any]], clie` |
| `f645ca39a` | `tools/harness/backends/scene_shot.py` `@@ -729,7 +729,8 @@ def execute(probe: ProbeSteps, artifact: Path, steps: list[dict[str, Any]], clie` |
| `f645ca39a` | `tools/harness/backends/scene_shot.py` `@@ -1211,8 +1212,26 @@ def check_shade_override(probe: ProbeSteps, artifact: Path) -> None:` |
| `f645ca39a` | `tools/harness/backends/scene_shot.py` `@@ -1222,13 +1241,47 @@ def check_player_head(probe: ProbeSteps, artifact: Path) -> None:` |
| `f645ca39a` | `tools/harness/backends/scene_shot.py` `@@ -1269,19 +1322,7 @@ def check_dynamic_context(probe: ProbeSteps, artifact: Path) -> None:` |
| `f645ca39a` | `tools/harness/backends/scene_shot.py` `@@ -1537,9 +1578,15 @@ def run(` |
| `f645ca39a` | `tools/harness/backends/scene_shot.py` `@@ -1550,7 +1597,8 @@ def run(` |
| `f645ca39a` | `tools/harness/dynamic_context_scene.json` `@@ -52,7 +52,10 @@` |
| `f645ca39a` | `tools/harness/java_clients.py` `@@ -20,9 +20,8 @@ Contract the matrix runner relies on:` |
| `f645ca39a` | `tools/harness/java_clients.py` `@@ -65,7 +64,6 @@ from . import runtime` |
| `f645ca39a` | `tools/harness/java_clients.py` `@@ -381,14 +379,9 @@ def client_group(` |
| `f645ca39a` | `tools/harness/java_launcher.py` `@@ -169,7 +169,7 @@ def prepare_fabric_launch(artifact_dir: Path, agent_root: Path, timeout_seconds:` |
| `f645ca39a` | `tools/harness/java_launcher.py` `@@ -203,7 +203,7 @@ def modded_gradle_arguments(task: str) -> list[str]:` |
| `f645ca39a` | `tools/harness/java_launcher.py` `@@ -214,5 +214,5 @@ def modded_gradle_arguments(task: str) -> list[str]:` |
| `f645ca39a` | `tools/harness/player_head_scene.json` `@@ -61,10 +61,10 @@` |
| `f645ca39a` | `tools/harness/player_head_scene.json` `@@ -85,10 +85,10 @@` |
| `f645ca39a` | `tools/harness/runtime.py` `@@ -68,8 +68,6 @@ USERNAME_RE = re.compile(r"[A-Za-z0-9_]{1,16}\Z")` |
| `f645ca39a` | `tools/harness/runtime.py` `@@ -999,7 +997,6 @@ def start_client(` |
| `f645ca39a` | `tools/harness/runtime.py` `@@ -1022,20 +1019,34 @@ def start_client(` |
| `f645ca39a` | `tools/harness/vanilla_world_roundtrip.py` `@@ -127,7 +127,7 @@ def main():` |
| `fad1cef1e` | `tools/harness/backends/scene_shot.py` `@@ -1243,13 +1243,16 @@ def player_head_skin_pack(artifact: Path) -> Path:` |
| `fad1cef1e` | `tools/harness/backends/scene_shot.py` `@@ -1276,43 +1279,52 @@ def check_player_head(probe: ProbeSteps, artifact: Path) -> None:` |
| `fad1cef1e` | `tools/harness/backends/scene_shot.py` `@@ -1320,9 +1332,9 @@ def check_dynamic_context(probe: ProbeSteps, artifact: Path) -> None:` |
| `fad1cef1e` | `tools/harness/backends/scene_shot.py` `@@ -1339,8 +1351,7 @@ def check_dynamic_context(probe: ProbeSteps, artifact: Path) -> None:` |
| `fad1cef1e` | `tools/harness/backends/scene_shot.py` `@@ -1348,7 +1359,15 @@ def check_dynamic_context(probe: ProbeSteps, artifact: Path) -> None:` |
| `fad1cef1e` | `tools/harness/dynamic_context_scene.json` `@@ -49,7 +49,7 @@` |
| `fad1cef1e` | `tools/harness/dynamic_context_scene.json` `@@ -64,7 +64,7 @@` |
| `fad1cef1e` | `tools/harness/dynamic_context_scene.json` `@@ -82,7 +82,7 @@` |
| `fad1cef1e` | `tools/harness/dynamic_context_scene.json` `@@ -91,13 +91,40 @@` |

## Proof

Registered proving refs are run with retained tests on HEAD, with this patch reverse-applied and then restored. Receipts: `receipts/HarnessFix4/HARNESS-PLAYER-HEAD-HAT-UNOBSERVABLE.red.log` and `.green.log`. Only the symptom assertion named above carries the red; other failures are not evidence. The row stays open until Main runs ledger promote and independent review.
