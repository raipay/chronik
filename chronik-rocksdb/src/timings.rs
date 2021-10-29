use std::{
    fmt::{Display, Formatter},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Timings {
    timings: Vec<(&'static str, Duration)>,
    current_timer: Instant,
}

impl Timings {
    pub fn start_timer(&mut self) {
        self.current_timer = Instant::now();
    }

    pub fn stop_timer(&mut self, name: &'static str) {
        let dt = self.current_timer.elapsed();
        self.add_timing(name, dt);
    }

    pub fn reset(&mut self) {
        for (_, dt) in self.timings.iter_mut() {
            *dt = Duration::ZERO;
        }
    }

    pub fn add(&mut self, other: &Timings) {
        for &(name, dt) in other.timings.iter() {
            self.add_timing(name, dt);
        }
    }

    fn add_timing(&mut self, name: &'static str, dt: Duration) {
        match self
            .timings
            .iter_mut()
            .find(|(timing_name, _)| timing_name == &name)
        {
            Some((_, timing_dt)) => *timing_dt += dt,
            None => self.timings.push((name, dt)),
        }
    }
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            timings: Default::default(),
            current_timer: Instant::now(),
        }
    }
}

impl Display for Timings {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let max_name_len = self
            .timings
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or_default();
        for &(name, duration) in self.timings.iter() {
            writeln!(
                f,
                " {:>width$}: {:10.4}ms",
                name,
                duration.as_secs_f64() * 1000.0,
                width = max_name_len,
            )?;
        }
        Ok(())
    }
}
