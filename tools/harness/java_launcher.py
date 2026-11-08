"""Prepared real Java client commands over Mojang's locally cached metadata.

This is a command-building primitive called by runtime.start_client, not another
launcher process/lifecycle. It never invokes Gradle per player. Vanilla uses the
unmodified Mojang main class with the existing java-agent instrumentation only.
All published client/library/index sizes and SHA-1s are checked before execution.
Missing artifacts report their exact URL; no neighboring version is substituted.
"""
from __future__ import annotations

from functools import lru_cache
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import uuid
from typing import Any

from .matrix_api import MissingCapability

VERSION = "26.1.2"


def allowed(entry: dict[str, Any]) -> bool:
    rules = entry.get("rules")
    if not rules:
        return True
    permitted = False
    for rule in rules:
        system = rule.get("os", {})
        if system.get("name", "linux") != "linux":
            continue
        arch = system.get("arch")
        if arch and arch not in {platform.machine(), "amd64" if platform.machine() == "x86_64" else platform.machine()}:
            continue
        if system.get("version") and not re.search(system["version"], platform.release()):
            continue
        # No demo, custom resolution, quick-play, or account features enabled.
        if any(bool(value) for value in rule.get("features", {}).values()):
            continue
        permitted = rule["action"] == "allow"
    return permitted


@lru_cache(maxsize=512)
def _digest(path: Path, size: int, modified: int) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha1").hexdigest()


def verified(path: Path, pin: dict[str, Any], label: str) -> Path:
    if not path.is_file():
        raise MissingCapability(f"java_artifact:{label}:missing:{path}; published {pin.get('url', '')}")
    stat = path.stat()
    if stat.st_size != pin["size"] or _digest(path, stat.st_size, stat.st_mtime_ns) != pin["sha1"]:
        raise MissingCapability(f"java_artifact:{label}:size/hash mismatch:{path}; published {pin.get('url', '')}")
    return path.resolve()


def library_path(library: dict[str, Any], gradle_home: Path, roots: list[Path]) -> Path:
    pin = library["downloads"]["artifact"]
    coordinate = library["name"]
    relative = Path(pin["path"])
    candidates = [root / relative for root in roots]
    parts = coordinate.split(":")
    if len(parts) >= 3:
        group, artifact, version = parts[:3]
        base = gradle_home / "caches/modules-2/files-2.1" / group / artifact / version
        candidates.extend(sorted(base.glob("*/" + relative.name)))
    for path in candidates:
        if path.is_file():
            return verified(path, pin, coordinate)
    raise MissingCapability(f"java_library:{coordinate}; need {relative}; published {pin['url']}")


