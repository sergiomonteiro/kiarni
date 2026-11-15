"""Download pinned Linux amd64 test tools; not included in the runtime image."""
import argparse
import hashlib
from pathlib import Path
import platform
import urllib.request

TOOLS = {
    'kind': ('https://github.com/kubernetes-sigs/kind/releases/download/v0.33.0/kind-linux-amd64',
             'aee6151561422756b764a4ae28e7f44cda5af5a9eead3cc9985112b1de8d8e0d'),
    'kubectl': ('https://dl.k8s.io/release/v1.37.0/bin/linux/amd64/kubectl',
                '6129359f4e1f3848a5572ccb0b26cf28b8ca08cef38c95a765b2f64a2c961a2f'),
}

def install(destination):
    if (platform.system(), platform.machine()) != ('Linux', 'x86_64'):
        raise ValueError('Linux amd64 test tools required')
    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    for name, (url, expected) in TOOLS.items():
        path = destination / name
        if path.exists():
            if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
                raise ValueError('existing test tool checksum mismatch')
        else:
            with urllib.request.urlopen(url, timeout=60) as response:
                data = response.read(80 * 1024 * 1024 + 1)
            if hashlib.sha256(data).hexdigest() != expected:
                raise ValueError('test tool checksum mismatch')
            with path.open('xb') as output:
                output.write(data)
        path.chmod(0o700)
        print(f'{name}: pinned checksum verified')

if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True,type=Path)
    install(parser.parse_args().output)
