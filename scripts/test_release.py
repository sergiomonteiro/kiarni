"""Packaging boundaries and hostile-archive regression tests."""
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import package_release as package
import release_smoke as smoke

class ReleaseTests(unittest.TestCase):
    def test_product_graph_excludes_dev_but_keeps_build_dependencies(self):
        metadata = {'packages': [{'id': n, 'name': n, 'version': '1'} for n in ['kiarni','runtime','build','dev','transitive']],
            'resolve': {'nodes': [
                {'id':'kiarni','deps':[{'pkg':n,'dep_kinds':[{'kind':k}]} for n,k in [('runtime',None),('build','build'),('dev','dev')]]},
                {'id':'runtime','deps':[{'pkg':'transitive','dep_kinds':[{'kind':None}]}]},
                *[{'id':n,'deps':[]} for n in ['build','dev','transitive']]]}}
        self.assertEqual({p['name'] for p in package.production_packages(metadata)}, {'kiarni','runtime','build','transitive'})

    def test_missing_or_unreviewed_license_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            item = {'name':'library','version':'1','manifest_path':str(root/'Cargo.toml'), 'source':'registry', 'license':'MIT'}
            with self.assertRaises(ValueError):
                package.notices([item],root)
            (root/'LICENSE').write_text('full license text')
            package.notices([item],root)
            self.assertIn('full license text',(root/'THIRD_PARTY_NOTICES.txt').read_text())
            item['license']='unreviewed-license'
            with self.assertRaises(ValueError):
                package.notices([item],root)

    def fixture(self, root):
        source = root/'native'
        source.mkdir()
        for name in ['kiarni','LICENSE','THIRD_PARTY_NOTICES.txt','DEPENDENCIES.json','BUILD_INFO.json','INSTALL.md','toolchain/COPYRIGHT-library.html','toolchain/licenses/MIT.txt']:
            path = source/name
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_text('fixture')
        package.checksums(source)
        archive = root/'native.tar.gz'
        package.archive(source,archive)
        self.outer_checksum(archive)
        return source, archive

    def outer_checksum(self, archive):
        archive.with_name(archive.name+'.sha256').write_text(f'{package.sha256(archive)}  {archive.name}\n')

    def test_deterministic_archive_modes_and_full_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            source, archive=self.fixture(root)
            second=root/'second.tar.gz'
            (source/'kiarni').chmod(0o600)
            package.archive(source,second)
            self.assertEqual(archive.read_bytes(),second.read_bytes())
            extracted=root/'extracted'
            extracted.mkdir()
            result=smoke.verify_extract(archive,extracted)
            self.assertEqual((result/'kiarni').stat().st_mode & 0o777,0o755)
            self.assertEqual((result/'LICENSE').stat().st_mode & 0o777,0o644)

    def test_outer_and_inner_tampering_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            source, archive=self.fixture(root)
            archive.write_bytes(archive.read_bytes()+b'tampered')
            with self.assertRaises(ValueError):
                smoke.verify_extract(archive,root/'outer')
            archive.unlink()
            (source/'kiarni').write_text('changed after manifest')
            package.archive(source,archive)
            self.outer_checksum(archive)
            with self.assertRaises(ValueError):
                smoke.verify_extract(archive,root/'inner')

    def test_traversal_links_duplicates_and_size_bombs_rejected(self):
        for kind in ['traversal','symlink','hardlink','duplicate','size']:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary)
                archive=root/'native.tar.gz'
                with tarfile.open(archive,'w:gz') as tar:
                    info=tarfile.TarInfo('native/../outside' if kind=='traversal' else 'native/kiarni')
                    if kind in ['symlink','hardlink']:
                        info.type=tarfile.SYMTYPE if kind=='symlink' else tarfile.LNKTYPE
                        info.linkname='/outside'
                    if kind=='size':
                        info.size=smoke.MAX_BYTES+1
                        # A header alone is enough: the size must be rejected before reading data.
                        tar.fileobj.write(info.tobuf())
                    else:
                        tar.addfile(info)
                        if kind=='duplicate':
                            tar.addfile(info)
                self.outer_checksum(archive)
                with self.assertRaises(ValueError):
                    smoke.verify_extract(archive,root/'extracted')
                self.assertFalse((root/'outside').exists())

if __name__=='__main__':
    unittest.main()