def vanilla_command(*, game_dir: Path, username: str, agent_jar: Path,
                    metadata_path: Path | None = None, assets_root: Path | None = None,
                    gradle_home: Path | None = None, library_roots: list[Path] | None = None) -> list[str]:
    version = os.environ.get("SOLARIS_JAVA_VERSION", VERSION)
    if version not in ("26.1.2", "26.3"):
        raise MissingCapability(f"unsupported plain Java version: {version}")
    gradle_home = gradle_home or Path(os.environ.get("GRADLE_USER_HOME", Path.home() / ".gradle"))
    cache = gradle_home / "caches/fabric-loom"
    metadata_path = metadata_path or Path(os.environ.get("SOLARIS_JAVA_VERSION_METADATA", cache / version / "mojang_minecraft_info.json"))
    if not metadata_path.is_file():
        raise MissingCapability(f"java_version_metadata:{version}:{metadata_path}; prepare the canonical Loader Java build cache")
    metadata = json.loads(metadata_path.read_text())
    if metadata.get("id") != version or metadata.get("mainClass") != "net.minecraft.client.main.Main":
        raise MissingCapability(f"true_vanilla:{version}:metadata version/main class mismatch")
    if metadata.get("javaVersion", {}).get("majorVersion") != 25:
        raise MissingCapability("true_vanilla:Java25 metadata required")
    if not agent_jar.is_file():
        raise MissingCapability(f"java_agent_jar:{agent_jar}; build :java-agent:jar once through mem-run")
    roots = library_roots if library_roots is not None else [
        gradle_home / "caches/minecraft/libraries",
        Path.home() / ".local/share/PrismLauncher/libraries",
    ]
    classpath = [library_path(item, gradle_home, roots) for item in metadata["libraries"]
                 if allowed(item) and "artifact" in item.get("downloads", {})]
    client_override = os.environ.get("SOLARIS_CLIENT_JAR")
    client = Path(client_override) if client_override else metadata_path.parent / "minecraft-client.jar"
    classpath.append(verified(client, metadata["downloads"]["client"], version + " client"))
    assets_root = assets_root or Path(os.environ.get("SOLARIS_JAVA_ASSETS_ROOT", cache / "assets"))
    pin = metadata["assetIndex"]
    choices = [assets_root / "indexes" / (pin["id"] + ".json"),
               assets_root / "indexes" / (version + "-" + pin["id"] + ".json")]
    index = next((path for path in choices if path.is_file()), choices[0])
    verified(index, pin, version + " asset index")
    if not (assets_root / "objects").is_dir():
        raise MissingCapability(f"java_assets_objects:{assets_root / 'objects'}")
    game_dir.mkdir(parents=True, exist_ok=True)
    natives = game_dir / "natives"
    natives.mkdir(exist_ok=True)
    java = shutil.which("java")
    if java is None:
        raise MissingCapability("java25:executable_not_on_PATH")
    offline_uuid = uuid.UUID(bytes=hashlib.md5(("OfflinePlayer:" + username).encode()).digest(), version=3)
    return [java, "-Xms128M", "-Xmx512M", "--sun-misc-unsafe-memory-access=allow",
            "--enable-native-access=ALL-UNNAMED", "-Dsolaris.client.vanillaEvents=true",
            f"-javaagent:{agent_jar.resolve()}",
            f"-Djava.library.path={natives}", f"-Djna.tmpdir={natives}",
            f"-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}",
            f"-Dio.netty.native.workdir={natives}", "-Dminecraft.launcher.brand=SolarisHarness",
            "-Dminecraft.launcher.version=1", "-cp", os.pathsep.join(map(str, classpath)),
            metadata["mainClass"], "--username", username, "--version", version,
            "--gameDir", str(game_dir), "--assetsDir", str(assets_root),
            "--assetIndex", index.stem, "--uuid", offline_uuid.hex,
            "--accessToken", "0", "--versionType", "release"]


def prepared_fabric_command(manifest_path: Path, game_dir: Path, username: str) -> list[str]:
    """Reuse the canonical Loom JavaExec command prepared once per group."""
    if not manifest_path.is_file():
        raise MissingCapability(f"fabric_launch_manifest:{manifest_path}")
    manifest = json.loads(manifest_path.read_text())
    original_game = manifest["game_dir"]
    original_name = manifest["username"]
    for path in manifest["classpath"]:
        if not Path(path).exists():
            raise MissingCapability(f"fabric_classpath:{path}")
    def rewrite(value: str) -> str:
        return value.replace(original_game, str(game_dir))
    jvm = [rewrite(value) for value in manifest["jvm_args"]
           if not value.startswith(("-Xmx", "-Xms"))]
    arguments = []
    for value in manifest["args"]:
        arguments.append(username if value == original_name else rewrite(value))
    return [manifest["java"], "-Xms128M", "-Xmx512M", *jvm, "-cp",
            os.pathsep.join(manifest["classpath"]), manifest["main_class"], *arguments]


def prepare_fabric_launch(artifact_dir: Path, agent_root: Path, timeout_seconds: float) -> Path:
    """Prepare canonical dependencies and export one exact JavaExec command."""
    import subprocess
    from . import runtime
    manifest = artifact_dir / "fabric-java-launch.json"
    game = artifact_dir / "prepared-game"
    env = os.environ.copy()
    env.update(SOLARIS_CLIENT_LAUNCH_MANIFEST=str(manifest),
               SOLARIS_CLIENT_MCP_TOKEN="prepared-command-not-a-listening-token",
               SOLARIS_CLIENT_MCP_PORT="26299")
    command = [str(agent_root / "gradlew"), "--no-daemon", "--no-configuration-cache",
               "--max-workers=2", "-p", str(agent_root),
               f"-Psolaris.clientMcp.gameDir={game}",
               "-Psolaris.clientMcp.username=PreparedJava",
               ":loader-fabric:runClientMcp"]
    log_path = artifact_dir / "fabric-prepare.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen(command, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            returncode = process.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired as error:
            runtime.stop_process(process)
            raise MissingCapability(f"fabric_prepare_timeout:{log_path}") from error
        except BaseException:
            runtime.stop_process(process)
            raise
    if returncode != 0 or not manifest.is_file():
        raise MissingCapability(f"fabric_prepare_failed:exit {returncode}; {log_path}")
    return manifest
