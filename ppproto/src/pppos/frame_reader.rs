use core::ops::Range;

use super::crc::crc16;

#[derive(Copy, Clone, Debug)]
enum State {
    Start,
    Address,
    Data,
    Complete,
}

pub struct FrameReader {
    state: State,
    escape: bool,
    len: usize,
}

impl FrameReader {
    pub fn new() -> Self {
        Self {
            state: State::Start,
            escape: false,
            len: 0,
        }
    }

    pub fn receive(&mut self) -> Option<Range<usize>> {
        match self.state {
            State::Complete => {
                let len = self.len;
                self.len = 0;
                self.state = State::Address;
                Some(1..len - 2)
            }
            _ => None,
        }
    }

    pub fn consume(&mut self, buf: &mut [u8], data: &[u8]) -> usize {
        for (i, &b) in data.iter().enumerate() {
            match (self.state, b) {
                (State::Start, 0x7e) => self.state = State::Address,
                (State::Start, _) => {}
                (State::Address, 0xff) => self.state = State::Data,
                (State::Address, 0x7e) => self.state = State::Address,
                (State::Address, _) => self.state = State::Start,
                (State::Data, 0x7e) => {
                    // End of packet
                    let ok = self.len >= 3
                        && buf[0] == 0x03
                        && crc16(0x00FF, &buf[..self.len]) == 0xf0b8;
                    self.state = if ok { State::Complete } else { State::Address }
                }
                (State::Data, 0x7d) => self.escape = true,
                (State::Data, mut b) => {
                    if self.escape {
                        self.escape = false;
                        b ^= 0x20;
                    }
                    if self.len == usize::MAX || self.len >= buf.len() {
                        self.state = State::Start;
                        self.len = 0;
                    } else {
                        buf[self.len as usize] = b;
                        self.len += 1;
                    }
                }
                // When we have received a frame, do not consume more data until it's processed with receive()
                (State::Complete, _) => return i,
            }
        }

        // All consumed
        data.len()
    }
}
