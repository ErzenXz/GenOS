//! Bounded command sequences against the production process authority registry.
//!
//! Each 20-byte record contains action (byte 0 modulo 4: register, allows,
//! unregister, clear), owner (byte 1 modulo 2), kind (byte 2 modulo 7: File,
//! EndpointReceive, EndpointSend, Console, Lifecycle, Process, Socket), one
//! reserved byte, a little-endian u64 handle (4..12), and little-endian u64
//! rights (12..20). Clear ignores the handle/kind/rights fields. At most 64
//! complete records execute; partial trailing records are ignored. Numeric
//! handles have no generation semantics and may exist in both owner tables.

use kernel::capability::{HandleKind, HandleTable};
use std::collections::{BTreeMap, BTreeSet};

const KINDS: [HandleKind; 7] = [
    HandleKind::File,
    HandleKind::EndpointReceive,
    HandleKind::EndpointSend,
    HandleKind::Console,
    HandleKind::Lifecycle,
    HandleKind::Process,
    HandleKind::Socket,
];
type Model = BTreeMap<u64, (HandleKind, u64)>;

/// Exercise actual authority operations, checking an independent owner model.
/// This target always returns true: rejected commands are expected outcomes.
pub fn exercise(bytes: &[u8]) -> bool {
    let mut tables = [HandleTable::<4>::new(), HandleTable::<4>::new()];
    let mut models = [Model::new(), Model::new()];
    let mut observed = BTreeSet::from([0, u64::MAX]);
    for raw in bytes.chunks_exact(20).take(64) {
        let owner = usize::from(raw[1] & 1);
        let kind = KINDS[usize::from(raw[2]) % KINDS.len()];
        let handle = u64::from_le_bytes(raw[4..12].try_into().unwrap());
        let rights = u64::from_le_bytes(raw[12..20].try_into().unwrap());
        observed.insert(handle);
        let table = &mut tables[owner];
        let model = &mut models[owner];
        match raw[0] % 4 {
            0 => {
                let expected = handle != 0 && !model.contains_key(&handle) && model.len() < 4;
                assert_eq!(table.register(handle, kind, rights), expected);
                if expected {
                    model.insert(handle, (kind, rights));
                }
            }
            1 => {
                let expected = model.get(&handle).is_some_and(|&(stored_kind, stored)| {
                    stored_kind == kind && stored & rights == rights
                });
                assert_eq!(table.allows(handle, kind, rights), expected);
            }
            2 => {
                let expected = model
                    .get(&handle)
                    .is_some_and(|&(stored_kind, _)| stored_kind == kind);
                assert_eq!(table.unregister(handle, kind), expected);
                if expected {
                    model.remove(&handle);
                }
            }
            _ => {
                table.clear();
                model.clear();
            }
        }
        // Rejected operations leave the model unchanged. Comparing both owners
        // after every operation detects unauthorized writes and cross-owner
        // changes, including when both independently use the same numeric key.
        for index in 0..2 {
            assert_matches(&tables[index], &models[index], &observed);
        }
    }
    for table in &mut tables {
        table.clear();
        assert_matches(table, &Model::new(), &observed);
    }
    true
}

fn assert_matches(table: &HandleTable<4>, model: &Model, observed: &BTreeSet<u64>) {
    assert_eq!(table.len(), model.len());
    assert_eq!(table.is_empty(), model.is_empty());
    for &handle in observed {
        for kind in KINDS {
            let expected = model
                .get(&handle)
                .is_some_and(|&(stored_kind, _)| stored_kind == kind);
            assert_eq!(table.allows(handle, kind, 0), expected);
        }
    }
    for (&handle, &(kind, rights)) in model {
        assert!(table.allows(handle, kind, rights));
        assert_eq!(table.allows(handle, kind, u64::MAX), rights == u64::MAX);
        // Each authority bit is independently observable, including bit 63.
        for bit in 0..64 {
            let required = 1u64 << bit;
            assert_eq!(table.allows(handle, kind, required), rights & required != 0);
        }
    }
}
