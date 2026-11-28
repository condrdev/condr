//! The sidebars' widths as they spring open and shut: one spring per sidebar of a window,
//! the Agents section's, which the window's chrome reads and its terminals wait out, so a
//! terminal is resized once, where the sidebar settles, rather than on every frame.

use gpui_kit::*;
use std::cell::Cell;
use std::time::Instant;

/// The Agents section's fold spring: critically damped, so a sidebar never passes its
/// width; about 90% of the way in 130 ms, settled by 250 ms.
const SIDEBAR_SPRING: SpringConfig = SpringConfig::new(900., 60., 1.);
/// Settled within a fifth of a percent of the way, under a pixel on the widest sidebar.
const SIDEBAR_EPSILON: f32 = 0.002;

/// How far open one sidebar is drawn, 0 shut to 1 open. Its cells step as the window
/// draws, which reads it through `&WindowView`.
pub(super) struct SidebarMotion {
    spring: Cell<SpringState>,
    /// Where it is heading, `None` until the first frame, which lands there at once.
    target: Cell<Option<f32>>,
    stepped_at: Cell<Instant>,
}

impl Default for SidebarMotion {
    fn default() -> Self {
        Self {
            spring: Cell::new(SpringState {
                position: 0.,
                velocity: 0.,
            }),
            target: Cell::new(None),
            stepped_at: Cell::new(Instant::now()),
        }
    }
}

impl SidebarMotion {
    /// How far open it is drawn this frame, stepping toward `open`; the window asks for
    /// another frame while it is [`moving`](Self::moving). Under reduced motion it lands
    /// at once.
    pub(super) fn progress(&self, open: bool, reduce_motion: bool) -> f32 {
        let target = if open { 1. } else { 0. };
        let now = Instant::now();
        let mut state = self.spring.get();
        match self.target.replace(Some(target)) {
            // The first frame, or a sidebar no one saw move: it is where it is heading.
            None => state = settled(target),
            _ if reduce_motion => state = settled(target),
            // A new heading starts from here, its time counted from this frame.
            Some(previous) if previous != target => {}
            Some(_) => {
                let elapsed = now.duration_since(self.stepped_at.get()).as_secs_f32();
                state = SIDEBAR_SPRING.step(state, target, elapsed);
                if SIDEBAR_SPRING.is_settled(state, target, SIDEBAR_EPSILON) {
                    state = settled(target);
                }
            }
        }
        self.spring.set(state);
        self.stepped_at.set(now);
        state.position
    }

    /// Whether it is on its way, so a terminal keeps its size until it settles.
    pub(super) fn moving(&self) -> bool {
        self.target
            .get()
            .is_some_and(|target| self.spring.get().position != target)
    }

    /// Lands it where it is heading on the next frame, for a change no one asked to see
    /// move: another Workspace's right sidebar, open or shut as it was left.
    pub(super) fn land(&self) {
        self.target.set(None);
    }
}

fn settled(target: f32) -> SpringState {
    SpringState {
        position: target,
        velocity: 0.,
    }
}

/// `shut` to `open` by `progress`.
pub(super) fn width_between(shut: Pixels, open: Pixels, progress: f32) -> Pixels {
    shut + (open - shut) * progress
}

#[cfg(test)]
mod tests {
    use super::SidebarMotion;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn a_sidebar_lands_on_its_first_frame_and_springs_when_toggled() {
        let motion = SidebarMotion::default();
        assert_eq!(motion.progress(true, false), 1.);
        assert!(!motion.moving());
        // Toggled, it starts from where it is and is on its way.
        assert_eq!(motion.progress(false, false), 1.);
        assert!(motion.moving());
        sleep(Duration::from_millis(40));
        let drawn = motion.progress(false, false);
        assert!(0. < drawn && drawn < 1., "partway shut: {drawn}");
        sleep(Duration::from_millis(400));
        assert_eq!(motion.progress(false, false), 0.);
        assert!(!motion.moving());
        // Landed, as for another Workspace's sidebar, or under reduced motion: no motion.
        motion.land();
        assert_eq!(motion.progress(true, false), 1.);
        assert_eq!(motion.progress(false, true), 0.);
        assert!(!motion.moving());
    }
}
