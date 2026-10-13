#!/usr/bin/env python3
"""Check the complete TCP requirement inventory and its live test references.

This checks traceability, not RFC conformance. Gap/unsupported rows are required
to retain explicit missing behavior and planned tests. A source hash change needs
a matrix review; --write-doc only renders already reviewed data, never fixes it.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / 'tools/tcp-requirements.json'
DOCUMENT = ROOT / 'docs/TCP_REQUIREMENTS.md'
EXPECTED_IDS = {f'{kind}-{number}' for kind, count in
                [('MUST', 69), ('SHLD', 31), ('MAY', 18), ('REC', 1)]
                for number in range(1, count + 1)}
TOPICS = {'header', 'checksum', 'options', 'mss', 'isn', 'open', 'address', 'close',
          'rto', 'failure', 'keepalive', 'urgent', 'window', 'persist', 'sws', 'ack',
          'push', 'api', 'ttl', 'ip-options', 'icmp', 'ecn', 'reset', 'congestion', 'capacity'}
PRODUCTION = {'kernel/src/net.rs', 'kernel/src/network.rs', 'kernel/src/socket.rs'}
CLASSES = {'malformed', 'state-inapplicable', 'valid-unsupported', 'profile-limit'}
SOURCES = {'9293': '2022-08', '6298': '2011-06', '5681': '2009-09', '5961': '2010-08'}
ROLES = {'positive', 'negative', 'cleanup', 'exhaustion', 'cancellation'}


def read_matrix(path: Path = MATRIX) -> dict:
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f'duplicate JSON key: {key}')
            result[key] = value
        return result
    return json.loads(path.read_text(), object_pairs_hook=unique)


def audit(matrix: dict, root: Path = ROOT, listing: set[tuple[str, str]] | None = None) -> list[str]:
    errors = []
    def require(condition, message):
        if not condition:
            errors.append(message)
    require(matrix.get('schema') == 1 and matrix.get('profile') == 'genos-tcp-ipv4-bounded-v1'
            and matrix.get('roadmap_item') == 'N1.1', 'wrong matrix schema/profile/item')
    require(bool(re.fullmatch(r'\d{4}-\d{2}-\d{2}', matrix.get('reviewed_date', ''))), 'missing review date')
    require(set(matrix.get('input_classes', {})) == CLASSES, 'missing input classification')
    require(all(matrix.get('input_classes', {}).values()), 'empty input classification')
    sources = matrix.get('sources', {})
    require(set(sources) == set(SOURCES), 'missing normative source')
    for number, published in SOURCES.items():
        source = sources.get(number, {})
        require(source.get('url') == f'https://www.rfc-editor.org/rfc/rfc{number}.html'
                and source.get('published') == published, f'wrong pinned RFC {number}')
    hashes = matrix.get('production_sha256', {})
    require(set(hashes) == PRODUCTION, 'missing production review receipt')
    for path in PRODUCTION:
        actual = hashlib.sha256((root / path).read_bytes()).hexdigest()
        require(hashes.get(path) == actual, f'unreviewed production change: {path}')
    rows = matrix.get('rows', [])
    names = [row.get('id') for row in rows]
    require(len(names) == len(set(names)) and set(names) == TOPICS, 'missing/duplicate TCP topic')
    identifiers = [identifier for row in rows for identifier in row.get('rfc9293', [])]
    require(len(identifiers) == len(set(identifiers)), 'duplicate RFC 9293 identifier')
    require(set(identifiers) == EXPECTED_IDS, 'RFC 9293 inventory omitted or invented an identifier')
    for row in rows:
        label = row.get('id', '?')
        require(all(row.get(field) for field in ('title', 'behavior', 'implementation', 'references')),
                f'{label}: missing behavior/owner/source')
        status = row.get('status')
        require(status in {'verified', 'partial', 'gap', 'unsupported'}, f'{label}: invalid status')
        if status != 'verified':
            require(bool(row.get('gaps')) and bool(row.get('planned')), f'{label}: hidden gap or missing planned test')
        else:
            require(not row.get('gaps'), f'{label}: verified row contains a gap')
        for reference in row.get('references', []):
            number, separator, section = reference.partition(':')
            require(number in SOURCES and separator and
                    bool(re.fullmatch(r'(?:\d+(?:\.\d+)*|appendix-B)', section)), f'{label}: invalid RFC locator')
        for owner in row.get('implementation', []):
            path, symbol = owner.get('path'), owner.get('symbol')
            require(path in PRODUCTION and bool(symbol), f'{label}: unknown owner')
            if path in PRODUCTION and symbol:
                require(bool(re.search(r'\bfn\s+' + re.escape(symbol) + r'\s*(?:<|\()',
                                       (root / path).read_text())), f'{label}: detached implementation {symbol}')
        observed_roles = set()
        tests = row.get('tests', [])
        require(len(tests) == len({(test.get('package'), test.get('name')) for test in tests}),
                f'{label}: duplicate test reference')
        for test in tests:
            key = (test.get('package'), test.get('name'))
            roles = test.get('roles', [])
            require(key[0] == 'kernel' and bool(key[1]) and bool(roles)
                    and set(roles) <= ROLES and len(roles) == len(set(roles)), f'{label}: invalid test evidence')
            if listing is not None:
                require(key in listing, f'{label}: test does not compile: {key}')
            observed_roles.update(roles)
        if status == 'verified':
            require({'positive', 'negative'} <= observed_roles, f'{label}: verified without both outcomes')
    policy = json.loads((root / 'tools/required-ci-checks.json').read_text())
    check_names = {check['context'] for check in policy['checks']}
    system = matrix.get('system_evidence', [])
    require(bool(system), 'missing guest evidence layer')
    producers = '\n'.join((root / path).read_text() for path in
                          ['kernel/src/network.rs', 'kernel/src/userspace.rs', 'kernel/src/runtime.rs'])
    consumers = '\n'.join((root / path).read_text() for path in
                          ['tools/xtask/src/main.rs', 'tools/xtask/src/evidence_contract.rs'])
    for proof in system:
        require(proof.get('check') in check_names and proof.get('command') == 'cargo xtask test-network'
                and bool(proof.get('scope')) and bool(proof.get('markers')), 'invalid guest evidence binding')
        for marker in proof.get('markers', []):
            require(bool(marker) and marker in producers and marker in consumers,
                    f'detached guest producer/validator marker: {marker}')
    require(bool(matrix.get('followup')), 'missing remaining roadmap obligations')
    return errors


def test_listing(root: Path = ROOT) -> set[tuple[str, str]]:
    result = subprocess.run(['cargo', 'test', '-p', 'kernel', '--lib', '--', '--list'],
                            cwd=root, text=True, capture_output=True, check=True, timeout=120)
    return {('kernel', line.removesuffix(': test')) for line in result.stdout.splitlines()
            if line.endswith(': test')}


def render(matrix: dict) -> str:
    lines = [
        '# TCP requirement-to-test matrix', '',
        'N1.1 records the current experimental IPv4 TCP profile and its gaps. It does',
        'not promote GenOS or close the TCP implementation/qualification items N1.2–N1.5.', '',
        f"Profile: `{matrix['profile']}`. Source review: {matrix['reviewed_date']}.", '',
        'This document is generated from [the reviewed matrix](../tools/tcp-requirements.json).',
        '`python3 tools/check_tcp_matrix.py` checks all 119 RFC 9293 Appendix B identifiers,',
        'all 25 topics, production source hashes, compiled host test names, guest',
        'marker producers/consumers and the required CI context. It also rejects a',
        'stale generated document. Source/test changes require review and receipt updates;',
        '`--write-doc` renders the reviewed data and does not refresh hashes or statuses.', '',
        '## Scope and interpretation', '',
        'The wire path is Ethernet/IPv4 TCP. Active transactions have one 128-byte',
        'request/response; passive streams have four global handshake/stream slots,',
        'two per process incarnation, 128-byte application buffers, one outstanding',
        'send flight and one deferred receive segment. Repeated passive operations',
        'can transfer more bytes than a buffer. Three retry attempts and 200 logical',
        'ticks of passive idle lifetime are resource policies. Logical ticks are not',
        'a qualified wall-clock RFC timer. IPv6 application TCP remains N2.1.', '',
        '`verified` means only the row\'s stated behavior has positive/negative tests.',
        '`partial` and `gap` retain missing mandatory behavior; `unsupported` identifies',
        'unadvertised features and their conditional obligations. No status asserts',
        'whole-RFC conformance. Empty test cells are explicit gaps, never passing tests.', '',
        'Pinned sources:', '',
    ]
    for number, source in matrix['sources'].items():
        lines.append(f"- [RFC {number}]({source['url']}), published {source['published']}.")
    lines += ['', 'RFC 9293 Appendix B supplies the identifier inventory; RFC 6298 and',
              'RFC 5681 supply the companion timer/congestion review, and RFC 5961',
              'supplies additional reset/injection review. The table below describes',
              'GenOS behavior from its code/tests, not copied normative prose.', '',
              '## Input classification', '', '| Class | Current meaning |', '| --- | --- |']
    for kind, meaning in matrix['input_classes'].items():
        lines.append(f'| `{kind}` | {meaning} |')
    lines += ['', 'A legal larger segment, zero window, urgent indication or optional offer',
              'is not malformed solely because this bounded implementation lacks its',
              'complete semantics. Mandatory missing behavior stays a conformance gap.', '',
              '## Requirement and implementation inventory', '',
              '| Topic / status | RFC identifiers and sources | Current behavior / owner |',
              '| --- | --- | --- |']
    for row in matrix['rows']:
        sources = []
        for reference in row['references']:
            number, section = reference.split(':')
            anchor = section if section.startswith('appendix-') else 'section-' + section
            sources.append(f"[RFC {number} §{section}]({matrix['sources'][number]['url']}#{anchor})")
        references = ', '.join(f'`{identifier}`' for identifier in row['rfc9293'])
        owners = '; '.join(f"[{owner['symbol']}](../{owner['path']})" for owner in row['implementation'])
        lines.append(f"| {row['id']}: {row['title']} **{row['status']}** | {references}<br>{'; '.join(sources)} | {row['behavior']}<br>{owners} |")
    lines += ['', '## Executable evidence and remaining scenarios', '',
              'Host evidence executes the production parser/state machine with a memory',
              'frame device or the production socket authority model. It does not replace',
              'privilege, DMA or real device tests. Names below must appear in the compiled',
              '`cargo test -p kernel --lib -- --list` output.', '',
              '| Topic | Host tests and outcomes | Missing behavior / next scenarios |',
              '| --- | --- | --- |']
    for row in matrix['rows']:
        tests = '<br>'.join(f"`{test['name']}` ({', '.join(test['roles'])})" for test in row['tests']) or 'No executable claim'
        gaps = ' '.join(row['gaps'] + row['planned']) or 'The stated bounded behavior is covered; no broader inference.'
        lines.append(f"| {row['id']} | {tests} | {gaps} |")
    lines += ['', '## Guest layer and execution', '']
    for proof in matrix['system_evidence']:
        lines += [f"Required CI context: **{proof['check']}**; command `{proof['command']}`.", '',
                  proof['scope'], '', 'Required producer/validator marker bindings:', '']
        lines += [f'- `{marker}`' for marker in proof['markers']]
    lines += ['', 'CI runs the compiled host suite and this inventory guard, then the existing',
              'Build and boot suite executes both the fault-injected network phase and',
              'the production phase without host services. Run identity, source/image',
              'hashes, serial and environment metadata stay in the existing evidence',
              'artifacts. Configured wiring is not execution evidence; see the dated',
              '[verification record](VERIFICATION.md).', '', matrix['followup'], '',
              '## Maintenance', '',
              'For a TCP source change, review the affected rows and test outcomes, update',
              '`production_sha256` from the final source, record missing/new scenarios,',
              'then render with `--write-doc` and run the guard. Keep the full identifier',
              'inventory even for unsupported or untested behavior. Closing N1.1 means',
              'this complete inventory is maintained; it does not mean all rows conform.', '']
    return '\n'.join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write-doc', action='store_true')
    options = parser.parse_args()
    matrix = read_matrix()
    errors = audit(matrix, listing=test_listing())
    if errors:
        raise SystemExit('\n'.join(errors))
    expected = render(matrix)
    if options.write_doc:
        DOCUMENT.write_text(expected)
    elif not DOCUMENT.exists() or DOCUMENT.read_text() != expected:
        raise SystemExit('stale TCP matrix document; review the data and run --write-doc')
    print('TCP_MATRIX_OK item=N1.1 topics=25 rfc9293_identifiers=119 compiled_tests=bound gaps=explicit')


if __name__ == '__main__':
    main()
