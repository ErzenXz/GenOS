//! Per-process endpoint authority and bounded fair queues.
//!
//! The process manager owns one state and one unified handle registry, mutating
//! them together on the admitted BSP. This module owns no globals, addresses it
//! dereferences, scheduler context, or hardware access. Callers supply the same
//! registry on every operation. Returned wait metadata is opaque: user-copy and
//! process wakeup remain the process manager's responsibility.
//! Closing receive authority drops its queue and parked receive; revocation
//! matches the exact remote PID/generation. Clear retains the generation counter
//! so local slot reuse cannot resurrect a stale handle. Admission is bounded by
//! ABI capacities; rejected handles never index before tag and slot validation.

use crate::{
    capability::{HandleKind, HandleTable},
    ipc::ChannelQueue,
};

const ENDPOINT_HANDLE_CAPACITY: usize = genos_abi::USER_ENDPOINT_HANDLE_CAPACITY as usize;
const ENDPOINT_QUEUE_CAPACITY: usize = genos_abi::USER_ENDPOINT_QUEUE_CAPACITY;
const HANDLE_RIGHT_USE: u64 = 1;
const ENDPOINT_HANDLE_TAG: u64 = 0xe9 << 56;
const ENDPOINT_HANDLE_TAG_MASK: u64 = 0xff << 56;
const ENDPOINT_GENERATION_MAX: u64 = u32::MAX as u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueResult {
    Queued(usize),
    DuplicateProducer,
    Full,
    Unpublished,
}

