"""Phone setup file handling; synthetic files only."""
from pathlib import Path
import json, tempfile, unittest

from tools import phone_setup


class PhoneSetupTests(unittest.TestCase):
    def test_disc_names_are_respelled_for_case_sensitive_storage(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'DATA/BIG').mkdir(parents=True)
            (root / 'DATA/Content').mkdir()
            (root / 'Default.XEX').write_bytes(b'x')
            (root / 'DATA/BIG/MiscLoad.BIG').write_bytes(b'x')
            (root / 'DATA/Content/WorldDist_University.big').write_bytes(b'x')
            (root / 'DATA/Content/worldDIST_Hawaii.big').write_bytes(b'x')
            phone_setup.normalize_disc(root, lambda _: None)
            names = sorted(p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file())
            self.assertEqual(names, ['data/big/miscload.big', 'data/content/worldDIST_Hawaii.big',
                                     'data/content/worldDIST_University.big', 'default.xex'])

    def test_publish_moves_installation_and_replaces_the_old_one(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            work, shared = temp / 'work/data', temp / 'shared/data'
            new, old = 'a' * 32, 'b' * 32
            (work / 'installations' / new / 'assets/private').mkdir(parents=True)
            (work / 'installations' / new / 'assets/private/game.json').write_text('{}')
            (work / 'installation.json').write_text(json.dumps({'version': 1, 'directory': 'installations/' + new}))
            (shared / 'installations' / old).mkdir(parents=True)
            (shared / 'installation.json').write_text(json.dumps({'version': 1, 'directory': 'installations/' + old}))
            phone_setup.publish(work, shared, lambda _: None)
            self.assertTrue((shared / 'installations' / new / 'assets/private/game.json').is_file())
            self.assertFalse((shared / 'installations' / old).exists())
            self.assertFalse((work / 'installations' / new).exists())
            self.assertEqual(json.loads((shared / 'installation.json').read_text())['directory'], 'installations/' + new)

    def test_interrupted_runs_are_cleaned_but_the_current_install_is_kept(self):
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp) / 'data'
            keep, stale = 'c' * 32, 'd' * 32
            for name in (keep, stale):
                (base / 'installations' / name).mkdir(parents=True)
            (base / 'installation.json').write_text(json.dumps({'directory': 'installations/' + keep}))
            (base.parent / 'character-source-x').mkdir()
            phone_setup.clean_incomplete(base)
            self.assertTrue((base / 'installations' / keep).is_dir())
            self.assertFalse((base / 'installations' / stale).exists())
            self.assertFalse((base.parent / 'character-source-x').exists())


if __name__ == '__main__':
    unittest.main()
