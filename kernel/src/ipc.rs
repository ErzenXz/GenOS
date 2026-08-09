use genos_abi::UserChannelMessage;

/// Fixed-capacity FIFO of endpoint messages. Admission is fair: a sender that
/// already has a queued message is rejected until that message is popped, so a
/// single producer cannot fill an endpoint and starve the others.
pub struct ChannelQueue<const CAPACITY: usize> {
    messages: [UserChannelMessage; CAPACITY],
    head: usize,
    len: usize,
}

impl<const CAPACITY: usize> ChannelQueue<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            messages: [UserChannelMessage::empty(); CAPACITY],
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, message: UserChannelMessage) -> bool {
        if CAPACITY == 0 || self.len == CAPACITY || self.contains_sender(message.sender_pid) {
            return false;
        }
        let tail = (self.head + self.len) % CAPACITY;
        self.messages[tail] = message;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> Option<UserChannelMessage> {
        if self.len == 0 {
            return None;
        }
        let message = self.messages[self.head];
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        Some(message)
    }

    pub fn contains_sender(&self, sender_pid: u64) -> bool {
        (0..self.len)
            .any(|offset| self.messages[(self.head + offset) % CAPACITY].sender_pid == sender_pid)
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const CAPACITY: usize> Default for ChannelQueue<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(sender_pid: u64, value: u64) -> UserChannelMessage {
        UserChannelMessage { sender_pid, value }
    }

    #[test]
    fn queue_is_fifo_and_rejects_overflow() {
        let mut queue = ChannelQueue::<2>::new();
        assert!(queue.is_empty());
        assert!(queue.push(message(1, 10)));
        assert!(queue.push(message(2, 20)));
        assert!(!queue.push(message(3, 30)));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop(), Some(message(1, 10)));
        assert_eq!(queue.pop(), Some(message(2, 20)));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn queue_reuses_slots_after_wraparound() {
        let mut queue = ChannelQueue::<3>::new();
        assert!(queue.push(message(1, 1)));
        assert!(queue.push(message(2, 2)));
        assert_eq!(queue.pop(), Some(message(1, 1)));
        assert!(queue.push(message(3, 3)));
        assert!(queue.push(message(1, 4)));
        assert_eq!(queue.pop(), Some(message(2, 2)));
        assert_eq!(queue.pop(), Some(message(3, 3)));
        assert_eq!(queue.pop(), Some(message(1, 4)));
        assert!(queue.is_empty());
    }

    #[test]
    fn queue_rejects_a_second_message_from_the_same_sender() {
        let mut queue = ChannelQueue::<4>::new();
        assert!(queue.push(message(5, 1)));
        assert!(!queue.push(message(5, 2)));
        assert!(queue.push(message(6, 3)));
        assert!(!queue.push(message(6, 4)));
        assert_eq!(queue.len(), 2);
        assert!(queue.contains_sender(5));
        assert!(!queue.contains_sender(7));
    }

    #[test]
    fn sender_is_admitted_again_once_its_message_is_popped() {
        let mut queue = ChannelQueue::<4>::new();
        assert!(queue.push(message(5, 1)));
        assert!(queue.push(message(6, 2)));
        assert!(!queue.push(message(5, 3)));
        assert_eq!(queue.pop(), Some(message(5, 1)));
        assert!(!queue.contains_sender(5));
        assert!(queue.push(message(5, 3)));
        assert_eq!(queue.pop(), Some(message(6, 2)));
        assert_eq!(queue.pop(), Some(message(5, 3)));
    }
}
