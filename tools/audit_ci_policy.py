#!/usr/bin/env python3
"""Audit required-check enforcement and exact-head lane execution without changing GitHub."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def required_checks(contract: dict) -> set[tuple[str, int]]:
    if contract.get('schema_version') != 1:
        raise ValueError('unsupported CI policy contract')
    checks = contract.get('checks')
    if not isinstance(checks, list) or not checks:
        raise ValueError('missing required checks')
    result = set()
    for check in checks:
        name, app = check.get('context'), check.get('app_id')
        if not isinstance(name, str) or not name.strip() or type(app) is not int or app <= 0:
            raise ValueError('invalid check name or unbound GitHub App identity')
        if (name, app) in result or any(existing == name for existing, _ in result):
            raise ValueError('duplicate required check name')
        result.add((name, app))
    return result


def audit_policy(contract: dict, policy: dict) -> None:
    expected = required_checks(contract)
    status = policy.get('required_status_checks') or {}
    if status.get('strict') is not True or policy.get('enforce_admins', {}).get('enabled') is not True:
        raise ValueError('strict base freshness and admin enforcement must be enabled')
    for field in ('allow_force_pushes', 'allow_deletions'):
        if policy.get(field, {}).get('enabled') is not False:
            raise ValueError(f'{field} must be explicitly disabled')
    actual = required_checks({'schema_version': 1, 'checks': status.get('checks')})
    if actual != expected:
        raise ValueError(f'required-check drift: missing={sorted(expected - actual)} extra={sorted(actual - expected)}')


def audit_execution(contract: dict, report: dict, head: str, require_success: bool = False) -> list[str]:
    if re.fullmatch('[0-9a-f]{40}', head) is None:
        raise ValueError('exact forty-digit commit required')
    failed = []
    for name, app in sorted(required_checks(contract)):
        matches = [run for run in report.get('check_runs', [])
                   if run.get('name') == name and run.get('app', {}).get('id') == app]
        if len(matches) != 1:
            raise ValueError(f'missing, duplicate or wrong-app lane: {name}')
        run = matches[0]
        if run.get('head_sha') != head:
            raise ValueError(f'stale or wrong-head lane: {name}')
        if run.get('status') != 'completed' or run.get('conclusion') not in ('success', 'failure', 'timed_out'):
            raise ValueError(f'lane has not completed execution: {name}')
        if run['conclusion'] != 'success':
            failed.append(name)
    if require_success and failed:
        raise ValueError(f'exact-head verification failed: {failed}')
    return failed


def github(path: str) -> dict:
    output = subprocess.run(['gh', 'api', path], capture_output=True, text=True,
                            check=True, timeout=60)
    return json.loads(output.stdout)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--head', required=True)
    parser.add_argument('--require-success', action='store_true',
                        help='also require all lanes to pass; default audits execution and enforcement')
    parser.add_argument('--protection', type=Path, help='replay a retained branch protection response')
    parser.add_argument('--check-runs', type=Path, help='replay a retained exact-head check-runs response')
    parser.add_argument('--evidence', type=Path, help='new directory for retained read-only API evidence')
    options = parser.parse_args()
    if bool(options.protection) != bool(options.check_runs):
        parser.error('--protection and --check-runs must be supplied together')
    if re.fullmatch('[0-9a-f]{40}', options.head) is None:
        parser.error('--head must be an exact forty-digit commit')
    contract = json.loads((Path(__file__).resolve().parent / 'required-ci-checks.json').read_text())
    repo, branch = contract['repository'], contract['branch']
    if re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo) is None or branch != 'main':
        parser.error('invalid repository or branch in policy contract')
    manifest = {'status': 'incomplete', 'head': options.head,
                'require_success': options.require_success, 'verification_passed': False}
    if options.evidence:
        options.evidence.mkdir(parents=True, exist_ok=False)
        (options.evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    try:
        if options.protection:
            policy = json.loads(options.protection.read_text())
            report = json.loads(options.check_runs.read_text())
        else:
            policy = github(f'repos/{repo}/branches/{branch}/protection')
            report = github(f'repos/{repo}/commits/{options.head}/check-runs?per_page=100')
        # Retain responses before checking, including failed/pending lanes.
        # This is API evidence, not a signed attestation or independent review.
        if options.evidence:
            for name, value in [('protection.json', policy), ('check-runs.json', report),
                                ('contract.json', contract)]:
                raw = (json.dumps(value, indent=2) + '\n').encode()
                (options.evidence / name).write_bytes(raw)
                manifest[name + '_sha256'] = hashlib.sha256(raw).hexdigest()
        audit_policy(contract, policy)
        failed = audit_execution(contract, report, options.head)
        manifest.update(failed_lanes=failed, verification_passed=not failed)
        if options.require_success:
            audit_execution(contract, report, options.head, True)
        manifest['status'] = 'passed'
    except Exception as error:
        manifest.update(status='failed', failure=f'{type(error).__name__}: {error}')
        raise
    finally:
        if options.evidence:
            (options.evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'CI_POLICY_OK required={len(required_checks(contract))} executed={len(required_checks(contract))} '
          f'failed={len(failed)} head={options.head}')
    for name in failed:
        print(f'CI_LANE_FAILED {name}')


if __name__ == '__main__':
    main()
