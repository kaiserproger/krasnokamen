//! Transient running-time prefix from the existing durable climate images.
use crate::config::CoreError;
use crate::dimensions::Dimension;
use crate::journal::JournalEntry;
use crate::weather::GameClockImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Point {
    tick: u64,
    image: GameClockImage,
}

/// Shared by every dimension while folding a recovery. Retained until recovery
/// ends: a later dimension or compact anchor can still need an older boundary.
#[derive(Debug)]
pub(crate) struct ReplayGameClock {
    initial: Point,
    changes: Vec<Point>,
    through_tick: u64,
}

impl ReplayGameClock {
    pub(crate) fn new(tick: u64, image: Option<GameClockImage>) -> Result<Self, CoreError> {
        let image = image.unwrap_or_default();
        image.held_ticks(tick)?;
        Ok(Self {
            initial: Point { tick, image },
            changes: Vec::new(),
            through_tick: tick,
        })
    }

    /// Inspect unrouted batches once, before any dimension folds entity images.
    pub(crate) fn observe_batches(
        &mut self,
        batches: &[(u64, u64, Vec<JournalEntry>)],
    ) -> Result<(), CoreError> {
        for (_, hi, entries) in batches {
            if *hi <= self.through_tick {
                continue;
            }
            self.observe_entries(entries, Dimension::Overworld, *hi)?;
            self.through_tick = *hi;
        }
        Ok(())
    }

    fn observe_entries(
        &mut self,
        entries: &[JournalEntry],
        dimension: Dimension,
        hi: u64,
    ) -> Result<(), CoreError> {
        for entry in entries {
            match entry {
                JournalEntry::Dimension { dimension, entries } => {
                    self.observe_entries(entries, *dimension, hi)?;
                }
                JournalEntry::Climate { value } if dimension == Dimension::Overworld => {
                    self.observe(value.game_clock, hi)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn observe(&mut self, image: GameClockImage, hi: u64) -> Result<(), CoreError> {
        image.held_ticks(hi)?;
        let previous = self.changes.last().copied().unwrap_or(self.initial);
        if image == previous.image {
            return Ok(());
        }
        let (held_ticks, game_ticks) = match (previous.image, image) {
            (GameClockImage::Running { held_ticks }, GameClockImage::Held { game_ticks, .. })
            | (GameClockImage::Held { game_ticks, .. }, GameClockImage::Running { held_ticks }) => {
                (held_ticks, game_ticks)
            }
            _ => return Err(invalid("ambiguous same-mode counter transition")),
        };
        let tick = held_ticks
            .checked_add(game_ticks)
            .and_then(|tick| tick.checked_add(1))
            .ok_or_else(|| invalid("transition boundary overflows native time"))?;
        if let GameClockImage::Held {
            held_since: Some(since),
            ..
        } = image
            && since != tick
        {
            return Err(invalid(
                "held boundary disagrees with the preceding running counter",
            ));
        }
        let point = Point { tick, image };
        if let Ok(index) = self.changes.binary_search_by_key(&tick, |point| point.tick) {
            return if self.changes[index] == point {
                Ok(())
            } else {
                Err(invalid("conflicting images at one transition boundary"))
            };
        }
        if tick <= previous.tick || tick <= self.through_tick || tick > hi {
            return Err(invalid(
                "transition boundary is outside its ordered native interval",
            ));
        }
        self.changes.push(point);
        Ok(())
    }

    fn point_at(&self, tick: u64) -> Point {
        let index = self.changes.partition_point(|point| point.tick <= tick);
        index
            .checked_sub(1)
            .map_or(self.initial, |index| self.changes[index])
    }

    pub(crate) fn game_ticks(&self, tick: u64) -> u64 {
        let point = self.point_at(tick);
        match point.image {
            GameClockImage::Running { held_ticks } => tick.saturating_sub(held_ticks),
            GameClockImage::Held {
                game_ticks,
                held_since: Some(since),
            } => {
                if tick >= since {
                    game_ticks
                } else {
                    tick.saturating_sub(since - 1 - game_ticks)
                }
            }
            GameClockImage::Held {
                held_since: None, ..
            } => unreachable!("validated held clock"),
        }
    }

    pub(crate) fn between(&self, from: u64, to: u64) -> u64 {
        self.game_ticks(to).saturating_sub(self.game_ticks(from))
    }

    /// Native output boundary of the most recent running tick.
    pub(crate) fn last_running_at(&self, tick: u64) -> u64 {
        match self.point_at(tick).image {
            GameClockImage::Running { .. } => tick,
            GameClockImage::Held {
                held_since: Some(since),
                ..
            } => tick.min(since - 1),
            GameClockImage::Held {
                held_since: None, ..
            } => unreachable!("validated held clock"),
        }
    }

    /// Visit (start, end] spans containing only running native output ticks.
    /// The start may be held; fresh memories still belong to last_running_at(start).
    pub(crate) fn running_intervals(&self, from: u64, to: u64, mut visit: impl FnMut(u64, u64)) {
        let mut start = from;
        let mut image = self.point_at(from).image;
        let first = self.changes.partition_point(|point| point.tick <= from);
        for point in &self.changes[first..] {
            if point.tick > to {
                break;
            }
            let end = point.tick - 1;
            if matches!(image, GameClockImage::Running { .. }) && start < end {
                visit(start, end);
            }
            start = end;
            image = point.image;
        }
        if matches!(image, GameClockImage::Running { .. }) && start < to {
            visit(start, to);
        }
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::Storage(format!("entity replay game clock: {reason}"))
}
