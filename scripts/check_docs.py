"""Check local Markdown links and coverage of the implemented HTTP/config contracts."""
from pathlib import Path
import re
import subprocess
import unicodedata
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]

def anchors(text):
    result, counts = set(), {}
    fenced = False
    for line in text.splitlines():
        if line.startswith('```'):
            fenced = not fenced
        if fenced:
            continue
        match = re.match(r'^#{1,6}\s+(.+?)\s*#*$', line)
        if not match:
            continue
        title = re.sub(r'\[([^\]]+)\]\([^)]*\)', r'\1', match[1]).lower()
        slug = ''.join(char for char in title if char in ' -_' or unicodedata.category(char)[0] in 'LN').replace(' ', '-')
        count = counts.get(slug, 0)
        counts[slug] = count + 1
        result.add(slug if count == 0 else f'{slug}-{count}')
    result.update(re.findall(r'<a\s+(?:id|name)=[\"\']([^\"\']+)', text))
    return result

def check():
    paths = subprocess.check_output(['rg', '--files', '--hidden', '-g', '*.md', '-g', '!.git', '-g', '!target', '-g', '!node_modules'], cwd=ROOT, text=True).splitlines()
    errors, links = [], 0
    for name in sorted(paths):
        source = ROOT / name
        # Ignore fenced examples when locating inline Markdown links.
        text = re.sub(r'^```[^\n]*\n.*?^```\s*$', '', source.read_text(), flags=re.M | re.S)
        for match in re.finditer(r'\[[^\]\n]+\]\(([^\s)]+)(?:\s+"[^"]*")?\)', text):
            raw = match[1].strip('<>')
            url = urlsplit(raw)
            if url.scheme or url.netloc:
                continue
            target = (ROOT / unquote(url.path).lstrip('/')) if url.path.startswith('/') else (source.parent / unquote(url.path) if url.path else source)
            links += 1
            if not target.exists():
                errors.append(f'{name}: missing {raw}')
            elif url.fragment and target.is_file() and target.suffix == '.md' and unquote(url.fragment) not in anchors(target.read_text()):
                errors.append(f'{name}: missing anchor {raw}')
    api = (ROOT / 'docs/API.md').read_text()
    catalog = re.findall(r'\w+ => \("([a-z_]+)", (\d+), "([^"]+)"\)', (ROOT / 'crates/http/src/catalog.rs').read_text())
    if not catalog:
        errors.append('HTTP catalog could not be read')
    documented_errors = re.findall(r'^\| `([a-z_]+)` \| (\d+) \| ([^\n]+) \|$', api, re.M)
    if sorted(catalog) != sorted(documented_errors):
        errors.append('API error catalog inventory mismatch')
    for code, status, message in catalog:
        if f'| `{code}` | {status} | {message} |' not in api:
            errors.append(f'API catalog mismatch: {code}')
    config_source = (ROOT / 'crates/config/src/lib.rs').read_text().split('fn known(')[1].split('fn number(')[0]
    fields = set(re.findall(r'"([a-z_]+)"', config_source))
    cli = (ROOT / 'docs/CLI.md').read_text()
    documented = set(re.findall(r'^\| `([a-z_]+)` \| `--', cli, re.M))
    if fields != documented:
        errors.append('CLI configuration field inventory mismatch')
    for key in fields:
        if f'`--{key.replace("_", "-")}`' not in cli or f'`KIARNI_{key.upper()}`' not in cli:
            errors.append(f'CLI/env spelling missing: {key}')
    import json
    routes = json.loads((ROOT / 'crates/http/tests/fixtures/routes.json').read_text())
    documented_routes = set()
    for methods, path in re.findall(r'^\| ([A-Z, ]+) \| `(/v1/[^`]+)` \|', api, re.M):
        for method in methods.split(', '):
            documented_routes.add((method, path))
    fixture_routes = {(item['method'], item['path'].replace('/orders', '/{collection}').replace('/ord_1', '/{id}')) for item in routes if 'operation' in item}
    if fixture_routes != documented_routes:
        errors.append(f'API route inventory mismatch: missing={fixture_routes-documented_routes}; extra={documented_routes-fixture_routes}')
    if errors:
        raise SystemExit('\n'.join(errors))
    print(f'Documentation passed: {links} local links/anchors, {len(catalog)} errors, {len(fields)} config fields, {len(documented_routes)} method/routes.')

if __name__ == '__main__':
    check()
