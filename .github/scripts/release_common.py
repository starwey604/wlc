# SPDX-License-Identifier: Apache-2.0
"""Integrity and immutable promotion shared by the paired release workflows."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile
from urllib.parse import quote


def run(*args, **kwargs):
    return subprocess.run(list(map(str, args)), check=True, **kwargs)


def git_sha(root):
    return subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + '\n', encoding='utf-8')


def write_checksums(directory):
    directory = Path(directory)
    names = sorted(p.name for p in directory.iterdir() if p.name != 'SHA256SUMS')
    (directory / 'SHA256SUMS').write_text(
        ''.join(f'{sha256(directory / name)}  {name}\n' for name in names), encoding='ascii')


def verify_checksums(directory):
    directory = Path(directory)
    expected = {}
    for line in (directory / 'SHA256SUMS').read_text(encoding='ascii').splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  ([A-Za-z0-9_.+-]+)', line)
        if not match or match[2] in expected or match[2] == 'SHA256SUMS':
            raise ValueError('invalid or duplicate checksum entry')
        expected[match[2]] = match[1]
    actual = {p.name for p in directory.iterdir() if p.is_file()} - {'SHA256SUMS'}
    if actual != set(expected):
        raise ValueError('bundle inventory differs from SHA256SUMS')
    for name, digest in expected.items():
        if sha256(directory / name) != digest:
            raise ValueError(f'checksum mismatch: {name}')
    return expected


def api(path, *args, missing=False):
    result = subprocess.run(['gh', 'api', path, *map(str, args)], capture_output=True, text=True)
    if missing and result.returncode and 'HTTP 404' in result.stderr:
        return None
    result.check_returncode()
    return json.loads(result.stdout)


def verify_origin(repository, run_id, metadata, source_sha):
    origin = api(f'repos/{repository}/actions/runs/{int(run_id)}')
    if (origin['conclusion'] != 'success' or origin['head_sha'] != source_sha
            or origin['path'] != '.github/workflows/release.yml'):
        raise ValueError('promotion requires a successful release run of this exact source commit')
    if str(metadata['source_run_id']) != str(run_id):
        raise ValueError('bundle provenance does not name the promoted run')


def ensure_tag(repository, tag, source_sha, create=False):
    if not re.fullmatch(r'v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', tag):
        raise ValueError('invalid release tag')
    ref = api(f'repos/{repository}/git/ref/tags/{quote(tag, safe="")}', missing=True)
    if ref is None:
        if not create:
            raise ValueError('publishing without an existing tag requires a verified promotion run')
        api(f'repos/{repository}/git/refs', '-f', f'ref=refs/tags/{tag}', '-f', f'sha={source_sha}')
        return
    obj = ref['object']
    while obj['type'] == 'tag':
        obj = api(f'repos/{repository}/git/tags/{obj["sha"]}')['object']
    if obj['type'] != 'commit' or obj['sha'] != source_sha:
        raise ValueError('release tag does not point to the verified source commit')


def publish(repository, tag, directory, source_sha, *, promotion=False, notes=None):
    directory = Path(directory)
    expected = verify_checksums(directory)
    ensure_tag(repository, tag, source_sha, create=promotion)
    existing = api(f'repos/{repository}/releases/tags/{quote(tag, safe="")}', missing=True)
    if existing is not None:
        if existing['draft'] or existing['prerelease'] != ('-' in tag):
            raise ValueError('existing release has a different publication state')
        # Repeated promotion is a read-only success only for identical assets.
        if {a['name'] for a in existing['assets']} != set(expected) | {'SHA256SUMS'}:
            raise ValueError('existing release has a different asset inventory; no assets were replaced')
        with tempfile.TemporaryDirectory() as temporary:
            run('gh', 'release', 'download', tag, '--repo', repository, '--dir', temporary)
            if verify_checksums(temporary) != expected:
                raise ValueError('existing release assets differ; no assets were replaced')
        print(f'{repository} {tag}: identical release already published')
        return
    args = ['gh', 'release', 'create', tag, *sorted(directory.iterdir()), '--repo', repository,
            '--verify-tag', '--title', f'{repository.split("/")[1]} {tag}']
    args += ['--prerelease', '--latest=false'] if '-' in tag else ['--latest']
    args += ['--notes-file', str(notes)] if notes else ['--generate-notes']
    run(*args)
