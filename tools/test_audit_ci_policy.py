"""Required CI policy and exact-head execution must fail closed on evidence drift."""
import copy
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

from audit_ci_policy import audit_execution, audit_policy, main, required_checks

HEAD = 'a' * 40


def contract():
    return {'schema_version': 1, 'checks': [{'context': 'host', 'app_id': 15368},
                                           {'context': 'fault', 'app_id': 15368}]}


def policy():
    return {'required_status_checks': {'strict': True, 'checks': contract()['checks']},
            'enforce_admins': {'enabled': True},
            'allow_force_pushes': {'enabled': False}, 'allow_deletions': {'enabled': False}}


def report():
    return {'check_runs': [{'name': check['context'], 'app': {'id': check['app_id']},
                            'head_sha': HEAD, 'status': 'completed', 'conclusion': 'success'}
                           for check in contract()['checks']]}


class CiPolicyTests(unittest.TestCase):
    def test_every_pr_checkout_selects_reviewed_head_and_manifest_records_actual_source(self):
        root = Path(__file__).resolve().parents[1]
        for name in ['ci.yml', 'exception-entry.yml', 'foundation-hardening.yml']:
            source = (root / '.github/workflows' / name).read_text()
            blocks = source.split('uses: actions/checkout@')[1:]
            self.assertTrue(blocks)
            for block in blocks:
                checkout = block.split('      - ', 1)[0]
                self.assertIn('ref: ${{ github.event.pull_request.head.sha || github.sha }}', checkout)
                self.assertIn('persist-credentials: false', checkout)
        source = (root / '.github/workflows/ci.yml').read_text()
        self.assertIn("git log -1 --format='commit=%H%ntree=%T'", source)
        for path in ['build/validation-evidence/', 'build/normal-evidence/', 'build/exception-evidence/']:
            self.assertIn(path, source)

    def test_enforced_policy_and_all_exact_head_successes_pass(self):
        audit_policy(contract(), policy())
        self.assertEqual(audit_execution(contract(), report(), HEAD, True), [])

    def test_unprotected_relaxed_or_administrator_bypass_policy_fails(self):
        variants = [{}, policy(), policy(), policy(), policy()]
        variants[1]['required_status_checks']['strict'] = False
        variants[2]['enforce_admins']['enabled'] = False
        variants[3]['allow_force_pushes']['enabled'] = True
        variants[4]['allow_deletions']['enabled'] = True
        for value in variants:
            with self.subTest(policy=value), self.assertRaises(ValueError):
                audit_policy(contract(), value)

    def test_missing_duplicate_unbound_or_replaced_required_checks_fail(self):
        variants = [policy() for _ in range(6)]
        variants[0]['required_status_checks']['checks'].pop()
        variants[1]['required_status_checks']['checks'] *= 2
        variants[2]['required_status_checks']['checks'][0]['app_id'] = -1
        variants[3]['required_status_checks']['checks'][0]['app_id'] = 1
        variants[4]['required_status_checks']['checks'][0]['context'] = 'forged'
        variants[5]['required_status_checks']['checks'][0]['app_id'] = True
        for value in variants:
            with self.subTest(policy=value), self.assertRaises(ValueError):
                audit_policy(contract(), value)

    def test_missing_duplicate_wrong_app_and_wrong_head_execution_fail(self):
        variants = [report() for _ in range(5)]
        variants[0]['check_runs'].pop()
        variants[1]['check_runs'].append(copy.deepcopy(variants[1]['check_runs'][0]))
        variants[2]['check_runs'][0]['app']['id'] = 1
        variants[3]['check_runs'][0]['head_sha'] = 'b' * 40
        variants[4]['check_runs'][0]['name'] = 'forged host'
        for value in variants:
            with self.subTest(report=value), self.assertRaises(ValueError):
                audit_execution(contract(), value, HEAD)

    def test_pending_skipped_cancelled_and_neutral_lanes_do_not_prove_execution(self):
        for status, conclusion in [('queued', None), ('in_progress', None),
                                    ('completed', 'skipped'), ('completed', 'cancelled'),
                                    ('completed', 'neutral'), ('completed', 'action_required')]:
            value = report()
            value['check_runs'][0].update(status=status, conclusion=conclusion)
            with self.subTest(status=status, conclusion=conclusion), self.assertRaises(ValueError):
                audit_execution(contract(), value, HEAD)

    def test_executed_failure_is_visible_and_always_blocks_success_mode(self):
        for conclusion in ['failure', 'timed_out']:
            value = report()
            value['check_runs'][0]['conclusion'] = conclusion
            self.assertEqual(audit_execution(contract(), value, HEAD), ['host'])
            with self.assertRaises(ValueError):
                audit_execution(contract(), value, HEAD, True)

    def test_contract_and_head_identity_cannot_be_omitted_or_ambiguous(self):
        for value in [{}, {'schema_version': 2, 'checks': contract()['checks']},
                      {'schema_version': 1, 'checks': []}]:
            with self.assertRaises(ValueError):
                required_checks(value)
        for head in ['', 'a' * 7, HEAD.upper(), 'main', HEAD + '?other']:
            with self.subTest(head=head), self.assertRaises(ValueError):
                audit_execution(contract(), report(), head)

    def test_cli_retains_success_failure_and_executed_failed_lanes_without_overwrite(self):
        tool = Path(__file__).with_name('audit_ci_policy.py')
        expected = json.loads(tool.with_name('required-ci-checks.json').read_text())
        protected = policy()
        protected['required_status_checks']['checks'] = expected['checks']
        runs = {'check_runs': [{'name': check['context'], 'app': {'id': check['app_id']},
                               'head_sha': HEAD, 'status': 'completed', 'conclusion': 'success'}
                              for check in expected['checks']]}
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'policy.json').write_text(json.dumps(protected))
            for label, conclusion, strict, code, status, verified in [
                ('green', 'success', True, 0, 'passed', True),
                ('failed-execution', 'failure', False, 0, 'passed', False),
                ('failed-release', 'failure', True, 1, 'failed', False),
                ('pending', None, False, 1, 'failed', False),
            ]:
                runs['check_runs'][0].update(conclusion=conclusion,
                                            status='in_progress' if conclusion is None else 'completed')
                (root / 'runs.json').write_text(json.dumps(runs))
                evidence = root / label
                command = [sys.executable, str(tool), '--head', HEAD,
                           '--protection', str(root / 'policy.json'),
                           '--check-runs', str(root / 'runs.json'), '--evidence', str(evidence)]
                if strict:
                    command.append('--require-success')
                result = subprocess.run(command, capture_output=True, text=True, timeout=10)
                with self.subTest(label=label):
                    self.assertEqual(result.returncode, code, result.stderr)
                    saved = (evidence / 'manifest.json').read_bytes()
                    manifest = json.loads(saved)
                    self.assertEqual(manifest['status'], status)
                    self.assertEqual(manifest['verification_passed'], verified)
                    self.assertIn('check-runs.json_sha256', manifest)
                    self.assertIn('protection.json_sha256', manifest)
                    self.assertIn('contract.json_sha256', manifest)
                    rerun = subprocess.run(command, capture_output=True, text=True, timeout=10)
                    self.assertNotEqual(rerun.returncode, 0)
                    self.assertEqual((evidence / 'manifest.json').read_bytes(), saved)

    def test_api_failure_retains_a_failed_manifest(self):
        with TemporaryDirectory() as temporary:
            evidence = Path(temporary) / 'unavailable'
            command = ['audit_ci_policy.py', '--head', HEAD, '--evidence', str(evidence)]
            with patch('sys.argv', command), patch('audit_ci_policy.github', side_effect=RuntimeError('API unavailable')):
                with self.assertRaisesRegex(RuntimeError, 'API unavailable'):
                    main()
            manifest = json.loads((evidence / 'manifest.json').read_text())
            self.assertEqual(manifest['status'], 'failed')
            self.assertFalse(manifest['verification_passed'])
            self.assertIn('API unavailable', manifest['failure'])

    def test_historical_contract_replay_cannot_override_a_live_policy_read(self):
        tool = Path(__file__).with_name('audit_ci_policy.py')
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            historical = dict(contract(), repository='ErzenXz/GenOS', branch='main')
            for name, value in [('contract.json', historical), ('protection.json', policy()), ('runs.json', report())]:
                (root/name).write_text(json.dumps(value))
            base = [sys.executable, str(tool), '--head', HEAD, '--contract', str(root/'contract.json')]
            refused = subprocess.run(base, capture_output=True, text=True, timeout=10)
            self.assertEqual(refused.returncode, 2)
            self.assertIn('only allowed with retained', refused.stderr)
            command = base + ['--protection', str(root/'protection.json'), '--check-runs', str(root/'runs.json'),
                              '--require-success', '--evidence', str(root/'audit')]
            passed = subprocess.run(command, capture_output=True, text=True, timeout=10)
            self.assertEqual(passed.returncode, 0, passed.stderr)
            manifest = json.loads((root/'audit/manifest.json').read_text())
            self.assertEqual(manifest['read_mode'], 'retained-replay')
            self.assertEqual(manifest['contract_source'], str(root/'contract.json'))
            self.assertTrue(manifest['verification_passed'])
            failed = report(); failed['check_runs'][0]['conclusion'] = 'failure'
            (root/'runs.json').write_text(json.dumps(failed))
            command[-1] = str(root/'failed-audit')
            rejected = subprocess.run(command, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse(json.loads((root/'failed-audit/manifest.json').read_text())['verification_passed'])


if __name__ == '__main__':
    unittest.main()
