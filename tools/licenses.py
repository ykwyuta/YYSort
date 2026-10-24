"""Bundle the license/notice files for every resolved Cargo dependency."""
import json
from pathlib import Path
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
host = next(line.removeprefix('host: ') for line in subprocess.check_output(['rustc', '-vV'], text=True).splitlines() if line.startswith('host: '))
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--manifest-path',
    str(root/'Cargo.toml'), '--format-version', '1', '--locked', '--offline', '--filter-platform', host]))
nodes = {node['id']: node for node in metadata['resolve']['nodes']}
reachable, pending = set(), [metadata['resolve']['root']]
while pending:
    node = pending.pop()
    if node in reachable:
        continue
    reachable.add(node)
    pending.extend(dep['pkg'] for dep in nodes[node]['deps'])
target = Path(sys.argv[1]).resolve()
target.mkdir(parents=True, exist_ok=True)
index = []
for package in metadata['packages']:
    if not package['source'] or package['id'] not in reachable:
        continue
    folder = Path(package['manifest_path']).parent
    destination = target/f"{package['name']}-{package['version']}"
    destination.mkdir(exist_ok=True)
    files = []
    for file in folder.iterdir():
        if file.is_file() and file.name.upper().startswith(('LICENSE', 'COPYING', 'COPYRIGHT', 'NOTICE', 'UNLICENSE', 'AUTHORS')):
            shutil.copyfile(file, destination/file.name)
            files.append(file.name)
    if not files:
        raise SystemExit(f'No license text found: {package["name"]}')
    index.append(dict(name=package['name'],version=package['version'],license=package['license'],files=files))
(target/'index.json').write_text(json.dumps(index, indent=2)+'\n', encoding='utf-8')
print(f'License/notice files for {len(index)} dependencies copied to {target}')
