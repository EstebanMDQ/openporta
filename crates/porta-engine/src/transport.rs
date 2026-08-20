//! Transport state machine with a sample-accurate playhead.
//!
//! Punch-in is `record()` from Playing; punch-out is `play()` from
//! Recording. Reaching the tape end stops the transport (REQ-104). Seeks
//! are instant and allowed only while Stopped, which keeps record passes
//! contiguous and undo bookkeeping simple.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransportState {
    Stopped,
    Playing,
    Recording,
}

pub struct Transport {
    state: TransportState,
    playhead: usize,
    tape_len: usize,
}

impl Transport {
    pub fn new(tape_len: usize) -> Self {
        Self {
            state: TransportState::Stopped,
            playhead: 0,
            tape_len,
        }
    }

    pub fn state(&self) -> TransportState {
        self.state
    }

    pub fn playhead(&self) -> usize {
        self.playhead
    }

    pub fn is_stopped(&self) -> bool {
        self.state == TransportState::Stopped
    }

    /// Start playback, or punch out of recording into playback.
    pub fn play(&mut self) {
        if self.playhead < self.tape_len {
            self.state = TransportState::Playing;
        }
    }

    pub fn stop(&mut self) {
        self.state = TransportState::Stopped;
    }

    /// Engage recording: from Stopped (record from standstill) or Playing
    /// (punch-in). No-op at the end of the tape.
    pub fn record(&mut self) {
        if self.playhead < self.tape_len {
            self.state = TransportState::Recording;
        }
    }

    /// Instant seek; only honored while Stopped. Returns success.
    pub fn seek(&mut self, pos: usize) -> bool {
        if self.state != TransportState::Stopped {
            return false;
        }
        self.playhead = pos.min(self.tape_len);
        true
    }

    pub fn rewind(&mut self, samples: usize) -> bool {
        let target = self.playhead.saturating_sub(samples);
        self.seek(target)
    }

    pub fn fast_forward(&mut self, samples: usize) -> bool {
        let target = self.playhead.saturating_add(samples);
        self.seek(target)
    }

    /// Advance the playhead by up to `n` samples while rolling. Clamps at
    /// the tape end and auto-stops there. Returns samples actually
    /// advanced (0 while Stopped).
    pub fn advance(&mut self, n: usize) -> usize {
        if self.state == TransportState::Stopped {
            return 0;
        }
        let advanced = n.min(self.tape_len - self.playhead);
        self.playhead += advanced;
        if self.playhead == self.tape_len {
            self.state = TransportState::Stopped;
        }
        advanced
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TransportState::*;

    #[test]
    fn transitions() {
        let mut t = Transport::new(1000);
        assert_eq!(t.state(), Stopped);
        t.play();
        assert_eq!(t.state(), Playing);
        t.record(); // punch in
        assert_eq!(t.state(), Recording);
        t.play(); // punch out
        assert_eq!(t.state(), Playing);
        t.stop();
        assert_eq!(t.state(), Stopped);
        t.record(); // record from standstill
        assert_eq!(t.state(), Recording);
    }

    #[test]
    fn advance_is_sample_accurate() {
        let mut t = Transport::new(10_000);
        assert_eq!(t.advance(128), 0, "no motion while stopped");
        t.play();
        assert_eq!(t.advance(128), 128);
        assert_eq!(t.advance(300), 300);
        assert_eq!(t.playhead(), 428);
    }

    #[test]
    fn end_of_tape_stops_transport() {
        let mut t = Transport::new(1000);
        t.play();
        assert_eq!(t.advance(900), 900);
        assert_eq!(t.advance(200), 100, "clamped at tape end");
        assert_eq!(t.state(), Stopped);
        assert_eq!(t.playhead(), 1000);
        t.play();
        assert_eq!(t.state(), Stopped, "cannot roll from the very end");
        t.record();
        assert_eq!(t.state(), Stopped, "cannot record at the very end");
    }

    #[test]
    fn seek_only_while_stopped() {
        let mut t = Transport::new(1000);
        assert!(t.seek(500));
        assert_eq!(t.playhead(), 500);
        t.play();
        assert!(!t.seek(0), "seek refused while rolling");
        assert_eq!(t.playhead(), 500);
        t.stop();
        assert!(t.rewind(200));
        assert_eq!(t.playhead(), 300);
        assert!(t.fast_forward(10_000));
        assert_eq!(t.playhead(), 1000, "ff clamps to tape end");
        assert!(t.rewind(usize::MAX));
        assert_eq!(t.playhead(), 0);
    }
}
