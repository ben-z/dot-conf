import logging
import os
import unittest
from pathlib import Path
from pyfakefs.fake_filesystem_unittest import TestCase
from src.dcconfig import DCConfig
from src.dcutils import absp

logging.basicConfig(level=logging.DEBUG)

fixture_path = os.path.join(os.path.dirname(
    __file__), 'fixtures')


class TestDCConfig(TestCase):
    def setUp(self):
        self.setUpPyfakefs()
        self.fs.add_real_directory(fixture_path)

    def test_loads_and_applies_basic_config(self):
        config = DCConfig.from_yaml(os.path.join(fixture_path, 'basic_config/basic_config.yaml'))
        self.assertEqual(config.backup_directory, absp('~/.config/backup'))
        self.assertEqual(config.symlinks[absp(
            fixture_path, 'basic_config', '.vimrc')], absp('~/.vimrc'))
        self.assertEqual(config.symlinks[absp(
            fixture_path, 'basic_config', '.bashrc')], absp('~/.bashrc'))
        config.apply()

        # Test backup when dest file exists


if __name__ == "__main__":

    unittest.main()
