"""Read declared Fabric, Forge and NeoForge mod metadata without loading code.

``inspect_mod`` returns a JSON-serializable dictionary with these fields:

* ``loader``: ``fabric``, ``forge`` or ``neoforge``.
* ``mod_ids``: mod IDs in declaration order; ``versions``: ID-to-version map.
* ``minecraft_constraints``: dependency records whose ID is ``minecraft``.
* ``entrypoints``: Fabric records with ``key``, ``value``, ``adapter`` and
  ``environment``. Forge/NeoForge annotation entrypoints are not resolved.
* ``dependencies``: records with ``owner``, ``id``, ``relation``, ``version``,
  ``side``, ``mandatory`` and ``ordering``. Unspecified values are null.
* ``mixins``: records with ``config``, ``environment``, ``required``,
  ``package``, and the unqualified ``mixins``, ``server`` and ``client`` lists.
* ``access_transformers``: declared access widener/transformer resource names,
  including Forge's conventional resource when present.
* ``embedded_jars``: Fabric nested JARs and Forge/NeoForge JarJar declarations
  (not recursively inspected).
* ``class_path``: manifest Class-Path entries; ``warnings``: limitations.

Declaration order is retained. Metadata inspection does not establish bytecode
compatibility, version support, or Mixin execution. Invalid ZIPs, missing or
ambiguous loader metadata, malformed metadata, and missing or malformed declared
Mixin configs raise ValueError; filesystem errors propagate as OSError.
"""

import json
from pathlib import Path
import tomllib
from zipfile import BadZipFile, ZipFile


def _object(value, description):
    if not isinstance(value, dict):
        raise ValueError(f"{description} must be an object")
    return value


def _string(value, description):
    if not isinstance(value, str) or not value:
        raise ValueError(f"{description} must be a nonempty string")
    return value


def _list(value, description):
    if not isinstance(value, list):
        raise ValueError(f"{description} must be a list")
    return value


