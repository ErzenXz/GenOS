//! Bounded syscall scalar admission and endpoint wait/cancel/revoke sequences.
//! Addresses remain scalar metadata; this host target never copies user memory.
use genos_abi::UserChannelMessage;
use kernel::{
    capability::HandleTable,
    endpoint::{EndpointRole, EndpointState, PendingReceive, QueueResult},
    request::RequestSequence,
    syscall,
};

pub fn syscalls(bytes: &[u8]) -> bool {
    for record in bytes.chunks_exact(56).take(64) {
        let words: Vec<_> = record
            .chunks_exact(8)
            .map(|word| u64::from_le_bytes(word.try_into().unwrap()))
            .collect();
        let args: [u64; 6] = words[1..].try_into().unwrap();
        for number in [words[0], words[0] & 0xff] {
            let first = syscall::dispatch(number, args);
            assert_eq!(
                first,
                syscall::dispatch(number, args),
                "admission must not mutate state"
            );
            if number > 0xff {
                assert!(first.is_err());
            }
        }
        // Wide arithmetic supplies an independent overflow/range oracle.
        let [address, length, start, size, _, _] = args;
        let end = u128::from(address) + u128::from(length);
        let range_end = u128::from(start) + u128::from(size);
        let expected = length != 0
            && address >= start
            && end <= range_end
            && end <= u128::from(u64::MAX)
            && range_end <= u128::from(u64::MAX);
        assert_eq!(
            syscall::validate_user_buffer(address, length, start, size),
            expected
        );
        // These scalar special cases have exact successful and rejection results.
        assert_eq!(
            syscall::dispatch(syscall::SYSCALL_YIELD, [0; 6]),
            Ok(syscall::SyscallAction::Yield)
        );
        let mut dirty = [0; 6];
        dirty[(words[0] % 6) as usize] = length.max(1);
        assert!(syscall::dispatch(syscall::SYSCALL_YIELD, dirty).is_err());
    }
    true
}

