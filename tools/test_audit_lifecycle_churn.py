import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from audit_lifecycle_churn import REQUIRED_FILES, audit
from test_cpu_paging import CAPABILITIES_ID, START_ID, STATUS_ID
from test_cpu_paging_harness import valid_qmp
from test_lifecycle_churn_harness import valid_log
from test_lifecycle_churn import validate_log


def evidence(root):
    for name in REQUIRED_FILES: (root/name).write_text('retained data\n')
    (root/'serial.log').write_text(valid_log(4))
    (root/'qmp.json').write_text(json.dumps(valid_qmp()))
    (root/'qmp-received.jsonl').write_text('\n'.join(json.dumps(m) for m in valid_qmp())+'\n')
    sent = [{'execute': command, 'id': identity} for command, identity in
            [('qmp_capabilities', CAPABILITIES_ID), ('cont', START_ID), ('query-status', STATUS_ID)]]
    (root/'qmp-sent.jsonl').write_text('\n'.join(json.dumps(m) for m in sent)+'\n')
    m = {'status':'passed', 'commit':'a'*40, 'source_clean':True, 'cycles':4,
         'qualifies_10000_cycles':False, 'archive_sha256':'b'*64, 'image_sha256':'c'*64,
         'result':validate_log(valid_log(4),4),
         'fixture_sha256':hashlib.sha256((root/'fixture.patch').read_bytes()).hexdigest(),
         'files_sha256':{name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in REQUIRED_FILES}}
    save(root,m)
    return m


def save(root, m): (root/'manifest.json').write_text(json.dumps(m))


class LifecycleAuditTests(unittest.TestCase):
    def test_short_pass_is_explicitly_not_qualification(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);evidence(root)
            self.assertIs(audit(root,4)['qualifies_10000_cycles'],False)
            with self.assertRaises(ValueError):audit(root)

    def test_failed_dirty_forged_scope_and_wrong_totals_are_rejected(self):
        for key,value in [('status','failed'),('source_clean',False),('cycles',10000),
                          ('qualifies_10000_cycles',True),('commit','a'*39),
                          ('archive_sha256',''),('fixture_sha256','b'*64),('result',{})]:
            with self.subTest(key=key),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);m=evidence(root);m[key]=value;save(root,m)
                with self.assertRaises(ValueError):audit(root,4)

    def test_every_missing_or_corrupted_raw_artifact_is_rejected(self):
        for name in REQUIRED_FILES:
            for mode in ['omit','delete','corrupt']:
                with self.subTest(name=name,mode=mode),tempfile.TemporaryDirectory() as temporary:
                    root=Path(temporary);m=evidence(root)
                    if mode=='omit':del m['files_sha256'][name]
                    elif mode=='delete':(root/name).unlink()
                    else:(root/name).write_text('substituted')
                    save(root,m)
                    with self.assertRaises(ValueError):audit(root,4)

    def test_paths_and_symlink_substitution_are_rejected(self):
        for path in ['../serial.log','/tmp/serial.log','manifest.json']:
            with self.subTest(path=path),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);m=evidence(root);m['files_sha256'][path]='d'*64;save(root,m)
                with self.assertRaises(ValueError):audit(root,4)
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);evidence(root);(root/'serial.log').unlink();(root/'serial.log').symlink_to(root/'build.log')
            with self.assertRaises(ValueError):audit(root,4)

    def test_qmp_liveness_and_wire_correspondence_remain_required(self):
        for mode in ['stopped','wire-drift','wrong-command']:
            with self.subTest(mode=mode),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);m=evidence(root)
                if mode=='stopped':
                    messages=valid_qmp();messages[-1]['return']['running']=False
                    (root/'qmp.json').write_text(json.dumps(messages))
                    (root/'qmp-received.jsonl').write_text('\n'.join(json.dumps(v) for v in messages)+'\n')
                elif mode=='wire-drift':(root/'qmp-received.jsonl').write_text('{}\n')
                else:(root/'qmp-sent.jsonl').write_text('{}\n')
                for name in ['qmp.json','qmp-received.jsonl','qmp-sent.jsonl']:
                    m['files_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
                save(root,m)
                with self.assertRaises(ValueError):audit(root,4)


if __name__=='__main__':unittest.main()
