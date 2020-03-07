#!/usr/bin/env python

from distutils.core import setup

setup(name='dot-conf',
      version='0.0.0',
      description='automatically manage dotfiles',
      author='Ben Zhang',
      author_email='benzhangniu@gmail.com',
      url='https://www.python.org/sigs/distutils-sig/',
      install_requires=['strictyaml'],
      tests_require=['pyfakefs'],
      packages=['dot-conf'],
      package_dir={'dot-conf': 'src'},
      )
