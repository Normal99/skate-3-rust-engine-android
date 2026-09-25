"""Termux compatibility for the desktop conversion pipeline.

Termux's Python has no os.link: Android forbids hard links for apps. The
pipeline only uses links to share identical files or to publish a staged file
without overwriting another, so an exclusive copy keeps its behaviour.
Import this before pathlib: Python 3.13 decides at import time whether
Path.hardlink_to exists.
"""
import os
import shutil


def _link(src, dst, *, src_dir_fd=None, dst_dir_fd=None, follow_symlinks=True):
    if src_dir_fd is not None or dst_dir_fd is not None:
        raise NotImplementedError('link emulation does not support directory descriptors')
    # 'xb' refuses an existing destination exactly like link() does.
    with open(src, 'rb') as source, open(dst, 'xb') as target:
        shutil.copyfileobj(source, target, 1024 * 1024)


def install():
    if hasattr(os, 'link'):
        return False
    os.link = _link
    import pathlib
    pathlib.Path.hardlink_to = lambda self, target: os.link(target, self)
    return True


install()
