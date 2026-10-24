"""Package the CLI, offline manual, demos, sources, and license notices."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist/yysort')
    parser.add_argument('--exe', type=Path, default=ROOT / 'target/release' / ('yysort.exe' if sys.platform == 'win32' else 'yysort'))
    parser.add_argument('--test', action='store_true')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(args.exe, output)
    files = ['Cargo.toml', 'Cargo.lock', 'README.md', 'VALIDATION.md', 'NOTICE.md',
             'COPYING', 'COPYING.EXCEPTION', '.gitignore', '.gitattributes']
    directories = ['src', 'tests', 'tools', 'demo', 'docs', 'help', '.github']
    source = output / 'source'
    source.mkdir(exist_ok=True)
    for name in files:
        shutil.copy2(ROOT / name, source / name)
        if name.endswith('.md') or name.startswith('COPYING'):
            shutil.copy2(ROOT / name, output / name)
    for name in directories:
        shutil.copytree(ROOT / name, source / name, dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns('target', '__pycache__', '*.pyc'))
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--locked', '--offline'], cwd=ROOT))
    encoding = next(package for package in metadata['packages'] if package['name'] == 'yy-encoding')
    if not (encoding['source'] or '').startswith('git+https://github.com/ykwyuta/YYLibrary'):
        raise RuntimeError('yy-encoding must come from the pinned YYLibrary Git dependency')
    library = Path(encoding['manifest_path']).parents[2]
    vendor = source / 'vendor/YYLibrary'
    vendor.mkdir(parents=True, exist_ok=True)
    for name in ['Cargo.toml', 'Cargo.lock', 'README.md', 'COPYING', 'COPYING.EXCEPTION']:
        shutil.copy2(library / name, vendor / name)
    for name in ['crates', 'tools']:
        shutil.copytree(library / name, vendor / name, dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns('target', '.git', '__pycache__', '*.pyc'))
    for name in ['help', 'demo', 'docs']:
        shutil.copytree(ROOT / name, output / name, dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    subprocess.run([sys.executable, str(ROOT / 'tools/licenses.py'), str(output / 'licenses')], check=True, cwd=ROOT)
    if args.test:
        with tempfile.TemporaryDirectory(prefix='yysort-package-', dir=output.parent) as temporary:
            work = Path(temporary)
            exe = output / args.exe.name
            manual = subprocess.check_output([str(exe), '--manual'], text=True, encoding='utf-8')
            if 'OUTFIL' not in manual:
                raise RuntimeError('Packaged manual is incomplete')
            for name, options in [('sample.csv', ['--format', 'csv', '--header', '--key', '1:TEXT:A', '--sum', '2:NUM']),
                                  ('sample.fixed', ['--record-length', '12', '--control', str(output / 'demo/sort.ctl')])]:
                subprocess.run([str(exe), str(output / 'demo' / name), '-o', str(work / name), *options], check=True, cwd=work)
                if not (work / name).stat().st_size:
                    raise RuntimeError('Packaged demo produced no records')
        print('Packaged CSV/fixed demos and offline manual: OK')


if __name__ == '__main__':
    main()
