#!/usr/bin/env python

import pathlib
from setuptools import setup

current_dir = pathlib.Path(__file__).parent
readme = (current_dir / "README.md").read_text()

setup(name='dot-conf',
      version='0.0.0',
      description='automatically manage dotfiles',
      long_description=readme,
      long_description_content_type="text/markdown",
      author='Ben Zhang',
      author_email='benzhangniu@gmail.com',
      url='https://www.python.org/sigs/distutils-sig/',
      install_requires=['strictyaml'],
      tests_require=['pyfakefs'],
      packages=['dc'],
      package_dir={'dc': 'src'},
      include_package_data=True,
      entry_points={
          "console_scripts": [
              "dot-conf=dc.__main__:main",
          ]
      },
      )
