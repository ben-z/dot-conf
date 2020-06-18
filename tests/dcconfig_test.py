import logging
import os
import unittest
from pathlib import Path
from pyfakefs.fake_filesystem_unittest import TestCase
from unittest import mock
from src.dcconfig import DCConfig, Scope
from src.dcutils import absp

logging.basicConfig()
logger = logging.getLogger('dot-conf')
logger.setLevel(logging.DEBUG)

fixture_path = os.path.join(os.path.dirname(
    __file__), 'fixtures')


class TestDCConfig(TestCase):
    def setUp(self):
        self.setUpPyfakefs()
        self.fs.add_real_directory(fixture_path)
        self.fs.create_dir('/etc')

    def test_loads_and_applies_basic_config(self):
        config = DCConfig.from_yaml(os.path.join(fixture_path, 'basic_config/basic_config.yaml'))
        self.assertEqual(config.backup_directory, absp('~/.config/backup'))
        self.assertEqual(config.symlinks[absp(
            fixture_path, 'basic_config', '.vimrc')], absp('~/.vimrc'))
        self.assertEqual(config.symlinks[absp(
            fixture_path, 'basic_config', '.bashrc')], absp('~/.bashrc'))
        config.apply(Scope.USER)

        self.assertTrue(Path(Path.home(), '.vimrc').is_symlink())
        self.assertEqual(Path(Path.home(), '.vimrc').resolve(),
                         Path(fixture_path, 'basic_config', '.vimrc').resolve())
        self.assertTrue(Path(Path.home(), '.bashrc').is_symlink())
        self.assertEqual(Path(Path.home(), '.bashrc').resolve(),
                         Path(fixture_path, 'basic_config', '.bashrc').resolve())
        self.assertTrue(Path(Path.home(), '.config/backup').is_dir())
        # wacky syntax from https://stackoverflow.com/a/54216885/4527337
        self.assertFalse(any(Path(Path.home(), '.config/backup').iterdir()))

    def test_loads_and_applies_sys_config(self):
        config = DCConfig.from_yaml(os.path.join(fixture_path, 'sys_only/sys_only.yaml'))
        self.assertEqual(config.backup_directory, absp('~/.config/backup'))
        self.assertEqual(config.sys_symlinks.get(absp(
            fixture_path, 'sys_only', '.vimrc')), absp('/etc/vimrc'))
        self.assertEqual(config.sys_symlinks.get(absp(
            fixture_path, 'sys_only', '.bashrc')), absp('/etc/bashrc'))
        config.apply(Scope.SYS)

        self.assertTrue(Path('/etc/vimrc').is_symlink())
        self.assertEqual(Path('/etc/vimrc').resolve(),
                         Path(fixture_path, 'sys_only', '.vimrc').resolve())
        self.assertTrue(Path('/etc/bashrc').is_symlink())
        self.assertEqual(Path('/etc/bashrc').resolve(),
                         Path(fixture_path, 'sys_only', '.bashrc').resolve())
        self.assertTrue(Path(Path.home(), '.config/backup').is_dir())
        # wacky syntax from https://stackoverflow.com/a/54216885/4527337
        self.assertFalse(any(Path(Path.home(), '.config/backup').iterdir()))

    def test_loads_and_applies_mixed_config(self):
        config = DCConfig.from_yaml(os.path.join(fixture_path, 'user_and_sys/user_and_sys.yaml'))
        self.assertEqual(config.backup_directory, absp('~/.config/backup'))
        self.assertEqual(config.symlinks.get(absp(
            fixture_path, 'user_and_sys', '.vimrc')), absp('~/.vimrc'))
        self.assertEqual(config.sys_symlinks.get(absp(
            fixture_path, 'user_and_sys', '.bashrc')), absp('/etc/bashrc'))
        config.apply(Scope.ALL)

        self.assertTrue(Path(Path.home(), '.vimrc').is_symlink())
        self.assertEqual(Path(Path.home(), '.vimrc').resolve(),
                         Path(fixture_path, 'user_and_sys', '.vimrc').resolve())
        self.assertTrue(Path('/etc/bashrc').is_symlink())
        self.assertEqual(Path('/etc/bashrc').resolve(),
                         Path(fixture_path, 'user_and_sys', '.bashrc').resolve())
        self.assertTrue(Path(Path.home(), '.config/backup').is_dir())
        # wacky syntax from https://stackoverflow.com/a/54216885/4527337
        self.assertFalse(any(Path(Path.home(), '.config/backup').iterdir()))

    @mock.patch('time.time', mock.MagicMock(return_value=12345))
    def test_backs_up_when_dest_files_exist(self):
        self.fs.create_file(absp('~/.vimrc'), contents='some-content')

        config = DCConfig.from_yaml(os.path.join(fixture_path, 'basic_config/basic_config.yaml'))
        config.apply(Scope.USER)
        self.assertTrue(Path(Path.home(), '.config/backup', '.vimrc.12345.bak').exists())
        with open(Path(Path.home(), '.config/backup', '.vimrc.12345.bak')) as f:
            self.assertEqual(f.read(), 'some-content')

    # TODO: test when the destination is a symlink
    # TODO: test when the destination is a symlink and the target does not exist


if __name__ == "__main__":

    unittest.main()
