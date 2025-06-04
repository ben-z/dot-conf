import os
import sys
import unittest
from pathlib import Path
from unittest import mock
from pyfakefs.fake_filesystem_unittest import TestCase

from dotconf.__main__ import main

fixture_path = os.path.join(os.path.dirname(__file__), 'fixtures')

class TestCLI(TestCase):
    def setUp(self):
        self.setUpPyfakefs()
        self.fs.add_real_directory(fixture_path)

    def test_basic_config_through_cli(self):
        config = os.path.join(fixture_path, 'basic_config', 'basic_config.yaml')
        argv = ['dot-conf', config, '--user-only']
        with mock.patch.object(sys, 'argv', argv):
            main()
        self.assertTrue(Path(Path.home(), '.vimrc').is_symlink())
        self.assertTrue(Path(Path.home(), '.bashrc').is_symlink())
        self.assertTrue(Path(Path.home(), '.config/backup').is_dir())

if __name__ == '__main__':
    unittest.main()
