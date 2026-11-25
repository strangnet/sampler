//! Places captured packets on Recording time (spec: Windows support, INV-3).

use super::SAMPLE_RATE;

/// Timestamps are in 100 ns ticks, the unit WASAPI reports QPC positions in.
const TICKS_PER_SECOND: u64 = 10_000_000;

/// A gap of up to 10 ms before a packet counts as continuous.
const JITTER_TOLERANCE_FRAMES: usize = SAMPLE_RATE as usize / 100;

/// Keeps a Recording's length equal to the time since Record succeeded, by
/// saying how many frames of silence to write before each packet and while
/// no packets arrive. All counts are stereo frames at `SAMPLE_RATE`.
pub struct RecordingTimePlacer {
    start: u64,
    written: usize,
}

impl RecordingTimePlacer {
    /// `start` is the time Record succeeded.
    pub fn new(start: u64) -> Self {
        Self { start, written: 0 }
    }

    /// Frames of silence to write so the Recording reaches `now`, when no
    /// packets have arrived.
    pub fn fill_until(&mut self, now: u64) -> usize {
        let zeros = self.gap_to(now);
        self.written += zeros;
        zeros
    }

    /// Frames of silence to write before a packet of `frames` captured at
    /// `at`. A gap within the jitter tolerance, an overlap, or a packet from
    /// before the start gets none, and the packet is appended directly.
    pub fn place_packet(&mut self, at: u64, frames: usize) -> usize {
        let gap = self.gap_to(at);
        let zeros = if gap > JITTER_TOLERANCE_FRAMES {
            gap
        } else {
            0
        };
        self.written += zeros + frames;
        zeros
    }

    /// Frames between the end of what's written and `at`, or 0 when `at`
    /// falls inside what's written.
    fn gap_to(&self, at: u64) -> usize {
        self.frames_since_start(at).saturating_sub(self.written)
    }

    fn frames_since_start(&self, at: u64) -> usize {
        (at.saturating_sub(self.start) * u64::from(SAMPLE_RATE) / TICKS_PER_SECOND) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: u64 = 1_000_000_000;
    const SECOND: u64 = 10_000_000;
    const MS: u64 = SECOND / 1000;
    /// One frame at 48 kHz, rounded up to a whole tick.
    const FRAME: u64 = SECOND.div_ceil(48_000);

    #[test]
    fn no_silence_is_written_before_any_time_passes() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When
        let zeros = placer.fill_until(START);

        // Then
        assert_eq!(zeros, 0);
    }

    #[test]
    fn a_packet_at_the_start_is_written_without_silence() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When
        let zeros = placer.place_packet(START, 480);

        // Then
        assert_eq!(zeros, 0);
    }

    #[test]
    fn audio_without_a_break_gets_no_zeros() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When: three 10 ms packets, back to back.
        let zeros = [
            placer.place_packet(START, 480),
            placer.place_packet(START + 10 * MS, 480),
            placer.place_packet(START + 20 * MS, 480),
        ];

        // Then
        assert_eq!(zeros, [0, 0, 0]);
    }

    #[test]
    fn a_first_packet_two_seconds_late_is_preceded_by_two_seconds_of_silence() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When
        let zeros = placer.place_packet(START + 2 * SECOND, 480);

        // Then
        assert_eq!(zeros, 2 * 48_000);
    }

    #[test]
    fn a_three_second_gap_is_filled_with_three_seconds_of_silence() {
        // Given: 10 ms of audio from the start.
        let mut placer = RecordingTimePlacer::new(START);
        placer.place_packet(START, 480);

        // When: the next packet comes 3 s after the first one ended.
        let zeros = placer.place_packet(START + 10 * MS + 3 * SECOND, 480);

        // Then
        assert_eq!(zeros, 3 * 48_000);
    }

    #[test]
    fn five_seconds_without_packets_is_filled_with_five_seconds_of_silence() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When
        let zeros = placer.fill_until(START + 5 * SECOND);

        // Then
        assert_eq!(zeros, 5 * 48_000);
    }

    #[test]
    fn silence_already_filled_is_not_filled_again() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When: two waits with no packets, then a packet at the time filled up to.
        let zeros = [
            placer.fill_until(START + 2 * SECOND),
            placer.fill_until(START + 3 * SECOND),
            placer.place_packet(START + 3 * SECOND, 480),
        ];

        // Then
        assert_eq!(zeros, [2 * 48_000, 48_000, 0]);
    }

    #[test]
    fn a_gap_of_exactly_ten_milliseconds_counts_as_continuous() {
        // Given: 10 ms of audio from the start.
        let mut placer = RecordingTimePlacer::new(START);
        placer.place_packet(START, 480);

        // When: the next packet comes 10 ms after the first one ended.
        let zeros = placer.place_packet(START + 20 * MS, 480);

        // Then
        assert_eq!(zeros, 0);
    }

    #[test]
    fn a_gap_just_over_ten_milliseconds_is_filled() {
        // Given: 10 ms of audio from the start.
        let mut placer = RecordingTimePlacer::new(START);
        placer.place_packet(START, 480);

        // When: the next packet comes 10 ms and one frame after the first one ended.
        let zeros = placer.place_packet(START + 20 * MS + FRAME, 480);

        // Then
        assert_eq!(zeros, 481);
    }

    #[test]
    fn an_overlapping_packet_is_appended_directly() {
        // Given: 10 ms of audio from the start.
        let mut placer = RecordingTimePlacer::new(START);
        placer.place_packet(START, 480);

        // When: the next packet is stamped 5 ms before the first one ended,
        // and a third follows 10 ms after the second ended.
        let zeros = [
            placer.place_packet(START + 5 * MS, 480),
            placer.place_packet(START + 30 * MS, 480),
        ];

        // Then: nothing is trimmed, so the third gap is measured from all 960 frames.
        assert_eq!(zeros, [0, 0]);
    }

    #[test]
    fn jittered_timestamps_get_no_zeros() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When: back-to-back 10 ms packets, each stamped up to 5 ms off.
        let zeros = [
            placer.place_packet(START, 480),
            placer.place_packet(START + 10 * MS + 4 * MS, 480),
            placer.place_packet(START + 20 * MS - 3 * MS, 480),
            placer.place_packet(START + 30 * MS + 5 * MS, 480),
            placer.place_packet(START + 40 * MS - 5 * MS, 480),
            placer.place_packet(START + 50 * MS + 2 * MS, 480),
        ];

        // Then
        assert_eq!(zeros, [0; 6]);
    }

    #[test]
    fn a_device_clock_running_fast_gets_no_zeros() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When: for 1 s, every 10 ms brings 490 frames instead of 480.
        let zeros: usize = (0..100)
            .map(|n| placer.place_packet(START + n * 10 * MS, 490))
            .sum();

        // Then
        assert_eq!(zeros, 0);
    }

    #[test]
    fn a_packet_stamped_before_the_start_is_appended_directly() {
        // Given
        let mut placer = RecordingTimePlacer::new(START);

        // When: a packet captured 5 ms before Record succeeded, then the next one.
        let zeros = [
            placer.place_packet(START - 5 * MS, 480),
            placer.place_packet(START + 5 * MS, 480),
        ];

        // Then
        assert_eq!(zeros, [0, 0]);
    }
}