/// Metadata of a receive that already validated its output buffer and is now
/// parked on the published endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingReceive {
    pub handle: u64,
    pub generation: u64,
    pub address: u64,
    pub length: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointRole {
    /// Names the generation of the endpoint this process publishes itself.
    Receive { generation: u64 },
    /// Names one remote endpoint: a pid plus the generation it published.
    Send {
        target_pid: u8,
        target_generation: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EndpointCapability {
    handle: u64,
    owner_pid: u8,
    generation: u64,
    slot: u8,
    role: EndpointRole,
}

struct PublishedEndpoint {
    generation: u64,
    queue: ChannelQueue<ENDPOINT_QUEUE_CAPACITY>,
}

/// Per-process endpoint authority: a small capability table, the single
/// endpoint this process publishes, and the receive it is parked on.
pub struct EndpointState {
    owner_pid: u8,
    handles: [Option<EndpointCapability>; ENDPOINT_HANDLE_CAPACITY],
    next_generation: u64,
    published: Option<PublishedEndpoint>,
    pending_receive: Option<PendingReceive>,
}

const fn endpoint_handle(owner_pid: u8, generation: u64, slot: usize) -> u64 {
    ENDPOINT_HANDLE_TAG | ((owner_pid as u64) << 40) | (generation << 8) | (slot as u64 + 1)
}

fn endpoint_handle_slot(handle: u64) -> Option<usize> {
    if handle & ENDPOINT_HANDLE_TAG_MASK != ENDPOINT_HANDLE_TAG {
        return None;
    }
    let slot = (handle & 0xff) as usize;
    (1..=ENDPOINT_HANDLE_CAPACITY)
        .contains(&slot)
        .then(|| slot - 1)
}

impl EndpointState {
    pub const fn new(owner_pid: u8) -> Self {
        Self {
            owner_pid,
            handles: [None; ENDPOINT_HANDLE_CAPACITY],
            next_generation: 1,
            published: None,
            pending_receive: None,
        }
    }

    pub fn published_generation(&self) -> Option<u64> {
        self.published.as_ref().map(|endpoint| endpoint.generation)
    }

    pub fn queue_depth(&self) -> usize {
        self.published
            .as_ref()
            .map_or(0, |endpoint| endpoint.queue.len())
    }

    fn next_generation(&mut self) -> Option<u64> {
        let generation = self.next_generation;
        if generation > ENDPOINT_GENERATION_MAX {
            return None;
        }
        self.next_generation = generation + 1;
        Some(generation)
    }

    pub fn allocate<const N: usize>(
        &mut self,
        handles: &mut HandleTable<N>,
        role: EndpointRole,
    ) -> Option<u64> {
        let slot = self.handles.iter().position(Option::is_none)?;
        let generation = self.next_generation()?;
        let handle = endpoint_handle(self.owner_pid, generation, slot);
        let kind = match role {
            EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
            EndpointRole::Send { .. } => HandleKind::EndpointSend,
        };
        if !handles.register(handle, kind, HANDLE_RIGHT_USE) {
            return None;
        }
        self.handles[slot] = Some(EndpointCapability {
            handle,
            owner_pid: self.owner_pid,
            generation,
            slot: slot as u8,
            role,
        });
        Some(handle)
    }

    /// Resolves a handle this process owns. The tag, decoded slot, owner pid and
    /// generation must reproduce the handle exactly, so neither a guessed value
    /// nor another process' handle nor a stale local handle can ever resolve.
    fn capability<const N: usize>(
        &self,
        handles: &HandleTable<N>,
        handle: u64,
    ) -> Option<EndpointCapability> {
        let slot = endpoint_handle_slot(handle)?;
        let capability = self.handles[slot]?;
        let kind = match capability.role {
            EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
            EndpointRole::Send { .. } => HandleKind::EndpointSend,
        };
        (capability.handle == handle
            && capability.owner_pid == self.owner_pid
            && capability.slot as usize == slot
            && endpoint_handle(capability.owner_pid, capability.generation, slot) == handle
            && handles.allows(handle, kind, HANDLE_RIGHT_USE))
        .then_some(capability)
    }

    pub fn send_capability<const N: usize>(
        &self,
        handles: &HandleTable<N>,
        handle: u64,
    ) -> Option<(u8, u64)> {
        match self.capability(handles, handle)?.role {
            EndpointRole::Send {
                target_pid,
                target_generation,
            } => Some((target_pid, target_generation)),
            EndpointRole::Receive { .. } => None,
        }
    }

    /// A receive capability is only usable while it still names the exact
    /// endpoint generation this process publishes right now.
    pub fn receive_generation<const N: usize>(
        &self,
        handles: &HandleTable<N>,
        handle: u64,
    ) -> Option<u64> {
        let EndpointRole::Receive { generation } = self.capability(handles, handle)?.role else {
            return None;
        };
        (self.published_generation() == Some(generation)).then_some(generation)
    }

    /// Publishes an empty endpoint and returns its owned receive handle. Fails
    /// when an endpoint is already published or the handle table is full.
    pub fn create<const N: usize>(&mut self, handles: &mut HandleTable<N>) -> Option<u64> {
        if self.published.is_some() || !self.handles.iter().any(Option::is_none) {
            return None;
        }
        let slot = self.handles.iter().position(Option::is_none)?;
        let generation = self.next_generation()?;
        let handle = endpoint_handle(self.owner_pid, generation, slot);
        if !handles.register(handle, HandleKind::EndpointReceive, HANDLE_RIGHT_USE) {
            return None;
        }
        self.handles[slot] = Some(EndpointCapability {
            handle,
            owner_pid: self.owner_pid,
            generation,
            slot: slot as u8,
            role: EndpointRole::Receive { generation },
        });
        self.published = Some(PublishedEndpoint {
            generation,
            queue: ChannelQueue::new(),
        });
        Some(handle)
    }

    /// Closing a send capability revokes only that handle; closing the receive
    /// capability also drops the queue and unpublishes the endpoint.
    pub fn close<const N: usize>(
        &mut self,
        handles: &mut HandleTable<N>,
        handle: u64,
    ) -> Option<EndpointRole> {
        let capability = self.capability(handles, handle)?;
        if let EndpointRole::Receive { generation } = capability.role {
            if self.published_generation() != Some(generation) {
                return None;
            }
            self.published = None;
            self.pending_receive = None;
        }
        let kind = match capability.role {
            EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
            EndpointRole::Send { .. } => HandleKind::EndpointSend,
        };
        if !handles.unregister(handle, kind) {
            return None;
        }
        self.handles[capability.slot as usize] = None;
        Some(capability.role)
    }

    /// Drops every send capability naming one remote endpoint generation.
    pub fn revoke_send_handles<const N: usize>(
        &mut self,
        handles: &mut HandleTable<N>,
        target_pid: u8,
        target_generation: u64,
    ) -> usize {
        let mut revoked = 0;
        for entry in self.handles.iter_mut() {
            let names_target = matches!(
                entry.map(|capability| capability.role),
                Some(EndpointRole::Send {
                    target_pid: pid,
                    target_generation: generation,
                }) if pid == target_pid && generation == target_generation
            );
            if names_target {
                let handle = entry.expect("matched endpoint capability").handle;
                let removed = handles.unregister(handle, HandleKind::EndpointSend);
                debug_assert!(removed);
                *entry = None;
                revoked += 1;
            }
        }
        revoked
    }

    pub fn clear<const N: usize>(&mut self, handles: &mut HandleTable<N>) {
        for capability in self.handles.iter().flatten() {
            let kind = match capability.role {
                EndpointRole::Receive { .. } => HandleKind::EndpointReceive,
                EndpointRole::Send { .. } => HandleKind::EndpointSend,
            };
            let removed = handles.unregister(capability.handle, kind);
            debug_assert!(removed);
        }
        self.clear_payload();
    }

    /// Drop metadata after the process manager has cleared the unified registry.
    /// Use `clear` when endpoint handles still need individual unregistration.
    pub fn clear_payload(&mut self) {
        self.handles = [None; ENDPOINT_HANDLE_CAPACITY];
        self.published = None;
        self.pending_receive = None;
    }
    pub fn len(&self) -> usize {
        self.handles.iter().flatten().count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn resources_are_revoked(&self) -> bool {
        self.is_empty() && self.published.is_none() && self.pending_receive.is_none()
    }
    pub fn registry_is_consistent<const N: usize>(&self, handles: &HandleTable<N>) -> bool {
        self.handles.iter().flatten().all(|capability| {
            self.capability(handles, capability.handle) == Some(*capability)
                && !handles.allows(capability.handle, HandleKind::File, 0)
        })
    }
    pub fn pending_receive(&self) -> Option<PendingReceive> {
        self.pending_receive
    }
    /// The caller has validated the user buffer; this module stores metadata only.
    pub fn park_receive<const N: usize>(
        &mut self,
        handles: &HandleTable<N>,
        pending: PendingReceive,
    ) -> bool {
        if self.receive_generation(handles, pending.handle) != Some(pending.generation)
            || pending.length != genos_abi::USER_CHANNEL_MESSAGE_SIZE
        {
            return false;
        }
        self.pending_receive = Some(pending);
        true
    }
    pub fn clear_pending_receive(&mut self) {
        self.pending_receive = None;
    }
    pub fn enqueue(&mut self, message: genos_abi::UserChannelMessage) -> QueueResult {
        let Some(endpoint) = self.published.as_mut() else {
            return QueueResult::Unpublished;
        };
        if endpoint.queue.contains_sender(message.sender_pid) {
            return QueueResult::DuplicateProducer;
        }
        if !endpoint.queue.push(message) {
            return QueueResult::Full;
        }
        QueueResult::Queued(endpoint.queue.len())
    }
    pub fn pop_message(&mut self) -> Option<genos_abi::UserChannelMessage> {
        self.published
            .as_mut()
            .and_then(|endpoint| endpoint.queue.pop())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genos_abi::{UserChannelMessage, USER_CHANNEL_MESSAGE_SIZE};
    type Registry = HandleTable<20>;

    fn published() -> (EndpointState, Registry, u64) {
        let mut state = EndpointState::new(7);
        let mut registry = Registry::new();
        let handle = state.create(&mut registry).unwrap();
        (state, registry, handle)
    }
    fn send(target_pid: u8, target_generation: u64) -> EndpointRole {
        EndpointRole::Send {
            target_pid,
            target_generation,
        }
    }
    fn message(sender_pid: u64, value: u64) -> UserChannelMessage {
        UserChannelMessage { sender_pid, value }
    }

    #[test]
    fn exact_handle_owner_tag_generation_and_registry_are_required() {
        let (state, registry, handle) = published();
        assert_eq!(state.receive_generation(&registry, handle), Some(1));
        for candidate in [
            0,
            handle ^ (1 << 40),
            handle ^ (1 << 8),
            handle & !ENDPOINT_HANDLE_TAG_MASK,
        ] {
            assert_eq!(state.receive_generation(&registry, candidate), None);
        }
        for slot in 0..=255 {
            let candidate = (handle & !255) | slot;
            if candidate != handle {
                assert_eq!(state.receive_generation(&registry, candidate), None);
            }
        }
        assert_eq!(
            EndpointState::new(8).receive_generation(&registry, handle),
            None
        );
        assert_eq!(state.receive_generation(&Registry::new(), handle), None);
        assert!(state.registry_is_consistent(&registry));
    }

    #[test]
    fn unified_registry_kind_and_rights_cannot_be_bypassed() {
        let (state, mut registry, handle) = published();
        assert!(registry.unregister(handle, HandleKind::EndpointReceive));
        for (kind, rights) in [
            (HandleKind::File, 1),
            (HandleKind::EndpointSend, 1),
            (HandleKind::EndpointReceive, 0),
        ] {
            assert!(registry.register(handle, kind, rights));
            assert_eq!(state.receive_generation(&registry, handle), None);
            assert!(!state.registry_is_consistent(&registry));
            assert!(registry.unregister(handle, kind));
        }
    }

    #[test]
    fn publication_and_handle_capacity_fail_without_partial_authority() {
        let (mut state, mut registry, receive) = published();
        assert_eq!(state.create(&mut registry), None);
        for pid in 0..ENDPOINT_HANDLE_CAPACITY - 1 {
            assert!(state.allocate(&mut registry, send(pid as u8, 1)).is_some());
        }
        assert_eq!(state.allocate(&mut registry, send(99, 1)), None);
        assert_eq!(state.len(), ENDPOINT_HANDLE_CAPACITY);
        assert_eq!(state.receive_generation(&registry, receive), Some(1));
        let mut empty = EndpointState::new(8);
        let mut full_registry = HandleTable::<0>::new();
        assert_eq!(empty.create(&mut full_registry), None);
        assert_eq!(empty.allocate(&mut full_registry, send(9, 1)), None);
        assert!(empty.resources_are_revoked());
    }

    #[test]
    fn roles_and_remote_revocation_are_exact() {
        let (mut state, mut registry, receive) = published();
        let matching = state.allocate(&mut registry, send(9, 3)).unwrap();
        let other_generation = state.allocate(&mut registry, send(9, 4)).unwrap();
        let other_pid = state.allocate(&mut registry, send(10, 3)).unwrap();
        assert_eq!(state.send_capability(&registry, receive), None);
        assert_eq!(state.receive_generation(&registry, matching), None);
        assert_eq!(state.revoke_send_handles(&mut registry, 9, 3), 1);
        assert_eq!(state.send_capability(&registry, matching), None);
        assert_eq!(
            state.send_capability(&registry, other_generation),
            Some((9, 4))
        );
        assert_eq!(state.send_capability(&registry, other_pid), Some((10, 3)));
        assert_eq!(state.revoke_send_handles(&mut registry, 9, 3), 0);
        assert!(state.registry_is_consistent(&registry));
    }

    #[test]
    fn queue_preserves_first_message_and_denies_duplicate_or_excess_producers() {
        let (mut state, _, _) = published();
        assert_eq!(state.enqueue(message(1, 100)), QueueResult::Queued(1));
        assert_eq!(
            state.enqueue(message(1, 200)),
            QueueResult::DuplicateProducer
        );
        for pid in 2..=ENDPOINT_QUEUE_CAPACITY as u64 {
            assert_eq!(
                state.enqueue(message(pid, pid)),
                QueueResult::Queued(pid as usize)
            );
        }
        assert_eq!(state.enqueue(message(99, 99)), QueueResult::Full);
        assert_eq!(state.queue_depth(), ENDPOINT_QUEUE_CAPACITY);
        assert_eq!(state.pop_message(), Some(message(1, 100)));
        for pid in 2..=ENDPOINT_QUEUE_CAPACITY as u64 {
            assert_eq!(state.pop_message(), Some(message(pid, pid)));
        }
        assert_eq!(state.pop_message(), None);
    }

    #[test]
    fn closing_receive_retires_queue_and_waiter_but_preserves_other_sends() {
        let (mut state, mut registry, receive) = published();
        let outgoing = state.allocate(&mut registry, send(9, 3)).unwrap();
        let pending = PendingReceive {
            handle: receive,
            generation: 1,
            address: 0x4010,
            length: USER_CHANNEL_MESSAGE_SIZE,
        };
        assert!(!state.park_receive(
            &registry,
            PendingReceive {
                generation: 2,
                ..pending
            }
        ));
        assert!(!state.park_receive(
            &registry,
            PendingReceive {
                length: 1,
                ..pending
            }
        ));
        assert!(state.park_receive(&registry, pending));
        assert_eq!(state.pending_receive(), Some(pending));
        assert_eq!(state.enqueue(message(1, 2)), QueueResult::Queued(1));
        assert_eq!(
            state.close(&mut registry, receive),
            Some(EndpointRole::Receive { generation: 1 })
        );
        assert_eq!(state.pending_receive(), None);
        assert_eq!(state.published_generation(), None);
        assert_eq!(state.queue_depth(), 0);
        assert_eq!(state.enqueue(message(1, 2)), QueueResult::Unpublished);
        assert_eq!(state.send_capability(&registry, outgoing), Some((9, 3)));
        assert_eq!(state.close(&mut registry, receive), None);
    }

    #[test]
    fn send_close_does_not_unpublish_or_destroy_other_authority() {
        let (mut state, mut registry, receive) = published();
        let outgoing = state.allocate(&mut registry, send(9, 3)).unwrap();
        state.enqueue(message(1, 2));
        assert_eq!(state.close(&mut registry, outgoing), Some(send(9, 3)));
        assert_eq!(state.send_capability(&registry, outgoing), None);
        assert_eq!(state.receive_generation(&registry, receive), Some(1));
        assert_eq!(state.pop_message(), Some(message(1, 2)));
        assert!(state.registry_is_consistent(&registry));
    }

    #[test]
    fn cleanup_retains_generation_and_cannot_resurrect_old_handles() {
        let (mut state, mut registry, first) = published();
        let outgoing = state.allocate(&mut registry, send(9, 3)).unwrap();
        state.enqueue(message(1, 2));
        state.clear(&mut registry);
        assert!(registry.is_empty());
        assert!(state.resources_are_revoked());
        let second = state.create(&mut registry).unwrap();
        assert_ne!(first, second);
        assert_eq!(state.receive_generation(&registry, first), None);
        assert_eq!(state.send_capability(&registry, outgoing), None);
        assert!(state.receive_generation(&registry, second).is_some());
    }

    #[test]
    fn generation_exhaustion_never_wraps_into_earlier_authority() {
        let mut state = EndpointState::new(7);
        let mut registry = Registry::new();
        state.next_generation = ENDPOINT_GENERATION_MAX;
        let last = state.create(&mut registry).unwrap();
        state.close(&mut registry, last).unwrap();
        assert_eq!(state.create(&mut registry), None);
        assert_eq!(state.allocate(&mut registry, send(9, 1)), None);
        assert!(registry.is_empty());
        assert!(state.resources_are_revoked());
    }
}
