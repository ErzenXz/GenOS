//! Exhaustive bounded replay of the production Ledger + FrameAllocator.
//! No state merging: every word over the declared action alphabet up to depth 6
//! is executed against fresh production state and an independent small oracle.
use genos_abi::{MemoryRegion, MemoryRegionKind};
use kernel::{
    frame_grant::{Grant, Kind, Ledger, Owner},
    physmem::{FrameAllocator, PAGE_SIZE},
};
use std::{fs, path::Path, time::Instant};

const ACTIONS: [&str; 12] = [
    "allocate-a",
    "allocate-b",
    "allocation-fails",
    "bind-first-va",
    "bind-foreign-owner",
    "bind-second-va",
    "retire-first-va",
    "retire-foreign-owner",
    "retire-second-va",
    "release-latest",
    "release-oldest",
    "preparation-fails",
];
const MAX_DEPTH: usize = 6;
#[derive(Clone, Copy)]
struct Saved {
    grant: Grant,
    owner: usize,
}
#[derive(Clone, Copy)]
struct Cell {
    saved: Saved,
    mapped: u64,
}

fn replay(trace: &[usize], bypass_pin: bool) -> Result<(), &'static str> {
    let mut bitmap = FrameAllocator::<1>::new();
    if !bitmap.add_region(MemoryRegion {
        start: PAGE_SIZE,
        size: PAGE_SIZE * 2,
        kind: MemoryRegionKind::Usable,
    }) {
        return Err("initial bitmap region rejected");
    }
    let mut ledger = Ledger::<2>::new();
    let owners = [
        ledger.new_owner().ok_or("owner a missing")?,
        ledger.new_owner().ok_or("owner b missing")?,
    ];
    if owners[0] == owners[1] || owners.contains(&Owner::KERNEL) {
        return Err("owner identity reused");
    }
    let mut cells: [Option<Cell>; 2] = [None, None];
    let mut history: [Option<Saved>; MAX_DEPTH] = [None; MAX_DEPTH];
    let mut history_len = 0usize;
    for action in trace {
        if *action <= 2 {
            let owner = usize::from(*action == 1);
            let fail = *action == 2;
            let mut called = false;
            let granted = ledger.allocate(owners[owner], Kind::User, || {
                called = true;
                if fail {
                    None
                } else {
                    bitmap.alloc_frame()
                }
            });
            let capacity = cells.iter().any(Option::is_none);
            if called != capacity || granted.is_some() != (capacity && !fail) {
                return Err("allocation admission mismatch");
            }
            if let Some(grant) = granted {
                let address = grant.address();
                if ![PAGE_SIZE, PAGE_SIZE * 2].contains(&address) {
                    return Err("out-of-model physical frame");
                }
                let slot = (address / PAGE_SIZE - 1) as usize;
                if cells[slot].is_some() || history.iter().flatten().any(|old| old.grant == grant) {
                    return Err("live alias or reused generation");
                }
                let saved = Saved { grant, owner };
                history[history_len] = Some(saved);
                history_len += 1;
                cells[slot] = Some(Cell { saved, mapped: 0 });
            }
        } else if history_len != 0 {
            let saved = history[if *action == 10 { 0 } else { history_len - 1 }]
                .ok_or("history missing")?;
            let slot = (saved.grant.address() / PAGE_SIZE - 1) as usize;
            let current = cells[slot].filter(|cell| cell.saved.grant == saved.grant);
            match *action {
                3..=5 => {
                    let owner = if *action == 4 {
                        1 - saved.owner
                    } else {
                        saved.owner
                    };
                    let address = if *action == 5 { 0x5000 } else { 0x4000 };
                    let expected =
                        current.is_some_and(|cell| cell.mapped == 0 && cell.saved.owner == owner);
                    if ledger.bind(saved.grant, owners[owner], address) != expected {
                        return Err("binding authority mismatch");
                    }
                    if expected {
                        cells[slot].as_mut().ok_or("binding absent cell")?.mapped = address;
                    }
                }
                6..=8 => {
                    let owner = if *action == 7 {
                        1 - saved.owner
                    } else {
                        saved.owner
                    };
                    let address = if *action == 8 { 0x5000 } else { 0x4000 };
                    let expected = cells[slot]
                        .filter(|cell| cell.saved.owner == owner && cell.mapped == address);
                    let actual = ledger.retire(owners[owner], saved.grant.address(), address);
                    if actual != expected.map(|cell| cell.saved.grant) {
                        return Err("retirement identity mismatch");
                    }
                    if actual.is_some() {
                        cells[slot].as_mut().ok_or("retirement absent cell")?.mapped = 0;
                    }
                }
                9..=11 => {
                    let fail = *action == 11;
                    let authorized = current.is_some_and(|cell| cell.mapped == 0);
                    let mut called = false;
                    let actual = if bypass_pin
                        && *action == 9
                        && current.is_some_and(|cell| cell.mapped != 0)
                    {
                        // Deliberately faulty physical adapter used only to
                        // demonstrate that the independent oracle fails closed.
                        called = true;
                        bitmap.free_frame(saved.grant.address())
                    } else {
                        ledger.release(saved.grant, |address| {
                            called = true;
                            bitmap.free_frame_with(address, |_| !fail)
                        })
                    };
                    if called != authorized {
                        return Err("denied or pinned authority touched backing frame");
                    }
                    if actual != (authorized && !fail) {
                        return Err("release transaction mismatch");
                    }
                    if actual {
                        cells[slot] = None;
                    }
                }
                _ => return Err("unknown action"),
            }
        }
        let live = cells.iter().flatten().count();
        if ledger.live() != live
            || bitmap.allocated_frames() != live as u64
            || !bitmap.is_consistent()
            || !ledger.is_consistent(bitmap.allocated_frames(), |address| {
                bitmap.contains_live(address)
            })
        {
            return Err("ledger and bitmap diverged");
        }
        for (slot, cell) in cells.iter().enumerate() {
            let address = (slot as u64 + 1) * PAGE_SIZE;
            if bitmap.contains_live(address) != cell.is_some() {
                return Err("bitmap population mismatch");
            }
            for (owner, identity) in owners.iter().enumerate() {
                let expected = cell.filter(|c| c.saved.owner == owner);
                if ledger.find(*identity, address, Kind::User) != expected.map(|c| c.saved.grant)
                    || ledger.find(*identity, address, Kind::Table).is_some()
                {
                    return Err("owner or kind lookup mismatch");
                }
                for virtual_address in [0, 0x4000, 0x5000] {
                    if ledger.mapping_matches(*identity, address, virtual_address)
                        != expected
                            .is_some_and(|c| virtual_address != 0 && c.mapped == virtual_address)
                    {
                        return Err("mapping lookup mismatch");
                    }
                }
            }
        }
        for saved in history.iter().flatten() {
            let cell = cells
                .iter()
                .flatten()
                .find(|c| c.saved.grant == saved.grant);
            if ledger.is_live(saved.grant) != cell.is_some()
                || ledger.can_bind(saved.grant, owners[saved.owner])
                    != cell.is_some_and(|c| c.mapped == 0)
                || ledger.can_bind(saved.grant, owners[1 - saved.owner])
            {
                return Err("stale or foreign grant retained authority");
            }
        }
        for (index, owner) in owners.iter().enumerate() {
            if ledger.owner_frames(*owner)
                != cells
                    .iter()
                    .flatten()
                    .filter(|c| c.saved.owner == index)
                    .count()
            {
                return Err("owner accounting mismatch");
            }
        }
    }
    Ok(())
}

