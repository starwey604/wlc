# SPDX-License-Identifier: Apache-2.0
import json
from pathlib import Path
import tempfile
import shutil
import tarfile
import zipfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import release
import release_common as common


class IntegrityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'asset.zip').write_bytes(b'archive')
        common.write_json(self.root / 'release.json', {'version': '0.13.0'})
        common.write_checksums(self.root)

    def test_release_integrity_covers_every_asset_and_metadata(self):
        self.assertEqual(set(common.verify_checksums(self.root)), {'asset.zip', 'release.json'})
        (self.root / 'release.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            common.verify_checksums(self.root)

    def test_missing_added_and_duplicate_entries_fail(self):
        sums = (self.root / 'SHA256SUMS').read_text()
        (self.root / 'extra.zip').write_bytes(b'extra')
        with self.assertRaisesRegex(ValueError, 'inventory'):
            common.verify_checksums(self.root)
        (self.root / 'extra.zip').unlink()
        (self.root / 'SHA256SUMS').write_text(sums + sums.splitlines()[0] + '\n')
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            common.verify_checksums(self.root)

    def test_origin_must_be_successful_and_match_source_and_workflow(self):
        origin = dict(conclusion='success', head_sha='a' * 40, path='.github/workflows/release.yml')
        with patch.object(common, 'api', return_value=origin):
            common.verify_origin('owner/repo', '123', {'source_run_id': '123'}, 'a' * 40)
            with self.assertRaises(ValueError):
                common.verify_origin('owner/repo', '123', {'source_run_id': '124'}, 'a' * 40)
            for key, value in [('conclusion', 'failure'), ('head_sha', 'b' * 40), ('path', 'ci.yml')]:
                with patch.object(common, 'api', return_value={**origin, key: value}):
                    with self.assertRaises(ValueError):
                        common.verify_origin('owner/repo', '123', {'source_run_id': '123'}, 'a' * 40)

    def test_tag_creation_requires_promotion_and_existing_tag_must_match(self):
        with patch.object(common, 'api', return_value=None) as api:
            with self.assertRaises(ValueError):
                common.ensure_tag('owner/repo', 'v0.13.0', 'a' * 40)
            common.ensure_tag('owner/repo', 'v0.13.0', 'a' * 40, create=True)
            self.assertEqual(api.call_count, 3)
        with patch.object(common, 'api', return_value={'object': {'type': 'commit', 'sha': 'b' * 40}}):
            with self.assertRaises(ValueError):
                common.ensure_tag('owner/repo', 'v0.13.0', 'a' * 40)

    def test_existing_different_release_is_never_overwritten(self):
        with patch.object(common, 'ensure_tag'), patch.object(common, 'api', return_value={'assets': [], 'draft': False, 'prerelease': False}), \
             patch.object(common, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'inventory'):
                common.publish('owner/repo', 'v0.13.0', self.root, 'a' * 40)
            run.assert_not_called()

    def test_identical_existing_release_is_read_only(self):
        assets = [{'name': p.name} for p in self.root.iterdir()]
        def download(*args):
            for source in self.root.iterdir():
                shutil.copy2(source, Path(args[-1]) / source.name)
        with patch.object(common, 'ensure_tag'), patch.object(common, 'api', return_value={
                'assets': assets, 'draft': False, 'prerelease': False}), \
             patch.object(common, 'run', side_effect=download) as run:
            common.publish('owner/repo', 'v0.13.0', self.root, 'a' * 40)
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.args[:3], ('gh', 'release', 'download'))

    def test_pack_and_collect_preserve_five_platforms_and_exact_pair(self):
        source = self.root / 'source'
        source.mkdir()
        (source / 'CHANGELOG.md').write_text('release notes')
        binary = source / 'binary'
        binary.write_bytes(b'compiler')
        info = dict(format='wlc-release-v1', version='0.13.0', wlc_sha='a' * 40,
                    wirelink_sha='b' * 40, codegen_contract='0.13', binding_api=3,
                    rust_toolchain='1.96.1', source_run_id='123')
        artifacts = self.root / 'packages'
        bundle = self.root / 'bundle'
        with patch.object(release, 'ROOT', source), patch.object(release, 'metadata', return_value=info):
            for asset, (target, extension) in release.ASSETS.items():
                release.pack(SimpleNamespace(asset=asset, wirelink=source, run_id='123',
                                             output=artifacts, binary=binary))
                archive = artifacts / f'{asset}.{extension}'
                if extension == 'zip':
                    with zipfile.ZipFile(archive) as package:
                        embedded = json.loads(package.read('wlc-build.json'))
                        self.assertEqual(package.read('wlc.exe'), b'compiler')
                else:
                    with tarfile.open(archive) as package:
                        embedded = json.load(package.extractfile('wlc-build.json'))
                        self.assertEqual(package.extractfile('wlc').read(), b'compiler')
                self.assertEqual(embedded, {**info, 'asset': asset, 'target': target})
            release.collect(SimpleNamespace(wirelink=source, run_id='123', artifacts=artifacts, output=bundle))
            self.assertEqual(len(common.verify_checksums(bundle)), 6)
            self.assertEqual(json.loads((bundle / 'wlc-release.json').read_text()), info)
            # Same filenames from a different core pairing must not be mixed.
            with patch.object(release, 'metadata', return_value={**info, 'wirelink_sha': 'c' * 40}):
                with self.assertRaises(AssertionError):
                    release.collect(SimpleNamespace(wirelink=source, run_id='123', artifacts=artifacts,
                                                    output=self.root / 'bad-bundle'))


if __name__ == '__main__':
    unittest.main()
