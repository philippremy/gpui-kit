use gpui::{Context, Pixels, Task, px};
use instant::Duration;

static INTERVAL: Duration = Duration::from_millis(500);
static PAUSE_DELAY: Duration = Duration::from_millis(300);

// On Windows, Linux, we should use integer to avoid blurry cursor.
#[cfg(not(target_os = "macos"))]
pub(super) const CURSOR_WIDTH: Pixels = px(2.);
#[cfg(target_os = "macos")]
pub(super) const CURSOR_WIDTH: Pixels = px(1.5);

/// To manage the Input cursor blinking.
///
/// It will start blinking with a interval of 500ms.
/// Every loop will notify the view to update the `visible`, and Input will observe this update to touch repaint.
///
/// The input painter will check if this in visible state, then it will draw the cursor.
pub(crate) struct BlinkCursor {
    visible: bool,
    paused: bool,
    /// Whether the input owning this cursor has focus. Only a focused input blinks: without this,
    /// the first programmatic cursor move (setting the text, say) started a timer loop that
    /// re-armed itself every [`INTERVAL`] for the life of the input and re-rendered it twice a
    /// second even though nothing was focused or visible.
    focused: bool,
    epoch: usize,

    _task: Task<()>,
}

impl BlinkCursor {
    pub(crate) fn new() -> Self {
        Self {
            visible: false,
            paused: false,
            focused: false,
            epoch: 0,
            _task: Task::ready(()),
        }
    }

    /// Start the blinking
    pub(crate) fn start(&mut self, cx: &mut Context<Self>) {
        self.focused = true;
        self.blink(self.epoch, cx);
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        self.focused = false;
        self.epoch = 0;
        cx.notify();
    }

    fn next_epoch(&mut self) -> usize {
        self.epoch += 1;
        self.epoch
    }

    fn blink(&mut self, epoch: usize, cx: &mut Context<Self>) {
        if self.paused || epoch != self.epoch || !self.focused {
            self.visible = true;
            return;
        }

        self.visible = !self.visible;
        cx.notify();

        // Schedule the next blink
        let epoch = self.next_epoch();
        self._task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INTERVAL).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| this.blink(epoch, cx));
            }
        });
    }

    pub(crate) fn visible(&self) -> bool {
        // Keep showing the cursor if paused
        self.paused || self.visible
    }

    /// Show the cursor immediately and restart the idle delay before blinking resumes.
    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        // An unfocused input shows no caret, so there is nothing to keep visible or resume.
        if !self.focused {
            return;
        }
        self.paused = true;
        self.visible = true;
        cx.notify();

        // Every pause replaces the pending timer, keeping repeated input visible.
        let epoch = self.next_epoch();
        self._task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE_DELAY).await;

            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    this.paused = false;
                    this.blink(epoch, cx);
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};

    #[gpui::test]
    fn repeated_pauses_keep_cursor_visible_until_idle(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        cursor.update(cx, |cursor, _| cursor.focused = true);
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
        for _ in 0..5 {
            cursor.update(cx, |cursor, cx| cursor.pause(cx));
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(200));
            cx.run_until_parked();
            assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
        }
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert!(!cursor.read_with(cx, |cursor, _| cursor.visible()));
        cx.executor().advance_clock(INTERVAL);
        cx.run_until_parked();
        assert!(cursor.read_with(cx, |cursor, _| cursor.visible()));
    }

    /// An input that is not focused must not run a blink loop: programmatic cursor moves call
    /// `pause`, and that used to start a timer that re-armed itself forever.
    #[gpui::test]
    fn unfocused_cursor_never_blinks(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let _subscription = cx.update(|cx| {
            let notifications = notifications.clone();
            cx.observe(&cursor, move |_, _| notifications.set(notifications.get() + 1))
        });
        for _ in 0..3 {
            cursor.update(cx, |cursor, cx| cursor.pause(cx));
            cx.run_until_parked();
        }
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 0, "an unfocused cursor stays silent");
    }

    #[gpui::test]
    fn cursor_stops_blinking_once_it_loses_focus(cx: &mut TestAppContext) {
        let cursor = cx.new(|_| BlinkCursor::new());
        cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.run_until_parked();
        cursor.update(cx, |cursor, cx| cursor.stop(cx));
        cx.run_until_parked();

        let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let _subscription = cx.update(|cx| {
            let notifications = notifications.clone();
            cx.observe(&cursor, move |_, _| notifications.set(notifications.get() + 1))
        });
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 0, "no blink timer outlives the focus");
    }
}
