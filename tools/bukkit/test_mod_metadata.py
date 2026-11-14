import json
from pathlib import Path
import tempfile
import unittest
import zipfile

from convert import metadata
from mod_metadata import inspect_mod


class ModMetadataTests(unittest.TestCase):
    def inspect(self, files):
        with tempfile.TemporaryDirectory() as directory:
            jar = Path(directory) / "input.jar"
            with zipfile.ZipFile(jar, "w") as archive:
                for name, content in files.items():
                    archive.writestr(name, content)
            return inspect_mod(jar)

    def test_fabric_dependencies_entrypoints_and_required_mixin(self):
        report = self.inspect({
            "fabric.mod.json": json.dumps({
                "schemaVersion": 1, "id": "example", "version": "1.0",
                "environment": "*", "entrypoints": {"main": ["example.Main"],
                                                       "client": ["example.Client"]},
                "depends": {"minecraft": "1.21.1", "fabric-api": "*"},
                "mixins": [{"config": "mixins.json", "environment": "*"}],
                "accessWidener": "example.accesswidener",
                "jars": [{"file": "META-INF/jars/library.jar"}],
            }),
            "mixins.json": json.dumps({"required": True, "package": "example.mixin",
                                        "mixins": ["BlockMixin"], "client": ["ClientMixin"]}),
        })
        self.assertEqual(report["loader"], "fabric")
        self.assertEqual(report["minecraft_constraints"][0]["version"], "1.21.1")
        self.assertEqual([e["key"] for e in report["entrypoints"]], ["main", "client"])
        self.assertEqual(report["mixins"][0]["mixins"], ["BlockMixin"])
        self.assertEqual(report["mixins"][0]["client"], ["ClientMixin"])
        self.assertTrue(report["mixins"][0]["required"])
        self.assertEqual(report["access_transformers"], ["example.accesswidener"])
        self.assertEqual(report["embedded_jars"], ["META-INF/jars/library.jar"])
        self.assertIn("Embedded JARs were declared but not inspected.", report["warnings"])

    def test_forge_manifest_continuation_and_mixin(self):
        report = self.inspect({
            "META-INF/mods.toml": 'modLoader="javafml"\n[[mods]]\nmodId="testmod"\nversion="1"\n'
                '[[dependencies.testmod]]\nmodId="minecraft"\nmandatory=true\n'
                'versionRange="[1.20.1,1.21)"\nside="BOTH"\n',
            "META-INF/MANIFEST.MF": "Manifest-Version: 1.0\nMixinConfigs: mixins.\n test.json\n\n",
            "mixins.test.json": '{"required":true,"mixins":["Example"]}',
        })
        self.assertEqual(report["loader"], "forge")
        self.assertEqual(report["mod_ids"], ["testmod"])
        self.assertEqual(report["minecraft_constraints"][0]["side"], "BOTH")
        self.assertEqual(report["mixins"][0]["config"], "mixins.test.json")
        self.assertTrue(report["mixins"][0]["required"])

    def test_forge_reports_jarjar_dependencies(self):
        report = self.inspect({
            "META-INF/mods.toml": 'modLoader="javafml"\n[[mods]]\nmodId="testmod"\nversion="1"\n',
            "META-INF/jarjar/metadata.json": json.dumps({
                "jars": [{"path": "META-INF/jarjar/dependency.jar"}]
            }),
        })
        self.assertEqual(report["embedded_jars"], ["META-INF/jarjar/dependency.jar"])

    def test_declared_missing_mixin_is_not_silently_ignored(self):
        with self.assertRaisesRegex(ValueError, "Mixin config.*missing"):
            self.inspect({"fabric.mod.json": '{"id":"example","version":"1","mixins":["missing.json"]}'})

    def test_mod_is_not_misrepresented_as_a_converted_bukkit_plugin(self):
        with tempfile.TemporaryDirectory() as directory:
            jar = Path(directory) / "fabric.jar"
            with zipfile.ZipFile(jar, "w") as archive:
                archive.writestr("fabric.mod.json", '{"id":"example","version":"1"}')
            with self.assertRaises(ValueError):
                metadata(jar)


if __name__ == "__main__":
    unittest.main()