fn retain(path: &Path, trace: &[usize], error: &str) {
    let names: Vec<_> = trace.iter().map(|index| ACTIONS[*index]).collect();
    let record = format!("{{\"trace\":{trace:?},\"actions\":{names:?},\"reason\":{error:?}}}\n");
    fs::write(path, record).expect("retain counterexample");
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--replay") {
        let trace: Vec<usize> = args
            .get(1)
            .expect("comma-separated trace")
            .split(',')
            .map(|n| n.parse().expect("numeric action"))
            .collect();
        assert!(trace.len() <= MAX_DEPTH && trace.iter().all(|n| *n < ACTIONS.len()));
        replay(&trace, false).expect("replayed counterexample");
        println!("OWNERSHIP_MODEL_REPLAY_OK trace={trace:?}");
        return;
    }
    assert!(
        args.is_empty() || args == ["--oracle-self-test"],
        "unsupported model arguments"
    );
    if !args.is_empty() {
        let trace = [0, 3, 9];
        let error =
            replay(&trace, true).expect_err("oracle failed to detect a deliberately bypassed pin");
        retain(Path::new("oracle-counterexample.json"), &trace, error);
        println!("OWNERSHIP_MODEL_ORACLE_OK rejected=release-with-live-pin trace={trace:?}");
        return;
    }
    let started = Instant::now();
    let mut sequences = 0u64;
    let mut transitions = 0u64;
    for depth in 1..=MAX_DEPTH {
        let count = ACTIONS.len().pow(depth as u32);
        for word in 0..count {
            let mut trace = [0; MAX_DEPTH];
            let mut encoded = word;
            for action in trace[..depth].iter_mut().rev() {
                *action = encoded % ACTIONS.len();
                encoded /= ACTIONS.len();
            }
            let result = std::panic::catch_unwind(|| replay(&trace[..depth], false));
            if let Err(error) = result.unwrap_or(Err("production transition panicked")) {
                retain(Path::new("counterexample.json"), &trace[..depth], error);
                eprintln!(
                    "OWNERSHIP_MODEL_FAILED depth={depth} trace={:?} reason={error}",
                    &trace[..depth]
                );
                std::process::exit(1);
            }
            sequences += 1;
            transitions += depth as u64;
        }
        println!("OWNERSHIP_MODEL_DEPTH_OK depth={depth} sequences={count}");
    }
    println!("OWNERSHIP_MODEL_OK frames=2 owners=2 actions={} depth={MAX_DEPTH} sequences={sequences} transitions={transitions} merged_states=0 elapsed_ms={}", ACTIONS.len(), started.elapsed().as_millis());
}
