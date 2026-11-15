"""Prepare a minimal Docker context from one verified native Linux archive."""
import argparse
import json
import os
from pathlib import Path
import shutil
import tempfile
from package_release import ROOT
from release_smoke import verify_extract

def prepare(artifacts, output, allow_dirty=False):
    archives = list(artifacts.glob('*.tar.gz'))
    if len(archives) != 1:
        raise ValueError('exactly one Linux archive required')
    if output.exists():
        raise ValueError('new context directory required')
    with tempfile.TemporaryDirectory(prefix='kiarni-image-') as temporary:
        package = verify_extract(archives[0], Path(temporary))
        info = json.loads((package / 'BUILD_INFO.json').read_text())
        if os.environ.get('GITHUB_ACTIONS') == 'true' and (info['dirty'] or info['commit'] != os.environ.get('GITHUB_SHA')):
            raise ValueError('CI archive must match the clean workflow commit')
        if info['target'] != 'x86_64-unknown-linux-gnu' or (info['dirty'] and not allow_dirty):
            raise ValueError('clean native Linux archive required')
        output.mkdir(parents=True)
        shutil.copytree(package, output / 'package')
        shutil.copyfile(ROOT / 'deploy/docker/Dockerfile', output / 'Dockerfile')
    print('Verified Docker context prepared; no credentials or source checkout included.')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifact-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--allow-dirty', action='store_true', help='local QA only')
    args = parser.parse_args()
    prepare(args.artifact_dir, args.output, args.allow_dirty)
