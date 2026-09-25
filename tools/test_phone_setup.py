"""Phone setup file handling; synthetic files only."""
from pathlib import Path
import json, os, subprocess, sys, tempfile, textwrap, unittest

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


    def test_resume_finishes_only_the_last_step_after_maps_were_converted(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            base, game = temp / 'work/data', temp / 'work/disc'
            stage = base / 'installations' / ('e' * 32)
            (stage / 'assets/private').mkdir(parents=True)
            (stage / 'assets/private/game.json').write_text('{}')
            (stage / 'maps').mkdir()
            (stage / 'maps.json').write_text('[]')
            game.mkdir(parents=True)
            (game / 'default.xex').write_bytes(b'XEX2')
            calls = []
            with mock.patch('tools.asset_pipeline.customiser_setup.prepare',
                            lambda g, assets, report: calls.append((g, assets))):
                self.assertTrue(phone_setup.resume_final_step(base, game, lambda _: None))
            self.assertEqual(calls, [(game, stage / 'assets')])
            marker = json.loads((base / 'installation.json').read_text())
            self.assertEqual(marker['directory'], 'installations/' + stage.name)
            self.assertEqual(set(marker['outputs']), {'core', 'hud', 'character', 'environment', 'maps'})
            # An unfinished map conversion (or a published install) is not resumed.
            self.assertFalse(phone_setup.resume_final_step(base, game, lambda _: None))
            (base / 'installation.json').unlink()
            (stage / 'conversion').mkdir()
            self.assertFalse(phone_setup.resume_final_step(base, game, lambda _: None))

    def test_pipeline_links_work_without_os_link_like_termux(self):
        # Termux's Python has no os.link; Python 3.13 also drops
        # Path.hardlink_to at import time. Run a fresh interpreter like that.
        root = Path(__file__).resolve().parents[1]
        script = textwrap.dedent(f"""
            import os, sys, subprocess
            del os.link
            sys.path[:0] = [{str(root / 'tools/phone_compat')!r}, {str(root)!r}]
            import skate_phone_compat
            from pathlib import Path
            from tools.asset_pipeline.customisation_catalog import write_private
            temp = Path(sys.argv[1])
            assert write_private(temp / 'a/staged.bin', b'data') == 'written'
            assert write_private(temp / 'a/staged.bin', b'data') == 'reused'
            (temp / 'src').write_bytes(b'shared')
            (temp / 'copy').hardlink_to(temp / 'src')
            assert (temp / 'copy').read_bytes() == b'shared'
            try:
                os.link(temp / 'src', temp / 'copy')
            except FileExistsError:
                pass
            else:
                raise AssertionError('link replaced an existing file')
            env = dict(os.environ, PYTHONPATH={str(root / 'tools/phone_compat')!r})
            # Workers load the shim via sitecustomize before any user code.
            child = ('import os, sys; assert "skate_phone_compat" in sys.modules; del os.link; '
                     'sys.modules["skate_phone_compat"].install(); os.link(%r, %r)') % (
                str(temp / 'src'), str(temp / 'child'))
            subprocess.run([sys.executable, '-c', child], env=env, check=True)
            assert (temp / 'child').read_bytes() == b'shared'
            print('ok')
        """)
        with tempfile.TemporaryDirectory() as temp:
            clean = {k: v for k, v in os.environ.items() if k != 'PYTHONPATH'}
            result = subprocess.run([sys.executable, '-c', script, temp], capture_output=True, text=True, env=clean)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), 'ok')


if __name__ == '__main__':
    unittest.main()
