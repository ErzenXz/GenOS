"""Reject misleading, detached or incomplete TCP requirement evidence."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

import check_tcp_matrix as matrix


class TcpMatrixTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.current = matrix.read_matrix()
        cls.listing = matrix.test_listing()

    def check(self, value):
        return matrix.audit(value, listing=self.listing)

    def test_complete_inventory_is_bound_to_compiled_production_tests(self):
        self.assertEqual(self.check(self.current), [])
        self.assertEqual(len(matrix.EXPECTED_IDS), 119)
        self.assertEqual(len(self.current['rows']), 25)
        document = matrix.DOCUMENT.read_text()
        self.assertEqual(document, matrix.render(self.current))

    def test_every_requirement_omission_and_duplicate_fails(self):
        for index, row in enumerate(self.current['rows']):
            for identifier in row['rfc9293']:
                with self.subTest(identifier=identifier):
                    value = copy.deepcopy(self.current)
                    value['rows'][index]['rfc9293'].remove(identifier)
                    self.assertTrue(self.check(value))
                    value['rows'][index]['rfc9293'] += [identifier, identifier]
                    self.assertTrue(self.check(value))

    def test_missing_topic_source_and_classification_cannot_hide_a_gap(self):
        for field in ('input_classes', 'sources', 'production_sha256'):
            for key in self.current[field]:
                value = copy.deepcopy(self.current)
                del value[field][key]
                self.assertTrue(self.check(value))
        for index in range(len(self.current['rows'])):
            value = copy.deepcopy(self.current)
            value['rows'].pop(index)
            self.assertTrue(self.check(value))

    def test_unreviewed_owner_change_and_orphaned_test_fail(self):
        for path in self.current['production_sha256']:
            value = copy.deepcopy(self.current)
            value['production_sha256'][path] = '0' * 64
            self.assertTrue(self.check(value))
        value = copy.deepcopy(self.current)
        value['rows'][0]['implementation'][0]['symbol'] = 'nonexistent_parser'
        self.assertTrue(self.check(value))
        value = copy.deepcopy(self.current)
        value['rows'][0]['tests'][0]['name'] = 'net::tests::does_not_compile'
        self.assertTrue(self.check(value))
        self.assertTrue(matrix.audit(self.current, listing=set()))

    def test_verified_status_cannot_replace_missing_outcomes_or_erase_gap_records(self):
        for index, row in enumerate(self.current['rows']):
            if row['status'] == 'verified':
                value = copy.deepcopy(self.current)
                value['rows'][index]['tests'] = []
                self.assertTrue(self.check(value))
            else:
                for field in ('gaps', 'planned'):
                    value = copy.deepcopy(self.current)
                    value['rows'][index][field] = []
                    self.assertTrue(self.check(value))
                value = copy.deepcopy(self.current)
                value['rows'][index]['status'] = 'verified'
                self.assertTrue(self.check(value))

    def test_missing_guest_check_producer_or_validator_fails(self):
        for field, replacement in [('check', 'Nonexistent CI'),
                                   ('command', 'true'), ('markers', ['FAKE_TCP_READY'])]:
            value = copy.deepcopy(self.current)
            value['system_evidence'][0][field] = replacement
            self.assertTrue(self.check(value))

    def test_duplicate_json_keys_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'bad.json'
            path.write_text('{"schema": 1, "schema": 2}')
            with self.assertRaises(ValueError):
                matrix.read_matrix(path)
            path.write_text(json.dumps(self.current))
            self.assertEqual(matrix.read_matrix(path), self.current)


if __name__ == '__main__':
    unittest.main()
