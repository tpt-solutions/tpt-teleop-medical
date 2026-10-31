//! Link quality for multi-site telesurgery (spec.txt §4.6, §5.1).
//!
//! The surgeon and the patient can be in different cities. Each command carries
//! the time it was sent, and the receiver reports when it arrived. This module
//! turns those one-way delays into a link state:
//!
//! * [`LinkState::Nominal`]: the 99th percentile over the recent window is within
//!   the end-to-end budget (10 ms, spec.txt §5.1).
//! * [`LinkState::Degraded`]: over the budget, but within the horizon that the
//!   predictive display can mask (50 ms, spec.txt §4.6). Motion continues, and
//!   the surgeon is warned.
//! * [`LinkState::Lost`]: beyond the horizon, silent for longer than the horizon,
//!   or a clock that runs backwards. The predictive display cannot mask this, so
//!   the adapter must stop.
//!
//! The window is a fixed array. Nothing here allocates, so the monitor can run on
//! the control path.

/// Recent samples kept for the percentile. Fixed, so there is no allocation.
pub const WINDOW: usize = 128;

/// The end-to-end budget, in microseconds (spec.txt §5.1: <10 ms).
pub const BUDGET_US: u64 = 10_000;

/// The longest delay the predictive display masks, in microseconds
/// (spec.txt §4.6: 50 ms).
pub const HORIZON_US: u64 = 50_000;

/// What the link looks like now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkState {
    /// Within budget.
    Nominal,
    /// Over budget, but within the masking horizon. The surgeon is warned.
    Degraded,
    /// Beyond the horizon, silent, or not trustworthy. The adapter must stop.
    Lost,
}

/// Tracks one-way command delay over a sliding window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMonitor {
    delays_us: [u64; WINDOW],
    count: usize,
    next: usize,
    last_received_us: Option<u64>,
    last_delay_us: u64,
}

impl Default for LinkMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkMonitor {
    /// Creates a monitor with no samples. Until the first sample, the state is
    /// [`LinkState::Lost`]: an unseen link is not assumed to be working.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            delays_us: [0; WINDOW],
            count: 0,
            next: 0,
            last_received_us: None,
            last_delay_us: 0,
        }
    }

    /// Records one command. `sent_us` is when it left the operator's side, and
    /// `received_us` is when it arrived. Returns the state after this sample.
    pub fn observe(&mut self, sent_us: u64, received_us: u64) -> LinkState {
        if received_us < sent_us {
            // The clocks disagree, so this delay is not a measurement.
            self.last_received_us = None;
            return LinkState::Lost;
        }
        self.delays_us[self.next] = received_us - sent_us;
        self.next = (self.next + 1) % WINDOW;
        self.count = (self.count + 1).min(WINDOW);
        self.last_received_us = Some(received_us);
        self.last_delay_us = received_us - sent_us;
        self.state_at(received_us)
    }

    /// Returns the state at `now_us`, given the samples so far. A link silent
    /// for longer than the horizon is lost, even if its last sample was good.
    #[must_use]
    pub fn state_at(&self, now_us: u64) -> LinkState {
        let Some(last) = self.last_received_us else {
            return LinkState::Lost;
        };
        if now_us.saturating_sub(last) > HORIZON_US || self.last_delay_us > HORIZON_US {
            // A single command past the horizon is lost even when the percentile
            // is still within budget: the display cannot mask that command.
            return LinkState::Lost;
        }
        let p99 = self.percentile_99();
        if p99 > HORIZON_US {
            LinkState::Lost
        } else if p99 > BUDGET_US {
            LinkState::Degraded
        } else {
            LinkState::Nominal
        }
    }

    /// The 99th-percentile delay over the window, in microseconds. Returns zero
    /// before the first sample.
    #[must_use]
    pub fn percentile_99(&self) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let mut sorted = [0_u64; WINDOW];
        sorted[..self.count].copy_from_slice(&self.delays_us[..self.count]);
        sorted[..self.count].sort_unstable();
        // The smallest index whose rank covers 99% of the samples.
        let rank = (self.count * 99).div_ceil(100);
        sorted[rank - 1]
    }

    /// Returns how many samples the window holds.
    #[must_use]
    pub const fn samples(&self) -> usize {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unseen_link_is_lost_not_assumed_good() {
        let m = LinkMonitor::new();
        assert_eq!(m.state_at(0), LinkState::Lost);
    }

    #[test]
    fn low_delay_is_nominal() {
        let mut m = LinkMonitor::new();
        for k in 0..WINDOW as u64 {
            let t = k * 1_000;
            assert_eq!(m.observe(t, t + 2_000), LinkState::Nominal);
        }
        assert_eq!(m.percentile_99(), 2_000);
    }

    #[test]
    fn delay_between_budget_and_horizon_is_degraded() {
        let mut m = LinkMonitor::new();
        let mut last = LinkState::Lost;
        for k in 0..WINDOW as u64 {
            let t = k * 1_000;
            last = m.observe(t, t + 30_000);
        }
        assert_eq!(last, LinkState::Degraded);
        assert_eq!(m.percentile_99(), 30_000);
    }

    #[test]
    fn delay_past_the_horizon_is_lost() {
        let mut m = LinkMonitor::new();
        assert_eq!(m.observe(0, 60_000), LinkState::Lost);
    }

    #[test]
    fn a_few_spikes_below_the_p99_rank_do_not_degrade_the_link() {
        // One sample in 128 is slow. With a full window the 99th percentile still
        // lands on a fast one.
        let mut m = LinkMonitor::new();
        for k in 0..WINDOW as u64 {
            let t = k * 1_000;
            let delay = if k == 10 { 40_000 } else { 2_000 };
            m.observe(t, t + delay);
        }
        assert_eq!(m.percentile_99(), 2_000);
    }

    #[test]
    fn one_command_past_the_horizon_is_lost_even_in_a_healthy_window() {
        let mut m = LinkMonitor::new();
        for k in 0..WINDOW as u64 {
            let t = k * 1_000;
            m.observe(t, t + 2_000);
        }
        assert_eq!(m.observe(1_000_000, 1_080_000), LinkState::Lost);
    }

    #[test]
    fn a_silent_link_is_lost_once_the_horizon_has_passed() {
        let mut m = LinkMonitor::new();
        m.observe(0, 1_000);
        assert_eq!(m.state_at(1_000 + HORIZON_US), LinkState::Nominal);
        assert_eq!(m.state_at(1_000 + HORIZON_US + 1), LinkState::Lost);
    }

    #[test]
    fn a_clock_that_runs_backwards_is_lost() {
        let mut m = LinkMonitor::new();
        m.observe(0, 1_000);
        assert_eq!(m.observe(5_000, 4_000), LinkState::Lost);
        assert_eq!(m.state_at(4_000), LinkState::Lost);
    }

    #[test]
    fn the_window_forgets_old_samples() {
        let mut m = LinkMonitor::new();
        for k in 0..WINDOW as u64 {
            let t = k * 1_000;
            m.observe(t, t + 30_000);
        }
        assert_eq!(m.percentile_99(), 30_000);
        for k in 0..WINDOW as u64 {
            let t = 1_000_000 + k * 1_000;
            m.observe(t, t + 1_000);
        }
        assert_eq!(m.samples(), WINDOW);
        assert_eq!(m.percentile_99(), 1_000);
    }
}
