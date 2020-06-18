#!/usr/bin/env python3

import pathlib
from setuptools import setup
from distutils.util import convert_path

current_dir = pathlib.Path(__file__).parent
readme = (current_dir / "README.md").read_text()

ver_dict = {}
ver_path = convert_path('src/version.py')
with open(ver_path) as ver_file:
    exec(ver_file.read(), ver_dict)

setup(name='dot-conf',
      version=ver_dict['__version__'],
      description='automatically manage dotfiles',
      long_description=readme,
      long_description_content_type="text/markdown",
      author='Ben Zhang',
      author_email='benzhangniu@gmail.com',
      url='https://www.python.org/sigs/distutils-sig/',
      python_requires='>=3.7',
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
