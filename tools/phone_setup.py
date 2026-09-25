"""Command-line setup for Android phones (Termux), Linux and macOS.

Converts the player's own Skate 3 Xbox 360 ISO into the `data` folder the
Android app reads. Conversion runs in a private work folder (a normal Linux
filesystem with file locks and exact-case names), then the finished
installation is moved to shared storage for the game.

    python phone_setup.py                      # uses the .iso in Download
    python phone_setup.py --iso /sdcard/Download/Skate3.iso
"""
import os, sys

# Termux compatibility (no os.link) for this process and the map workers it
# starts. Must run before pathlib is imported.
COMPAT = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'phone_compat')
os.environ['PYTHONPATH'] = os.pathsep.join(filter(None, [COMPAT, os.environ.get('PYTHONPATH')]))
sys.path.insert(0, COMPAT)
import skate_phone_compat  # noqa: E402,F401

from pathlib import Path  # noqa: E402
import argparse, json, re, shutil, subprocess, time  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

SHARED_DATA = Path('/storage/emulated/0/Skate3Rust/data')
# Disc paths the converters open by exact name. Windows ignores case; phones
# and Linux do not, so extracted names are respelled to match.
DISC_PATHS = ('default.xex', 'data/big/db.big', 'data/big/miscboot.big', 'data/big/miscload.big',
              'data/content/createacharacter.big', 'data/content/worlddmo.big', 'data/anim',
              'data/content/global_locators')
WORLD = re.compile(r'worlddist_(.+)\.big', re.IGNORECASE)


def respell(root, relative):
    """Rename each component of relative to its expected case if needed."""
    current = root
    for part in Path(relative).parts:
        exact = current / part
        if not exact.exists() and current.is_dir():
            match = next((p for p in current.iterdir() if p.name.lower() == part.lower()), None)
            if match is None:
                return
            match.rename(exact)
        current = exact


def normalize_disc(root, report):
    for relative in DISC_PATHS:
        respell(root, relative)
    content = root / 'data/content'
    if content.is_dir():
        for path in content.iterdir():
            match = WORLD.fullmatch(path.name)
            wanted = f'worldDIST_{match.group(1)}.big' if match else None
            if wanted and path.name != wanted:
                path.rename(content / wanted)
    report('Disc layout checked')


def check_native_refpack(report):
    """The converter zip ships an Android build of refpack_native.rs under the
    name fast_refpack.py loads. If this device cannot load it, fall back to
    the (much slower) pure-Python decoder instead of failing on import."""
    import ctypes
    library = ROOT / 'tools/asset_pipeline/refpack.dll'
    if not library.is_file():
        report('Fast decompressor not bundled; using the slower Python decoder')
        return
    try:
        ctypes.CDLL(str(library))
    except OSError as error:
        library.rename(library.with_name('refpack-unusable.dll'))
        report(f'Fast decompressor unusable here ({error}); using the slower Python decoder')


def resume_final_step(work_base, game, report):
    """Finish a run that failed only in its last step.

    Desktop setup prepares the character customiser after every map is
    converted and cleaned up. If that step failed (for example on a Termux
    limitation), keep the converted maps and finish just that step, then
    publish the installation record exactly as install.py would.
    """
    installations = work_base / 'installations'
    if (work_base / 'installation.json').exists() or not installations.is_dir() \
            or not (game / 'default.xex').is_file():
        return False
    stages = [p for p in installations.iterdir()
              if re.fullmatch(r'[0-9a-f]{32}', p.name) and (p / 'maps.json').is_file()
              and (p / 'assets/private/game.json').is_file() and not (p / 'conversion').exists()]
    if len(stages) != 1:
        return False
    stage = stages[0]
    report('Resuming: maps are already converted, finishing character preparation')
    from tools.asset_pipeline.customiser_setup import prepare
    from tools.asset_pipeline.group_receipts import record
    from tools.asset_pipeline.install import digest
    from tools.asset_pipeline.optional_content import summary
    from tools.asset_pipeline.setup_state import setup_lock
    from tools.asset_pipeline.versions import GROUPS, fingerprints
    with setup_lock(work_base):
        prepare(game, stage / 'assets', report)
        summary(stage)
        marker = work_base / 'installation.json.new'
        marker.write_text(json.dumps({
            'version': 1, 'directory': 'installations/' + stage.name,
            'source': str((game / 'default.xex').resolve()), 'source_hash': digest(game / 'default.xex'),
            'pipelines': fingerprints(), 'outputs': {group: record(stage, group) for group in GROUPS}}),
            encoding='utf-8')
        marker.replace(work_base / 'installation.json')
    return True


def clean_incomplete(base):
    """Remove leftovers of an interrupted run (Android may kill Termux)."""
    marker = base / 'installation.json'
    keep = None
    if marker.is_file():
        try:
            keep = json.loads(marker.read_text(encoding='utf-8')).get('directory')
        except ValueError:
            pass
    installations = base / 'installations'
    if installations.is_dir():
        for path in installations.iterdir():
            if re.fullmatch(r'[0-9a-f]{32}', path.name) and f'installations/{path.name}' != keep:
                shutil.rmtree(path, ignore_errors=True)
    for path in base.parent.glob('character-source-*'):
        shutil.rmtree(path, ignore_errors=True)


