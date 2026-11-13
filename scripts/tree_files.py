"""The files a check of the tree reads.

A check run from a checkout reads git's tracked files. The commit hook runs
its checks in a copy of the staged files (`git checkout-index`), which is
no repository, and with GIT_DIR naming the repository the hook belongs to;
there a check is run with --walk and reads every file under the current
directory, which are exactly the staged files.

  listed(walk, directory, env)   (root, paths): with walk, the directory
                                 and every file under it; without, the top
                                 of the repository the directory is in and
                                 its tracked files. Paths are as git names
                                 them: from the root, with `/`.
  walk_flag(arguments)           True for ["--walk"], False for [], None
                                 for anything else (the caller prints its
                                 usage).

Loaded by path from the scripts that use it (scripts/check_dependabot.py,
scripts/check_test_names.py).
"""

import os
import pathlib
import subprocess

# Directories --walk does not enter: git's own files, build output and
# installed packages.
WALK_SKIPS = {".git", "target", "node_modules"}


class NotARepository(Exception):
    """A check that reads git's tracked files was run outside a repository."""

    def __init__(self):
        super().__init__("not in a git repository; --walk reads the files under this directory")


def walked(root):
    """Every file under root, sorted, as git would name it."""
    root = pathlib.Path(root)
    paths = []
    for directory, names, files in os.walk(root):
        names[:] = [name for name in names if name not in WALK_SKIPS]
        paths += [(pathlib.Path(directory) / name).relative_to(root).as_posix() for name in files]
    return sorted(paths)


def tracked(directory, env=None):
    """(the top of the repository directory is in, its tracked files)."""
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=directory, env=env, capture_output=True, text=True)
    if top.returncode != 0:
        raise NotARepository
    root = pathlib.Path(top.stdout.rstrip("\n"))
    listing = subprocess.run(["git", "ls-files", "-z"], cwd=root, env=env, capture_output=True, text=True, check=True).stdout
    return root, [path for path in listing.split("\0") if path]


def listed(walk, directory=".", env=None):
    if walk:
        root = pathlib.Path(directory).resolve()
        return root, walked(root)
    return tracked(directory, env)


def walk_flag(arguments):
    if arguments == []:
        return False
    if arguments == ["--walk"]:
        return True
    return None