def _json_resource(archive, name, description):
    try:
        raw = archive.read(name)
    except KeyError as exc:
        raise ValueError(f"{description} {name!r} is missing from the JAR") from exc
    try:
        return json.loads(raw.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ValueError(f"malformed {description} {name!r}: {exc}") from exc


def _manifest(archive):
    try:
        raw = archive.read("META-INF/MANIFEST.MF")
    except KeyError:
        return {}
    try:
        lines = raw.decode("utf-8").splitlines()
    except UnicodeDecodeError as exc:
        raise ValueError(f"malformed JAR manifest: {exc}") from exc
    attributes = {}
    key = None
    for line in lines:
        if not line:
            break  # The main section ends here; named sections are not global.
        if line.startswith(" "):
            if key is None:
                raise ValueError("malformed JAR manifest: orphan continuation line")
            attributes[key] += line[1:]
        else:
            name, separator, value = line.partition(": ")
            if not separator or not name:
                raise ValueError(f"malformed JAR manifest attribute: {line!r}")
            key = name.lower()
            attributes[key] = value
    return attributes


def _mixin(archive, config, environment):
    config = _string(config, "Mixin config name")
    data = _object(_json_resource(archive, config, "Mixin config"), f"Mixin config {config!r}")
    required = data.get("required")
    if "required" in data and not isinstance(required, bool):
        raise ValueError(f"Mixin config {config!r}: required must be a boolean")
    package = data.get("package")
    if "package" in data and not isinstance(package, str):
        raise ValueError(f"Mixin config {config!r}: package must be a string")
    result = {"config": config, "environment": environment, "required": required, "package": package}
    for key in ("mixins", "server", "client"):
        values = _list(data.get(key, []), f"Mixin config {config!r} {key}")
        result[key] = [_string(value, f"Mixin config {config!r} {key} entry") for value in values]
    return result

def _dependency(owner, mod_id, relation, version, side=None, mandatory=None, ordering=None):
    return {"owner": owner, "id": mod_id, "relation": relation, "version": version,
            "side": side, "mandatory": mandatory, "ordering": ordering}


def _fabric(archive, result):
    data = _object(_json_resource(archive, "fabric.mod.json", "Fabric metadata"), "Fabric metadata")
    mod_id = _string(data.get("id"), "Fabric mod id")
    version = _string(data.get("version"), "Fabric mod version")
    result["mod_ids"].append(mod_id)
    result["versions"][mod_id] = version
    environment = data.get("environment")
    if environment is not None:
        _string(environment, "Fabric environment")
        result["warnings"].append(f"Fabric mod environment: {environment}")
    entrypoints = _object(data.get("entrypoints", {}), "Fabric entrypoints")
    for key, entries in entrypoints.items():
        for entry in _list(entries, f"Fabric entrypoints {key!r}"):
            if isinstance(entry, str):
                value, adapter = _string(entry, "Fabric entrypoint"), None
            else:
                entry = _object(entry, "Fabric entrypoint")
                value = _string(entry.get("value"), "Fabric entrypoint value")
                adapter = _string(entry["adapter"], "Fabric entrypoint adapter") if "adapter" in entry else None
            result["entrypoints"].append({"key": key, "value": value, "adapter": adapter,
                                          "environment": environment})
    for relation in ("depends", "recommends", "suggests", "conflicts", "breaks"):
        dependencies = _object(data.get(relation, {}), f"Fabric {relation}")
        for dependency_id, constraint in dependencies.items():
            _string(dependency_id, "Fabric dependency id")
            if isinstance(constraint, list):
                constraint = [_string(item, "Fabric dependency constraint") for item in constraint]
            else:
                constraint = _string(constraint, "Fabric dependency constraint")
            result["dependencies"].append(_dependency(mod_id, dependency_id, relation, constraint,
                                                       side=environment))
    for declaration in _list(data.get("mixins", []), "Fabric mixins"):
        if isinstance(declaration, str):
            config, side = declaration, None
        else:
            declaration = _object(declaration, "Fabric Mixin declaration")
            config = declaration.get("config")
            side = declaration.get("environment")
            if side is not None:
                _string(side, "Fabric Mixin environment")
        result["mixins"].append(_mixin(archive, config, side))
    if "accessWidener" in data:
        result["access_transformers"].append(_string(data["accessWidener"], "Fabric accessWidener"))
    for nested in _list(data.get("jars", []), "Fabric nested jars"):
        nested = _object(nested, "Fabric nested jar")
        result["embedded_jars"].append(_string(nested.get("file"), "Fabric nested jar file"))
    if result["embedded_jars"]:
        result["warnings"].append("Embedded JARs were declared but not inspected.")


def _forge(archive, result, metadata_name):
    try:
        raw = archive.read(metadata_name).decode("utf-8-sig")
        data = tomllib.loads(raw)
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        raise ValueError(f"malformed {metadata_name}: {exc}") from exc
    mods = _list(data.get("mods", []), f"{metadata_name} mods")
    if not mods:
        raise ValueError(f"{metadata_name} declares no mods")
    for mod in mods:
        mod = _object(mod, f"{metadata_name} mod")
        mod_id = _string(mod.get("modId"), "modId")
        if mod_id in result["versions"]:
            raise ValueError(f"duplicate modId {mod_id!r} in {metadata_name}")
        result["mod_ids"].append(mod_id)
        result["versions"][mod_id] = (_string(mod["version"], f"version for {mod_id!r}")
                                      if "version" in mod else "")
    dependencies = _object(data.get("dependencies", {}), f"{metadata_name} dependencies")
    for owner, entries in dependencies.items():
        for entry in _list(entries, f"dependencies for {owner!r}"):
            entry = _object(entry, f"dependency for {owner!r}")
            dep_id = _string(entry.get("modId"), "dependency modId")
            version = entry.get("versionRange")
            side = entry.get("side")
            ordering = entry.get("ordering")
            relation = entry.get("type")
            for label, value in (("versionRange", version), ("side", side),
                                 ("ordering", ordering), ("type", relation)):
                if value is not None:
                    _string(value, f"dependency {label}")
            mandatory = entry.get("mandatory")
            if mandatory is not None and not isinstance(mandatory, bool):
                raise ValueError("dependency mandatory must be a boolean")
            result["dependencies"].append(_dependency(owner, dep_id, relation, version, side,
                                                       mandatory, ordering))
    for declaration in _list(data.get("mixins", []), f"{metadata_name} mixins"):
        declaration = _object(declaration, "Mixin declaration")
        result["mixins"].append(_mixin(archive, declaration.get("config"), None))
    for declaration in _list(data.get("accessTransformers", []), f"{metadata_name} accessTransformers"):
        declaration = _object(declaration, "access transformer declaration")
        result["access_transformers"].append(_string(declaration.get("file"), "access transformer file"))
    if "META-INF/accesstransformer.cfg" in archive.namelist():
        result["access_transformers"].append("META-INF/accesstransformer.cfg")
    jarjar = "META-INF/jarjar/metadata.json"
    if jarjar in archive.namelist():
        metadata = _object(_json_resource(archive, jarjar, "JarJar metadata"), "JarJar metadata")
        for entry in _list(metadata.get("jars", []), "JarJar jars"):
            entry = _object(entry, "JarJar entry")
            result["embedded_jars"].append(_string(entry.get("path"), "JarJar entry path"))
        if result["embedded_jars"]:
            result["warnings"].append("Embedded JARs were declared but not inspected.")
    result["warnings"].append("Forge/NeoForge @Mod entrypoints require bytecode inspection and are unresolved here.")


def inspect_mod(jar: Path) -> dict:
    """Inspect declarations in a mod JAR; never extract or execute its contents."""
    try:
        with ZipFile(jar) as archive:
            names = set(archive.namelist())
            candidates = [("fabric", "fabric.mod.json"), ("forge", "META-INF/mods.toml"),
                          ("neoforge", "META-INF/neoforge.mods.toml")]
            present = [(loader, name) for loader, name in candidates if name in names]
            if len(present) != 1:
                raise ValueError(f"expected exactly one Fabric/Forge/NeoForge metadata file, found {len(present)}")
            loader, metadata_name = present[0]
            result = {"loader": loader, "mod_ids": [], "versions": {}, "minecraft_constraints": [],
                      "entrypoints": [], "dependencies": [], "mixins": [], "access_transformers": [],
                      "class_path": [], "embedded_jars": [], "warnings": []}
            manifest = _manifest(archive)
            if loader == "fabric":
                _fabric(archive, result)
            else:
                _forge(archive, result, metadata_name)
            for name in manifest.get("mixinconfigs", "").split(","):
                if name.strip():
                    result["mixins"].append(_mixin(archive, name.strip(), None))
            result["access_transformers"].extend(manifest.get("fmlat", "").split())
            result["access_transformers"] = list(dict.fromkeys(result["access_transformers"]))
            result["class_path"] = manifest.get("class-path", "").split()
            result["minecraft_constraints"] = [dep for dep in result["dependencies"] if dep["id"] == "minecraft"]
            if result["mixins"]:
                result["warnings"].append("Mixin declarations were inspected only; Mixin behavior is not supported by metadata inspection.")
            result["warnings"].append("Metadata inspection does not establish bytecode compatibility or version support.")
            return result
    except BadZipFile as exc:
        raise ValueError(f"invalid JAR {jar}: {exc}") from exc
