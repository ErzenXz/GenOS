pub struct MessageQueue<const CAPACITY: usize> {
    values: [u64; CAPACITY],
    head: usize,
    len: usize,
}

impl<const CAPACITY: usize> MessageQueue<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            values: [0; CAPACITY],
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, value: u64) -> bool {
        if CAPACITY == 0 || self.len == CAPACITY {
            return false;
        }
        let tail = (self.head + self.len) % CAPACITY;
        self.values[tail] = value;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        let value = self.values[self.head];
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        Some(value)
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const CAPACITY: usize> Default for MessageQueue<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_is_fifo_and_rejects_overflow() {
        let mut queue = MessageQueue::<2>::new();
        assert!(queue.is_empty());
        assert!(queue.push(10));
        assert!(queue.push(20));
        assert!(!queue.push(30));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop(), Some(10));
        assert_eq!(queue.pop(), Some(20));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn queue_reuses_slots_after_wraparound() {
        let mut queue = MessageQueue::<3>::new();
        assert!(queue.push(1));
        assert!(queue.push(2));
        assert_eq!(queue.pop(), Some(1));
        assert!(queue.push(3));
        assert!(queue.push(4));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), Some(4));
    }
}
