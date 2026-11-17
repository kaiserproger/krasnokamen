"""The split hot backends keep their patchable seams on the entry module.

``test_buildings_qa.py`` patches names on ``buildings_qa`` (``mock.patch.object(buildings_qa,
"_read")``, ``mock.patch.multiple(buildings_qa, ...)``) and ``test_scene_shot.py`` patches
``scene_shot.capture_native`` and ``scene_shot.png_size``. A Python patch replaces the attribute on
that one module, so a call site that bound such a name directly in an owner module would keep
running the real client call: the patch would assert "not called" while the run hangs or dies
somewhere unrelated. Nothing else fails on it, so this test fails instead.

It reads the patch targets out of the gate sources rather than listing them, so a new
``mock.patch.object(buildings_qa, ...)`` in a gate is covered here automatically.
"""
from __future__ import annotations

import ast
import unittest
from pathlib import Path

from tools.harness.backends import buildings_qa, scene_shot

HERE = Path(__file__).parent
CASE = (
    (buildings_qa, ("test_buildings_qa.py", "test_lifecycle_phases.py")),
    (scene_shot, ("test_scene_shot.py", "test_unloaded_eye.py")),
)


def _entry_name(module) -> str:
    return module.__name__.rsplit(".", 1)[-1]


def _file(module) -> Path:
    return Path(module.__file__)


def _defined(module) -> set[str]:
    """Top-level names the module defines itself (functions, classes, assignments)."""
    names: set[str] = set()
    for node in ast.parse(_file(module).read_text(encoding="utf-8")).body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    names.add(target.id)
                elif isinstance(target, ast.Tuple):
                    names.update(item.id for item in target.elts if isinstance(item, ast.Name))
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
    return names


def _imported(module) -> set[str]:
    """Names the module imports (so a seam that comes from the framework has one home too)."""
    names: set[str] = set()
    for node in ast.parse(_file(module).read_text(encoding="utf-8")).body:
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            names.update(alias.asname or alias.name.rsplit(".", 1)[-1] for alias in node.names)
    return names


def _bound(module) -> set[str]:
    return _defined(module) | _imported(module)


def patched(entry, sources: tuple[str, ...]) -> tuple[set[str], set[str]]:
    """(names patched on the module object, names patched through ``tools.harness.backends.<entry>``).

    Only the first kind is a seam: a ``patch.object(entry, name)`` replaces the attribute the owner
    modules read as ``_qa.<name>``. A path patch such as
    ``patch("tools.harness.backends.scene_shot.execute")`` is honoured by callers outside the package,
    which read the attribute at call time.
    """
    exact: set[str] = set()
    prefixed: set[str] = set()
    by_path: set[str] = set()
    for source in sources:
        tree = ast.parse((HERE / source).read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue
            func = node.func
            selector = node.args[0] if node.args else None
            if isinstance(func, ast.Attribute) and func.attr in ("object", "multiple"):
                if not (isinstance(selector, ast.Name) and selector.id == _entry_name(entry)):
                    continue
                if func.attr == "multiple":
                    exact.update(keyword.arg for keyword in node.keywords if keyword.arg)
                elif len(node.args) > 1:
                    choice = node.args[1]
                    if isinstance(choice, ast.Constant) and isinstance(choice.value, str):
                        exact.add(choice.value)
                    elif isinstance(choice, ast.JoinedStr):
                        head = choice.values[0]
                        if isinstance(head, ast.Constant) and isinstance(head.value, str):
                            prefixed.add(head.value)
            elif isinstance(func, ast.Name) and func.id == "patch" and selector is not None:
                if (isinstance(selector, ast.Constant) and isinstance(selector.value, str)
                        and selector.value.startswith(f"{entry.__package__}.{_entry_name(entry)}.")):
                    by_path.add(selector.value.rsplit(".", 1)[1])
    names = set(exact)
    for owner in entry._OWNER_MODULES:
        names.update(name for name in _bound(owner)
                     if any(name.startswith(prefix) for prefix in prefixed))
    return names, by_path


class SeamReachTests(unittest.TestCase):
    def test_every_patched_name_is_bound_by_one_owner_and_forwarded(self):
        for entry, sources in CASE:
            seams, by_path = patched(entry, sources)
            self.assertTrue(seams, f"{_entry_name(entry)}: no patch targets found in the gates")
            for name in sorted(seams | by_path):
                with self.subTest(entry=_entry_name(entry), name=name):
                    definitions = [owner.__name__ for owner in entry._OWNER_MODULES
                                   if name in _defined(owner)]
                    if definitions:
                        self.assertEqual(len(definitions), 1, f"{name} is defined by {definitions}")
                    else:  # a framework name such as run_split: exactly one owner imports it
                        homes = [owner.__name__ for owner in entry._OWNER_MODULES
                                 if name in _imported(owner)]
                        self.assertEqual(len(homes), 1, f"{name} is imported by {homes}")
                    self.assertTrue(getattr(entry, name) is not None,
                                    f"{entry.__name__}.{name} does not forward its owner's name")

    def test_an_owner_reaches_a_seam_only_through_the_entry_module(self):
        for entry, sources in CASE:
            seams, _by_path = patched(entry, sources)
            holder = {}
            for owner in entry._OWNER_MODULES:
                for name in _defined(owner) & seams:
                    holder[name] = owner.__name__
                for name in _imported(owner) & seams:
                    holder.setdefault(name, owner.__name__)
            for owner in entry._OWNER_MODULES:
                source = _file(owner).read_text(encoding="utf-8")
                tree = ast.parse(source)
                outward = {name for name, home in holder.items() if home != owner.__name__}
                for node in ast.walk(tree):
                    if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load):
                        with self.subTest(owner=owner.__name__, line=node.lineno):
                            self.assertNotIn(node.id, outward,
                                             f"{owner.__name__}:{node.lineno} binds {node.id!r}; "
                                             f"reach it as _qa.{node.id}")
                    elif isinstance(node, ast.Import):
                        with self.subTest(owner=owner.__name__, line=node.lineno):
                            self.assertEqual({alias.name for alias in node.names} & outward, set(),
                                             f"{owner.__name__}:{node.lineno} imports a seam")
                    elif isinstance(node, ast.ImportFrom):
                        imported = {alias.asname or alias.name.rsplit(".", 1)[-1]
                                    for alias in node.names}
                        with self.subTest(owner=owner.__name__, line=node.lineno):
                            self.assertEqual(imported & outward, set(),
                                             f"{owner.__name__}:{node.lineno} imports a seam from "
                                             f"another owner; reach it as _qa.<name>")
                for name in sorted(_defined(owner) & seams):
                    rebinds = [node.lineno for node in ast.walk(tree)
                               if isinstance(node, ast.Assign)
                               and any(isinstance(target, ast.Name) and target.id == name
                                       for target in node.targets)]
                    self.assertEqual(rebinds, [], f"{owner.__name__} reassigns the seam {name!r}")

    def test_owner_modules_define_no_duplicate_names(self):
        for entry, _sources in CASE:
            seen: dict[str, str] = {}
            for owner in entry._OWNER_MODULES:
                for name in sorted(_defined(owner)):
                    with self.subTest(owner=owner.__name__, name=name):
                        self.assertNotIn(name, seen,
                                         f"{name} is defined by {seen.get(name)} and {owner.__name__}")
                        seen[name] = owner.__name__


if __name__ == "__main__":
    unittest.main()