pub fn endpoints(bytes: &[u8]) -> bool {
    let mut states = [EndpointState::new(1), EndpointState::new(2)];
    let mut tables = [HandleTable::<4>::new(), HandleTable::<4>::new()];
    let mut requests = [RequestSequence::new(), RequestSequence::new()];
    let mut queued: [std::collections::VecDeque<UserChannelMessage>; 2] = Default::default();
    let mut handles = [[0u64; 16]; 2];
    let mut next = [0usize; 2];
    let mut last_request = [0u64; 2];
    let mut seen = std::collections::BTreeSet::new();
    for record in bytes.chunks_exact(20).take(64) {
        let owner = usize::from(record[1] & 1);
        let other = owner ^ 1;
        let index = usize::from(record[2] & 15);
        let raw = u64::from_le_bytes(record[4..12].try_into().unwrap());
        let value = u64::from_le_bytes(record[12..20].try_into().unwrap());
        let handle = match record[3] % 3 {
            0 => handles[owner][index],
            1 => handles[other][index],
            _ => raw,
        };
        match record[0] % 9 {
            0 => {
                if let Some(handle) = states[owner].create(&mut tables[owner]) {
                    assert!(
                        seen.insert(handle),
                        "generation reused a retired capability"
                    );
                    assert_eq!(states[owner].queue_depth(), 0);
                    handles[owner][next[owner] % 16] = handle;
                    next[owner] += 1;
                }
            }
            1 => {
                if let Some(generation) = states[other].published_generation() {
                    let role = EndpointRole::Send {
                        target_pid: other as u8 + 1,
                        target_generation: generation,
                    };
                    if let Some(handle) = states[owner].allocate(&mut tables[owner], role) {
                        assert!(seen.insert(handle));
                        assert_eq!(
                            states[owner].send_capability(&tables[owner], handle),
                            Some((other as u8 + 1, generation))
                        );
                        handles[owner][next[owner] % 16] = handle;
                        next[owner] += 1;
                    }
                }
            }
            2 => {
                let before = (
                    states[owner].published_generation(),
                    states[owner].queue_depth(),
                    states[owner].pending_receive(),
                    tables[owner].len(),
                );
                match states[owner].close(&mut tables[owner], handle) {
                    Some(EndpointRole::Receive { .. }) => {
                        queued[owner].clear();
                    }
                    Some(EndpointRole::Send { .. }) => {}
                    None => assert_eq!(
                        (
                            states[owner].published_generation(),
                            states[owner].queue_depth(),
                            states[owner].pending_receive(),
                            tables[owner].len()
                        ),
                        before
                    ),
                }
                assert!(states[owner]
                    .receive_generation(&tables[owner], handle)
                    .is_none());
                assert!(states[owner]
                    .send_capability(&tables[owner], handle)
                    .is_none());
            }
            3 => {
                let request = requests[owner].allocate().unwrap();
                assert!(request > last_request[owner]);
                last_request[owner] = request;
                let pending = PendingReceive {
                    handle,
                    generation: value,
                    address: raw,
                    length: if record[2] & 0x80 == 0 {
                        genos_abi::USER_CHANNEL_MESSAGE_SIZE
                    } else {
                        value
                    },
                };
                let expected = states[owner].receive_generation(&tables[owner], handle)
                    == Some(value)
                    && pending.length == genos_abi::USER_CHANNEL_MESSAGE_SIZE;
                let before = states[owner].pending_receive();
                assert_eq!(
                    states[owner].park_receive(&tables[owner], pending),
                    expected
                );
                assert_eq!(
                    states[owner].pending_receive(),
                    if expected { Some(pending) } else { before }
                );
            }
            4 => {
                states[owner].clear_pending_receive();
                assert_eq!(states[owner].pending_receive(), None);
            }
            5 => {
                let message = UserChannelMessage {
                    sender_pid: raw,
                    value,
                };
                let expected = if states[owner].published_generation().is_none() {
                    QueueResult::Unpublished
                } else if queued[owner].iter().any(|prior| prior.sender_pid == raw) {
                    QueueResult::DuplicateProducer
                } else if queued[owner].len() == genos_abi::USER_ENDPOINT_QUEUE_CAPACITY {
                    QueueResult::Full
                } else {
                    QueueResult::Queued(queued[owner].len() + 1)
                };
                assert_eq!(states[owner].enqueue(message), expected);
                if matches!(expected, QueueResult::Queued(_)) {
                    queued[owner].push_back(message);
                }
            }
            6 => {
                assert_eq!(states[owner].pop_message(), queued[owner].pop_front());
            }
            7 => {
                states[owner].clear(&mut tables[owner]);
                queued[owner].clear();
                assert!(states[owner].resources_are_revoked());
                assert!(tables[owner].is_empty());
            }
            _ => {
                states[owner].revoke_send_handles(&mut tables[owner], other as u8 + 1, value);
            }
        }
        for owner in 0..2 {
            assert_eq!(states[owner].queue_depth(), queued[owner].len());
            assert!(states[owner].registry_is_consistent(&tables[owner]));
            assert!(tables[owner].len() <= 4);
            for &handle in &handles[owner ^ 1] {
                assert!(states[owner]
                    .receive_generation(&tables[owner], handle)
                    .is_none());
                assert!(states[owner]
                    .send_capability(&tables[owner], handle)
                    .is_none());
            }
        }
    }
    for owner in 0..2 {
        states[owner].clear(&mut tables[owner]);
        assert!(states[owner].resources_are_revoked());
        assert!(tables[owner].is_empty());
        for handle in handles[owner] {
            assert!(states[owner]
                .receive_generation(&tables[owner], handle)
                .is_none());
            assert!(states[owner]
                .send_capability(&tables[owner], handle)
                .is_none());
        }
    }
    true
}
