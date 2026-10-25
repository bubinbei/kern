"""Package the independent redb/uPlot prototype; preserve profiles and local COM settings."""
from __future__ import annotations
import argparse,hashlib,json,shutil,subprocess
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]

def copy_file(source,target):
    source,target=Path(source),Path(target)
    if target.is_file() and hashlib.sha256(source.read_bytes()).digest()==hashlib.sha256(target.read_bytes()).digest():
        return
    shutil.copyfile(source,target)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config',type=Path)
    args=parser.parse_args()
    bundle=ROOT/'dist/Kern-Prototype-0.1.0'
    bundle.mkdir(parents=True,exist_ok=True)
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1'],cwd=ROOT))
    inventory=[]
    for package in metadata['packages']:
        if package['source'] is None:continue
        source=Path(package['manifest_path']).parent
        folder=ROOT/'third_party/licenses'/f"{package['name']}-{package['version']}"
        notices=[]
        for path in sorted(source.iterdir()):
            if path.is_file() and path.name.upper().startswith(('LICENSE','COPYING','NOTICE','AUTHORS')):
                folder.mkdir(parents=True,exist_ok=True)
                target=folder/path.name;target.write_bytes(path.read_bytes().rstrip(b'\r\n')+b'\n');notices.append(target.relative_to(ROOT).as_posix())
        if package['name']=='zstd-sys':
            upstream=source/'zstd/LICENSE'
            target=folder/'LICENSE-ZSTD';target.write_bytes(upstream.read_bytes().rstrip(b'\r\n')+b'\n');notices.append(target.relative_to(ROOT).as_posix())
        if not notices:raise RuntimeError('Missing dependency notices: '+package['name'])
        inventory.append(dict(name=package['name'],version=package['version'],license=package['license'],selected='MIT + Unicode-3.0' if package['name']=='unicode-ident' else ('BSD-3-Clause' if package['name'] in ('zstd-safe','zstd-sys') else 'MIT'),source=f"https://crates.io/crates/{package['name']}/{package['version']}",notices=notices))
    (ROOT/'docs/dependencies.json').write_text(json.dumps(inventory,indent=2)+'\n',encoding='utf8')
    uplot=ROOT/'third_party/uplot-1.6.32'
    pinned=json.loads((uplot/'source.json').read_text())
    for name,digest in pinned['files'].items():assert hashlib.sha256((uplot/name).read_bytes()).hexdigest()==digest,name
    for name in ('kern-core.exe','kern-shell.exe','kern-launch.exe'):
        copy_file(ROOT/'target/release'/name,bundle/name)
    for name,exe in [('im2480','im2480.exe'),('processor','kern-processor.exe'),('charts','kern-charts.exe'),('csv','kern-recorder.exe'),('backup','kern-recorder.exe')]:
        folder=bundle/'plugins'/name;folder.mkdir(parents=True,exist_ok=True)
        copy_file(ROOT/'plugins'/name/'plugin.json',folder/'plugin.json')
        copy_file(ROOT/'target/release'/exe,folder/exe)
    config=bundle/'plugins/im2480/config.json'
    if not config.exists():
        value=json.loads((args.config if args.config else ROOT/'plugins/im2480/config.example.json').read_text(encoding='utf8'))
        config.write_text(json.dumps(value,indent=2)+'\n',encoding='utf8')
    subprocess.run([str(ROOT/'target/release/im2480.exe'),'check','--config',str(config)],check=True,capture_output=True)
    copy_file(ROOT/'plugins/im2480/config.example.json',config.with_name('config.example.json'))
    copy_file(ROOT/'plugins/calibration.example.json',bundle/'calibration.example.json')
    shutil.copytree(ROOT/'third_party/licenses',bundle/'third_party/licenses',dirs_exist_ok=True)
    dest=bundle/'third_party/uplot-1.6.32';dest.mkdir(parents=True,exist_ok=True)
    for name in ('LICENSE','source.json'):copy_file(uplot/name,dest/name)
    for rel in ['LICENSE',*[p.relative_to(ROOT).as_posix() for p in (ROOT/'docs').rglob('*') if p.is_file() and p.suffix in ('.md','.json')]]:
        source=ROOT/rel
        if source.exists():
            dest=bundle/rel;dest.parent.mkdir(parents=True,exist_ok=True);copy_file(source,dest)
    for name,command in {
        'Start-Kern.cmd':'kern-launch.exe',
        'Stop-Kern.cmd':'kern-core.exe stop --profile "%~dp0data\\prototype"',
        'Status.cmd':'kern-core.exe status --profile "%~dp0data\\prototype"',
        'Check-Data.cmd':'kern-core.exe check --profile "%~dp0data\\prototype"',
    }.items():
        (bundle/name).write_text('@echo off\ncd /d "%~dp0"\n'+command+'\nif errorlevel 1 pause\n',encoding='ascii')
    (bundle/'README.txt').write_text('KERN — первый сквозной прототип, не промышленная версия.\n\nStart-Kern.cmd — запуск ядра, IM2480, интерпретатора, графиков, CSV и резерва.\nОкно графиков использует установленный Edge/Chrome в режиме приложения. Интернет не нужен.\nКарты по умолчанию ТЕСТОВЫЕ; изменяйте их на вкладке Калибровки.\nCOM задаётся в plugins/im2480/config.json. Старые профили не используются.\n\nЗакрытие окна не останавливает запись. Stop-Kern.cmd — полная остановка.\nРабочие данные: data/prototype/measurements.redb\nCSV: data/prototype/exports/parameters.csv (служебные строки #commit пропускать).\nРезерв: папка и сжатие задаются в настройках «Восстановительный резерв»; по умолчанию data/prototype/backup/events.jsonl.\nПроверка Check-Data.cmd выполняется при остановленном ядре.\nПодробности: docs/PROTOTYPE.md. Сеть, реальные датчики, слабые ПК и power-loss ещё не приняты.\n',encoding='utf8')
    files={p.relative_to(bundle).as_posix():dict(bytes=p.stat().st_size,sha256=hashlib.sha256(p.read_bytes()).hexdigest()) for p in bundle.rglob('*') if p.is_file() and 'data' not in p.relative_to(bundle).parts and p.name!='manifest.json'}
    (bundle/'manifest.json').write_text(json.dumps(dict(version='0.1.0',files=files),indent=2)+'\n',encoding='utf8')
    print(json.dumps(dict(bundle=str(bundle),files=len(files))))

if __name__=='__main__':main()
