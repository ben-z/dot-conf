import unittest
from dotconf.dcutils import absp, str2bool
from pathlib import Path

class TestDCUtils(unittest.TestCase):
    def test_absp_expands_home(self):
        path = absp('~')
        self.assertEqual(path, Path.home())

    def test_str2bool_true(self):
        for v in ['yes', 'true', 't', 'y', '1', True]:
            self.assertTrue(str2bool(v))

    def test_str2bool_false(self):
        for v in ['no', 'false', 'f', 'n', '0', False]:
            self.assertFalse(str2bool(v))

    def test_str2bool_invalid(self):
        with self.assertRaises(Exception):
            str2bool('maybe')

if __name__ == '__main__':
    unittest.main()