def move_tree(source, destination, report):
    """Copy file by file, deleting each source as it lands, so the phone never
    holds two full copies. Shared storage rejects metadata copies."""
    files = [p for p in source.rglob('*') if p.is_file()]
    total = sum(p.stat().st_size for p in files) or 1
    done, last = 0, 0.0
    for path in files:
        target = destination / path.relative_to(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        done += path.stat().st_size
        path.unlink()
        if time.monotonic() - last > 5:
            report(f'Moving to shared storage: {100 * done // total}%')
            last = time.monotonic()
    shutil.rmtree(source, ignore_errors=True)


def publish(work_base, shared_base, report):
    marker = json.loads((work_base / 'installation.json').read_text(encoding='utf-8'))
    directory = marker['directory']
    if not re.fullmatch(r'installations/[0-9a-f]{32}', directory):
        raise RuntimeError('Unexpected installation layout')
    shared_base.mkdir(parents=True, exist_ok=True)
    previous = shared_base / 'installation.json'
    old = None
    if previous.is_file():
        try:
            old = json.loads(previous.read_text(encoding='utf-8')).get('directory')
        except ValueError:
            pass
    move_tree(work_base / directory, shared_base / directory, report)
    # Publish the marker last: the game only sees a complete installation.
    temporary = shared_base / 'installation.json.new'
    temporary.write_text(json.dumps(marker), encoding='utf-8')
    temporary.replace(previous)
    if old and old != directory and re.fullmatch(r'installations/[0-9a-f]{32}', old):
        shutil.rmtree(shared_base / old, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--iso', type=Path,
                        help='Skate 3 Xbox 360 ISO, or default.xex in an extracted game folder '
                             '(default: the only .iso in the phone\'s Download folder)')
    parser.add_argument('--out', type=Path, default=SHARED_DATA,
                        help=f'data folder the game reads (default {SHARED_DATA})')
    parser.add_argument('--work', type=Path, default=Path.home() / 'skate3-setup',
                        help='private conversion folder (default ~/skate3-setup)')
    parser.add_argument('--workers', type=int, default=1,
                        help='maps converted at once; each needs several GB of RAM (default 1)')
    args = parser.parse_args()

    started = time.monotonic()

    def report(text):
        minutes = int(time.monotonic() - started) // 60
        print(f'[{minutes:3d} min] {text}', flush=True)

    try:
        import numpy, PIL  # noqa: F401
    except ImportError:
        sys.exit('Missing Python packages. In Termux run: pkg install python-numpy python-pillow')
    if args.iso is None:
        found = sorted(p for folder in (Path('/storage/emulated/0/Download'), Path('/storage/emulated/0'))
                       if folder.is_dir() for p in folder.glob('*') if p.suffix.lower() == '.iso')
        if len(found) != 1:
            listed = ''.join(f'\n  {p}' for p in found) or ' none found'
            sys.exit('Put your Skate 3 ISO in the Download folder, or pass --iso PATH. ISO files:' + listed)
        args.iso = found[0]
    iso = args.iso.expanduser().resolve()
    if not iso.exists():
        sys.exit(f'Not found: {iso}')
    if shutil.which('termux-wake-lock'):
        # Stops Android from pausing the conversion when the screen turns off.
        subprocess.run(['termux-wake-lock'], check=False)
    os.environ['SKATE_MAP_WORKERS'] = str(max(1, args.workers))

    # The desktop setup load-tests its output with the Windows game. There is
    # none here, so each check runs `true`; the converters still verify input.
    engine_check = shutil.which('true')
    if engine_check is None:
        sys.exit('The `true` command is missing (install coreutils)')
    check_native_refpack(report)
    work = args.work.expanduser().resolve()
    work_base = work / 'data'
    work_base.mkdir(parents=True, exist_ok=True)
    from_iso = iso.is_file() and iso.suffix.lower() == '.iso'
    game = work / 'disc' if from_iso else (iso if iso.is_dir() else iso.parent)
    try:
        if not resume_final_step(work_base, game, report):
            clean_incomplete(work_base)
            from tools.asset_pipeline.customiser_setup import install
            from tools.asset_pipeline.versions import installed
            refresh = installed(work_base) is not None
            report('Updating existing conversion' if refresh
                   else 'Starting conversion (on a phone this can take a few hours; keep Termux open)')
            if from_iso:
                from tools.asset_pipeline.xiso import extract
                shutil.rmtree(game, ignore_errors=True)
                report('Extracting your ISO')
                extract(iso, game, report)
            normalize_disc(game, report)
            install(game / 'default.xex', work_base, Path(engine_check), report, refresh=refresh)
    except Exception as error:
        import traceback
        log = work_base / 'setup-error.log'
        log.write_text(traceback.format_exc(), encoding='utf-8')
        sys.exit(f'\nSetup failed: {error}\nDetails: {log}\n'
                 'Your ISO is unchanged. Fix the problem and run the same command again.')
    out = args.out.expanduser()
    report(f'Publishing to {out}')
    publish(work_base, out, report)
    shutil.rmtree(work / 'disc', ignore_errors=True)
    shutil.rmtree(work_base, ignore_errors=True)
    report(f'Done. Open Skate 3 Rust and press Start. Game data: {out}')
    if shutil.which('termux-wake-unlock'):
        subprocess.run(['termux-wake-unlock'], check=False)


if __name__ == '__main__':
    main()
