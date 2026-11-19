import unittest

from tools.worldgen_diff.profile import samples


class ProfileReceiptTests(unittest.TestCase):
    def test_libtest_prefix_does_not_hide_first_sample(self):
        text = 'test terrain::profile_tests::parity_stages ... ' + '\n'.join(
            f'STAGES seed={seed} run=0 chunks=36 ms_per_chunk=12.5'
            for seed in (0, 81, 497625))
        self.assertEqual([row['seed'] for row in samples(text)], [0, 81, 497625])

    def test_missing_samples_cannot_be_reported_as_a_speedup(self):
        with self.assertRaisesRegex(ValueError, 'missing or unexpected'):
            samples('test result: ok\n')


if __name__ == '__main__':
    unittest.main()
