"""Create a local Windows core bundle; never publishes a release or includes runtime data."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
VERSION = '0.1.0'


def main():
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1'], cwd=ROOT))
    notices = ROOT / 'third_party/licenses'
    inventory = []
    for package in metadata['packages']:
        if package['source'] is None:
            continue
        source = Path(package['manifest_path']).parent
        destination = notices / (package['name'] + '-' + package['version'])
        copied = []
        for p in sorted(source.iterdir()):
            if p.is_file() and p.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE', 'AUTHORS')):
                destination.mkdir(parents=True, exist_ok=True)
                target = destination / p.name
                # Preserve the complete legal text; normalize redundant blank lines at EOF.
                target.write_bytes(p.read_bytes().rstrip(b'\r\n') + b'\n')
                copied.append(target.relative_to(ROOT).as_posix())
        if not copied:
            raise RuntimeError('Missing notices for ' + package['name'])
        inventory.append({'name': package['name'], 'version': package['version'], 'license': package['license'],
                          'selected': 'MIT + Unicode-3.0' if package['name']=='unicode-ident' else 'MIT',
                          'source': f"https://crates.io/crates/{package['name']}/{package['version']}", 'notices': copied})
    (ROOT/'docs/dependencies.json').write_text(json.dumps(inventory,indent=2)+'\n',encoding='utf-8')
    bundle = ROOT / 'dist' / ('Kern-Core-' + VERSION)
    bundle.mkdir(parents=True, exist_ok=True)
    # Only fixed, project-owned release binaries/docs are copied; no profile or directory cleanup.
    for name in ['kern-core.exe', 'kern-demo.exe']:
        binary = ROOT / 'target/release' / name
        if not binary.is_file():
            raise RuntimeError('Build Windows release binaries first')
        shutil.copyfile(binary,bundle/name)
    for rel in ['LICENSE','docs/CORE_GUIDE.md','docs/en/CORE_GUIDE.md','docs/CORE_INTERNALS.md','docs/en/CORE_INTERNALS.md','docs/CORE_VALIDATION.md','docs/en/CORE_VALIDATION.md','docs/ARCHITECTURE.md','docs/en/ARCHITECTURE.md','docs/ROADMAP.md','docs/en/ROADMAP.md','docs/DEPENDENCIES.md','docs/en/DEPENDENCIES.md','docs/dependencies.json','docs/validation/core-0.1.0.json','docs/validation/core-resources-0.1.0.json']:
        target=bundle/rel;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/rel,target)
    shutil.copytree(notices,bundle/'third_party/licenses',dirs_exist_ok=True)
    commands = {
        'Start-Core.cmd': 'kern-core.exe serve --profile "%~dp0data\\default"',
        'Run-Demo.cmd': 'kern-demo.exe --profile "%~dp0data\\default" --id demo-1 --channels 6 --count 100 --period-ms 100',
        'Status.cmd': 'kern-core.exe status --profile "%~dp0data\\default"',
        'Stop-Core.cmd': 'kern-core.exe stop --profile "%~dp0data\\default"',
    }
    for name, command in commands.items():
        (bundle/name).write_text('@echo off\ncd /d "%~dp0"\n'+command+'\npause\n',encoding='ascii')
    (bundle/'README.txt').write_text('KERN CORE 0.1.0 — первый headless-срез, не промышленная версия.\n\n1. Start-Core.cmd — ядро.\n2. Run-Demo.cmd — искусственные измерения в другой консоли.\n3. Status.cmd — состояние.\n4. Stop-Core.cmd — штатная остановка.\n\nПодробное руководство: docs/CORE_GUIDE.md.\nДанные создаются только в data/default этого комплекта. Реальные приборы не используются.\nПервое графическое меню ещё не реализовано. Проверено на Windows 11; Windows 10 и длительная работа впереди.\n',encoding='utf-8')
    files={p.relative_to(bundle).as_posix():{'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
           for p in bundle.rglob('*') if p.is_file() and 'data' not in p.relative_to(bundle).parts and p.name!='manifest.json'}
    (bundle/'manifest.json').write_text(json.dumps({'version':VERSION,'files':files},indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'bundle':bundle.relative_to(ROOT).as_posix(),'dependency_packages':len(inventory),'files':len(files)}))


if __name__=='__main__':
    main()
