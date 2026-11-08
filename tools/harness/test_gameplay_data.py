import unittest

from .backends.gameplay import select_vanilla_data


class SurvivalVanillaDataTest(unittest.TestCase):
    def test_override_and_default(self):
        config = 'vanilla_data_dir = "data/vanilla"\n'
        self.assertEqual(select_vanilla_data(config, None), config)
        self.assertEqual(select_vanilla_data(config, '/cache/26.3'),
                         'vanilla_data_dir = "/cache/26.3"\n')

    def test_rejects_missing_or_ambiguous_token(self):
        with self.assertRaises(RuntimeError):
            select_vanilla_data('', '/cache')
        with self.assertRaises(RuntimeError):
            select_vanilla_data('vanilla_data_dir = "data/vanilla"\n' * 2, '/cache')
