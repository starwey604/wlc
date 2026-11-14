# SPDX-License-Identifier: Apache-2.0
"""Package, exercise, and promote the exact WLC release artifacts."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib
import zipfile

from release_common import (git_sha, publish, run, verify_checksums, verify_origin,
                            write_checksums, write_json)

ASSETS = {
    'wlc-windows-x86_64': ('x86_64-pc-windows-msvc', 'zip'),
    'wlc-linux-x86_64-musl': ('x86_64-unknown-linux-musl', 'tar.gz'),
    'wlc-linux-aarch64-musl': ('aarch64-unknown-linux-musl', 'tar.gz'),
    'wlc-macos-x86_64': ('x86_64-apple-darwin', 'tar.gz'),
    'wlc-macos-aarch64': ('aarch64-apple-darwin', 'tar.gz'),
}
ROOT = Path(__file__).resolve().parents[2]


def metadata(root, wirelink, run_id):
    version = tomllib.loads((root / 'Cargo.toml').read_text())['package']['version']
    source_sha = git_sha(root)
    pair_path = wirelink / '.github/release-pair.json'
    if pair_path.is_file():
        compiler = json.loads(pair_path.read_text())['wlc']
        assert compiler['source_sha'] == source_sha, 'Wirelink does not pin this compiler source'
        assert compiler['version'] == version and compiler['codegen_contract'] == '0.13'
    return dict(format='wlc-release-v1', version=version, wlc_sha=source_sha,
                wirelink_sha=git_sha(wirelink), codegen_contract='0.13', binding_api=3,
                rust_toolchain='1.96.1', source_run_id=str(run_id))


def pack(args):
    target, extension = ASSETS[args.asset]
    info = {**metadata(ROOT, args.wirelink, args.run_id), 'asset': args.asset, 'target': target}
    args.output.mkdir(parents=True, exist_ok=True)
    stage = args.output / args.asset
    stage.mkdir()
    binary_name = 'wlc.exe' if extension == 'zip' else 'wlc'
    shutil.copy2(args.binary, stage / binary_name)
    (stage / 'README.md').write_text(
        f"# WLC {info['version']}\n\n"
        f"This archive contains the {target} compiler. Rust is not required.\n\n"
        "Verify the downloaded archive against the release's SHA256SUMS before extraction.\n"
        "Put wlc (wlc.exe on Windows) on PATH, or set Wirelink's\n"
        "WIRELINK_WLC_EXECUTABLE to its absolute path. Run `wlc --version` and\n"
        f"`wlc codegen-contract`; expect version {info['version']} and contract 0.13.\n\n"
        "Endpoint profiles use version 2; SDK binding API is 3. wlc-build.json\n"
        "records the exact compiler/core source pair and target.\n\n"
        "Usage: https://github.com/starwey604/wlc/blob/main/README.md\n"
        "Kernel API: https://github.com/starwey604/wirelink/blob/main/docs/kernel-api.md\n",
        encoding='utf-8')
    shutil.copy2(ROOT / 'CHANGELOG.md', stage / 'CHANGELOG.md')
    write_json(stage / 'wlc-build.json', info)
    archive = args.output / f'{args.asset}.{extension}'
    if extension == 'zip':
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as output:
            for path in sorted(stage.iterdir()):
                output.write(path, path.name)
    else:
        with tarfile.open(archive, 'w:gz') as output:
            for path in sorted(stage.iterdir()):
                output.add(path, path.name)
    shutil.rmtree(stage)
    print(archive)


def smoke(args):
    unpacked = args.work / 'unpacked'
    unpacked.mkdir(parents=True)
    if args.archive.suffix == '.zip':
        with zipfile.ZipFile(args.archive) as archive:
            archive.extractall(unpacked)
    else:
        with tarfile.open(args.archive) as archive:
            archive.extractall(unpacked, filter='data')
    info = json.loads((unpacked / 'wlc-build.json').read_text())
    expected = metadata(ROOT, args.wirelink, args.run_id)
    assert all(info[key] == value for key, value in expected.items()), 'archive provenance mismatch'
    binary = (unpacked / ('wlc.exe' if info['asset'].startswith('wlc-windows') else 'wlc')).resolve()
    assert subprocess.check_output([binary, '--version'], text=True).strip() == f'wlc {info["version"]}'
    assert subprocess.check_output([binary, 'codegen-contract'], text=True).strip() == '0.13'
    generated = args.work / 'generated'
    run(binary, 'compile-kernel', ROOT / 'tests/fixtures/release_smoke.wl', '--profile',
        ROOT / 'tests/fixtures/release_smoke.bind.wl', '--out-dir', generated)
    manifest = json.loads((generated / 'release_smoke_kernel_manifest.json').read_text())
    assert manifest['compiler']['codegen_contract'] == dict(major=0, minor=13)
    for suffix in ('.h', '.c', '_values.h', '_kernel.h', '_kernel.c', '_kernel_codecs.h',
                   '_kernel_deployment.json'):
        assert (generated / f'release_smoke{suffix}').is_file(), suffix
    sdk = args.work / 'sdk'
    run(binary, 'sdk', ROOT / 'tests/fixtures/sdk/device.wl', '--profile',
        ROOT / 'tests/fixtures/sdk/device.bind.wl', '--name', 'device', '--package-version',
        info['version'], '--out-dir', sdk)
    sdk_manifest = json.loads((sdk / 'wlc-sdk-info.json').read_text())
    assert sdk_manifest['binding_api'] == 3
    for name in ('python/device_sdk/__init__.py', 'src/endpoint.c'):
        assert (sdk / name).is_file(), name
    # Equality with checked-in SDKs connects archived compilers on all five
    # platforms to the same generated C exercised by the full Wirelink matrix.
    run('python', args.wirelink / 'tests/package/check_generated_sdks.py', '--wlc', binary)
    prefix = (args.work / 'install').resolve()
    core_build = args.work / 'core'
    run('cmake', '-S', args.wirelink, '-B', core_build, '-DCMAKE_BUILD_TYPE=Release',
        '-DBUILD_TESTING=OFF', '-DWIRELINK_BUILD_EXAMPLES=OFF')
    run('cmake', '--build', core_build, '--config', 'Release', '--parallel', '4')
    run('cmake', '--install', core_build, '--config', 'Release', '--prefix', prefix)
    for name, source in (('smoke', ROOT / 'tests/release_smoke'),
                         ('consumer', args.wirelink / 'tests/package/wlc_current')):
        build = args.work / name
        run('cmake', '-S', source, '-B', build, '-DCMAKE_BUILD_TYPE=Release',
            f'-DCMAKE_PREFIX_PATH={prefix}', f'-DWIRELINK_WLC_EXECUTABLE={binary}')
        run('cmake', '--build', build, '--config', 'Release', '--parallel', '4')
        run('ctest', '--test-dir', build, '-C', 'Release', '--output-on-failure')


def collect(args):
    info = metadata(ROOT, args.wirelink, args.run_id)
    args.output.mkdir()
    expected = {f'{name}.{ext}' for name, (_, ext) in ASSETS.items()}
    archives = [p for p in args.artifacts.rglob('*') if p.is_file()]
    assert {p.name for p in archives} == expected and len(archives) == 5, 'expected five platform archives'
    for archive in archives:
        if archive.suffix == '.zip':
            with zipfile.ZipFile(archive) as package:
                embedded = json.loads(package.read('wlc-build.json'))
        else:
            with tarfile.open(archive) as package:
                embedded = json.load(package.extractfile('wlc-build.json'))
        assert all(embedded[key] == value for key, value in info.items()), 'mixed source pairs in archives'
        target, extension = ASSETS[embedded['asset']]
        assert archive.name == f'{embedded["asset"]}.{extension}' and embedded['target'] == target
        shutil.copy2(archive, args.output / archive.name)
    write_json(args.output / 'wlc-release.json', info)
    write_checksums(args.output)


def verify(args):
    verify_checksums(args.output)
    info = json.loads((args.output / 'wlc-release.json').read_text())
    expected = metadata(ROOT, args.wirelink, args.promote_run or args.run_id)
    assert info == expected, 'release source/version/pair differs from the verified bundle'
    expected_names = {f'{name}.{ext}' for name, (_, ext) in ASSETS.items()} | {'wlc-release.json', 'SHA256SUMS'}
    assert {p.name for p in args.output.iterdir()} == expected_names, 'incorrect WLC asset inventory'
    if args.promote_run:
        verify_origin('starwey604/wlc', args.promote_run, info, info['wlc_sha'])
    if args.publish:
        assert args.tag == f'v{info["version"]}', 'Cargo version and release tag differ'
        publish('starwey604/wlc', args.tag, args.output, info['wlc_sha'], promotion=bool(args.promote_run))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('pack', 'smoke', 'collect', 'verify'))
    parser.add_argument('--wirelink', type=Path, required=True)
    parser.add_argument('--run-id', required=True)
    parser.add_argument('--asset', choices=ASSETS)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--work', type=Path)
    parser.add_argument('--artifacts', type=Path)
    parser.add_argument('--output', type=Path, default=Path('dist'))
    parser.add_argument('--promote-run', default='')
    parser.add_argument('--tag', default='')
    parser.add_argument('--publish', action='store_true')
    args = parser.parse_args()
    globals()[args.command](args)


if __name__ == '__main__':
    main()
