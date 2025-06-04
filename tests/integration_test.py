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
        self.fs.create_dir('/etc')
        self.fs.create_dir(Path.home())

    def test_basic_config_through_cli(self):
        config = os.path.join(fixture_path, 'basic_config', 'basic_config.yaml')
        argv = ['dot-conf', config, '--user-only']
        with mock.patch.object(sys, 'argv', argv):
            main()
        self.assertTrue(Path(Path.home(), '.vimrc').is_symlink())
        self.assertTrue(Path(Path.home(), '.bashrc').is_symlink())
        self.assertTrue(Path(Path.home(), '.config/backup').is_dir())

    def test_mixed_config_as_root(self):
        config = os.path.join(fixture_path, 'user_and_sys', 'user_and_sys.yaml')
        argv = ['dot-conf', config]
        with mock.patch.object(sys, 'argv', argv), \
             mock.patch('os.geteuid', return_value=0):
            main()

        self.assertTrue(Path(Path.home(), '.vimrc').is_symlink())
        self.assertTrue(Path('/etc/bashrc').is_symlink())

    def test_multiple_configs(self):
        config1 = os.path.join(fixture_path, 'basic_config', 'basic_config.yaml')
        config2 = os.path.join(fixture_path, 'sys_only', 'sys_only.yaml')
        argv = ['dot-conf', config1, config2]
        with mock.patch.object(sys, 'argv', argv), \
             mock.patch('os.geteuid', return_value=0):
            main()

        self.assertTrue(Path(Path.home(), '.vimrc').is_symlink())
        self.assertTrue(Path('/etc/vimrc').is_symlink())
        self.assertTrue(Path(Path.home(), '.config/backup').is_dir())

if __name__ == '__main__':
    unittest.main()
