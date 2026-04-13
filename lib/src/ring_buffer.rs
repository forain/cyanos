//! Lock-free single-producer / single-consumer ring buffer.

pub struct RingBuffer<T, const N: usize> {
    buf: [Option<T>; N],
    head: usize,
    tail: usize,
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    pub const fn new() -> Self {
        Self { buf: [None; N], head: 0, tail: 0 }
    }

    pub fn push(&mut self, val: T) -> bool {
        let next = (self.tail + 1) % N;
        if next == self.head { return false; }
        self.buf[self.tail] = Some(val);
        self.tail = next;
        true
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.head == self.tail { return None; }
        let val = self.buf[self.head].take();
        self.head = (self.head + 1) % N;
        val
    }

    pub fn is_empty(&self) -> bool { self.head == self.tail }
}
