import argparse
import logging
from .dcconfig import DCConfig
from .dcutils import str2bool
from .version import __version__

logging.basicConfig(format='%(message)s')
logger = logging.getLogger('dot-conf')


def main():
    parser = argparse.ArgumentParser(description='Apply dot-conf configuration files')

    parser.add_argument('filenames', metavar='filename', type=str, nargs='+',
                        help='The name of a configuration files')
    parser.add_argument('--version', action='version', version='%(prog)s {}'.format(__version__))

    args = parser.parse_args()

    # TODO: figure out how to add verbose logging option (--verbose, -v)
    logger.setLevel(logging.DEBUG)

    for filename in args.filenames:
        config = DCConfig.from_yaml(filename)
        config.apply()

    logger.info('Done!')


if __name__ == "__main__":
    main()
