import argparse
import logging
import sys
import os
import subprocess
from .dcconfig import DCConfig, Scope
from .dcutils import str2bool
from .version import __version__

logging.basicConfig(format='%(message)s')
logger = logging.getLogger('dot-conf')


def main():
    parser = argparse.ArgumentParser(description='Apply dot-conf configuration files')

    parser.add_argument('filenames', metavar='filename', type=str, nargs='+',
                        help='The name of a configuration files')
    parser.add_argument('--version', action='version', version='%(prog)s {}'.format(__version__))
    parser.add_argument('--sys-only', action='store_true')
    parser.add_argument('--user-only', action='store_true')

    args = parser.parse_args()

    # TODO: figure out how to add verbose logging option (--verbose, -v)
    logger.setLevel(logging.INFO)

    has_system_config = False

    for filename in args.filenames:
        config = DCConfig.from_yaml(filename)
        if args.sys_only:
            config.apply(scope=Scope.SYS)
        elif args.user_only:
            config.apply(scope=Scope.USER)
        elif not config.requires_root() or os.geteuid() == 0:
            config.apply(scope=Scope.ALL)
        else:
            config.apply(scope=Scope.USER)
            has_system_config = True

    if has_system_config and not args.user_only:
        print("Enter password here to apply system config:")
        subprocess_env = os.environ.copy()
        subprocess_env['DOTCONF_SUBPROCESS'] = 'true'
        subprocess.call(['sudo', '-E', sys.executable] + sys.argv + ['--sys-only'], env=subprocess_env)
        subprocess.call(['sudo', '-k']) # revoke sudo

    if os.environ.get('DOTCONF_SUBPROCESS') is None:
        logger.info('Done!')

if __name__ == "__main__":
    main()
