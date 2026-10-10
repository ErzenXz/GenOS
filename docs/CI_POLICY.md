# Required verification and exact-head execution

The `main` branch requires the sixty-nine GitHub Actions checks listed in
[`required-ci-checks.json`](../tools/required-ci-checks.json). Each context is bound
to GitHub Actions App ID 15368. Required checks use strict base freshness and apply
to administrators; force pushes and branch deletion are disabled. This policy was
created on October 10, 2026 and strengthened/read back on October 11. No approval-count
requirement was added. Independent review and reference reproduction remain
separate roadmap requirements.

The contract covers the CI, exception-entry and foundation-containment workflows:
host/static/release/documentation/unsafe checks, parser seeds, normal boots,
single-core and boot-memory admission, twelve user/kernel exceptions, and thirty-nine
foundation fixtures, including scoped current-process borrow reentry and a
fresh-volume write failure through the production quarantine and shell, plus a
falsified post-CR3 observation through the production CPU-local gate, and real
keyboard IRQ delivery deferred across a scoped input borrow. Five independent
memory lanes cover pre-initialization access, repeated/wrong-IF initialization,
reentrant access and real keyboard delivery delayed during frame allocation.
Matrix jobs use `fail-fast: false`; a failing CPU fixture must
not hide unrelated results. The weekly extended validation workflow remains
scheduled qualification, rather than a pull-request check or completed soak.
PR workflows explicitly check out the reviewed head; push runs check out their
event commit. The CI evidence manifest records the checked-out HEAD/tree and the
event commit separately. Per-run success/failure manifests are uploaded alongside
serial logs. Earlier PR #9 results used a synthetic merge whose source tree was
verified identical to its head; both identities are recorded in
[the verification ledger](VERIFICATION.md#october-10-cpuscheduler-extraction-and-published-verification).

## Read-only policy audit

```sh
python3 tools/audit_ci_policy.py --head COMMIT_SHA --evidence build/ci-policy-evidence/UNIQUE_RUN
python3 tools/audit_ci_policy.py --head COMMIT_SHA --require-success --evidence build/ci-policy-evidence/UNIQUE_GREEN_RUN
```

Use the full forty-digit reviewed head. The first command verifies actual branch
protection and independently completed exact-head lanes. It reports every failed
lane, but its successful **policy audit does not mean those tests passed**.
`--require-success` additionally rejects any failed or timed-out lane and is the
mode required when evaluating a green candidate. Pending, cancelled, skipped,
neutral, duplicate, stale, missing or wrong-app lanes fail either mode. Failure
cannot be substituted by a successful aggregate job. The GitHub checks themselves
are required directly, so the audit's reporting mode cannot override a failed lane.

The command performs GET requests only and requires the user's existing `gh`
authentication. It retains the contract, raw API responses, SHA-256 hashes and
passed/failed audit manifest in a fresh directory; a rerun cannot overwrite prior
evidence. Network/API failure leaves a failed manifest. To replay a retained read:

```sh
python3 tools/audit_ci_policy.py --head COMMIT_SHA --protection PATH/protection.json --check-runs PATH/check-runs.json --require-success
```

API responses are evidence obtained by the caller, not signed attestations or
independent reviewer approval. Replay verifies that snapshot; a fresh live read
is needed to establish current enforcement. Future workflow/check renames require
updating both the versioned contract and actual branch protection, then retaining
a new readback and exact-head execution audit. Neither side may silently remove
checks to allow a failing candidate to merge.

`python3 -m unittest discover -s tools -p test_audit_ci_policy.py` tests policy
weakening, app binding, head identity, missing/duplicate/skipped execution,
executed failures, success-mode refusal, and retention without overwrites.

The October 10 draft [PR #9](https://github.com/ErzenXz/GenOS/pull/9) publishes the
previously local workflow changes. Its retained strict failures still prevent
integration; publication and enforcement do not advance the release level.
