//! Where a frame's draw runs: on the loop's own thread, or on a render thread of its own.
//!
//! The draw's side of the app — the renderer, the overlay painter, the render stage — is one value,
//! and it is in exactly one place at a time. [`FramePipeline::sync`] is the sync point: it waits for
//! a draw in flight to finish and hands that value back. [`FramePipeline::draw`] gives it away to
//! draw a frame: on the loop's thread it runs there and then; on a render thread it runs while the
//! loop goes on to the next frame's update, and the next `sync` waits for it.
//!
//! Generic over what is handed back and forth, so the hand-over is tested without a GPU.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

use super::stage::ConditionalSend;

/// What a draw tells the loop, which acts on it at the next sync point.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DrawReport {
    /// The frame the screenshot request asked to exit after has been written.
    pub(crate) exit: bool,
    /// How long the draw's side was busy with the frame — the acquire, the draw, the paint, the
    /// present. Measured by the pipeline around the job, whatever the job reports.
    pub(crate) busy: std::time::Duration,
    /// The swapchain is stale (`Outdated`/`Lost`) and has to be configured again — by the loop, at
    /// the sync point, where nothing else submits (see `draw_and_present`).
    pub(crate) reconfigure: bool,
}

impl DrawReport {
    fn merge(&mut self, other: DrawReport) {
        self.exit |= other.exit;
        self.reconfigure |= other.reconfigure;
        // The latest frame's: the reports between two syncs are of one frame at most.
        self.busy = other.busy;
    }
}

/// Runs a frame's job and times it.
fn timed<G>(job: DrawJob<G>, side: &mut G) -> DrawReport {
    let started = super::FrameInstant::now();
    let mut report = job(side);
    report.busy = started.elapsed();
    report
}

/// One frame's draw, given the draw's side.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type DrawJob<G> = Box<dyn FnOnce(&mut G) -> DrawReport + Send>;
/// One frame's draw, given the draw's side.
#[cfg(target_arch = "wasm32")]
pub(crate) type DrawJob<G> = Box<dyn FnOnce(&mut G) -> DrawReport>;

/// The draw's side of the app, and where its draws run.
pub(crate) struct FramePipeline<G: ConditionalSend + 'static> {
    /// The draw's side while it is at home — always, on the loop's thread; between a `sync` and
    /// the next `draw`, on a render thread.
    home: Option<G>,
    /// What the draws since the last [`take_report`](Self::take_report) said.
    report: DrawReport,
    #[cfg(not(target_arch = "wasm32"))]
    thread: Option<RenderThread<G>>,
}

#[cfg(not(target_arch = "wasm32"))]
struct RenderThread<G> {
    jobs: Option<mpsc::SyncSender<(G, DrawJob<G>)>>,
    done: mpsc::Receiver<(G, DrawReport)>,
    handle: Option<thread::JoinHandle<()>>,
}

impl<G: ConditionalSend + 'static> FramePipeline<G> {
    /// Draws on the loop's own thread: [`draw`](Self::draw) runs the frame before it returns.
    pub(crate) fn inline(side: G) -> Self {
        Self {
            home: Some(side),
            report: DrawReport::default(),
            #[cfg(not(target_arch = "wasm32"))]
            thread: None,
        }
    }

    /// Draws on a render thread of its own: [`draw`](Self::draw) hands the frame over and returns,
    /// and the loop's next [`sync`](Self::sync) waits for it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn threaded(side: G) -> Self {
        // One frame in flight at most: the draw's side is a single value, so a second job could
        // not be sent before the first came back anyway. The bound only says so.
        let (jobs, inbox) = mpsc::sync_channel::<(G, DrawJob<G>)>(1);
        let (outbox, done) = mpsc::sync_channel::<(G, DrawReport)>(1);
        let handle = thread::Builder::new()
            .name(crate::RENDER_THREAD_NAME.into())
            .spawn(move || {
                while let Ok((mut side, job)) = inbox.recv() {
                    let report = timed(job, &mut side);
                    if outbox.send((side, report)).is_err() {
                        break;
                    }
                }
            })
            .expect("the render thread could not be started");
        Self {
            home: Some(side),
            report: DrawReport::default(),
            thread: Some(RenderThread { jobs: Some(jobs), done, handle: Some(handle) }),
        }
    }

    /// Whether draws run on a render thread.
    pub(crate) fn is_threaded(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.thread.is_some();
        #[cfg(target_arch = "wasm32")]
        false
    }

    /// The sync point: waits for a draw in flight, then hands the draw's side back.
    ///
    /// A draw that panicked on the render thread panics here, with its message — the loop must not
    /// go on as though the frame had been drawn, nor wait for a draw that will never return. A new
    /// panic rather than `resume_unwind`, which skips the panic hook: the app's hook opens the
    /// crash dialog only on the main thread, and this is where the panic reaches it.
    pub(crate) fn sync(&mut self) -> &mut G {
        #[cfg(not(target_arch = "wasm32"))]
        if self.home.is_none() {
            let thread = self.thread.as_mut().expect("the draw's side is away only on a render thread");
            match thread.done.recv() {
                Ok((side, report)) => {
                    self.home = Some(side);
                    self.report.merge(report);
                }
                Err(_) => {
                    let handle = thread.handle.take().expect("the render thread is joined once");
                    match handle.join() {
                        Err(payload) => {
                            let message = payload
                                .downcast_ref::<&str>()
                                .copied()
                                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                                .unwrap_or("a panic with no message");
                            let at = crate::take_render_thread_panic_location()
                                .map(|location| format!(" at {location}"))
                                .unwrap_or_default();
                            panic!("the draw panicked on the render thread{at}: {message}")
                        }
                        Ok(()) => panic!("the render thread ended with a frame in flight"),
                    }
                }
            }
        }
        self.home.as_mut().expect("the draw's side is home after a sync")
    }

    /// Draws a frame: now, or on the render thread. The draw's side is away until the next
    /// [`sync`](Self::sync).
    pub(crate) fn draw(&mut self, job: DrawJob<G>) {
        let mut side = self.home.take().expect("a frame is drawn only after a sync");
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(thread) = self.thread.as_mut() {
            let jobs = thread.jobs.as_ref().expect("the job channel lives as long as the pipeline");
            // A failed send means the thread is gone, which only a panic in a draw does; the side
            // that came back with the error is dropped here, and `sync` raises the panic.
            let _ = jobs.send((side, job));
            return;
        }
        let report = timed(job, &mut side);
        self.report.merge(report);
        self.home = Some(side);
    }

    /// What the draws finished by the last sync said, cleared.
    pub(crate) fn take_report(&mut self) -> DrawReport {
        std::mem::take(&mut self.report)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<G> Drop for RenderThread<G> {
    /// Closes the job channel and joins the thread, so a draw in flight finishes — and the
    /// renderer it holds is dropped — before the rest of the app is. A panic on the render thread
    /// is not raised again here: `sync` raises it, and a drop during an unwind must not.
    fn drop(&mut self) {
        // With the job channel closed the thread finishes the frame it holds, hands it back into
        // `done` (which has room for it) and leaves; the frame is then dropped with `done`.
        self.jobs = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The draw's side in these tests: the frames drawn, in order.
    type Side = Vec<u32>;

    fn record(frame: u32) -> DrawJob<Side> {
        Box::new(move |side: &mut Side| {
            side.push(frame);
            DrawReport::default()
        })
    }

    /// Inline, a frame is drawn before `draw` returns.
    #[test]
    fn inline_a_frame_is_drawn_before_draw_returns() {
        let mut pipe = FramePipeline::inline(Side::new());
        pipe.sync();
        pipe.draw(record(0));
        assert_eq!(pipe.home.as_deref(), Some(&[0][..]));
        assert!(!pipe.is_threaded());
    }

    /// On a render thread the frame's draw and the loop's next update run at the same time: the
    /// draw waits for a token the loop sends only after `draw` has returned, and gets it.
    #[test]
    fn threaded_the_draw_runs_while_the_loop_goes_on() {
        let (token_tx, token_rx) = mpsc::channel::<()>();
        let mut pipe = FramePipeline::threaded(Vec::<bool>::new());
        pipe.sync();
        pipe.draw(Box::new(move |side: &mut Vec<bool>| {
            side.push(token_rx.recv_timeout(Duration::from_secs(10)).is_ok());
            DrawReport::default()
        }));
        // The loop's next update — which inline could not reach until the draw had finished.
        token_tx.send(()).unwrap();
        assert_eq!(pipe.sync(), &vec![true], "the draw did not overlap the loop");
    }

    /// The sync point waits for the draw in flight: what the loop gets back has every frame drawn,
    /// in order, whichever thread drew them — and the same as inline.
    #[test]
    fn the_sync_point_hands_back_every_frame_in_order() {
        let mut inline = FramePipeline::inline(Side::new());
        let mut threaded = FramePipeline::threaded(Side::new());
        for frame in 0..50 {
            inline.sync();
            inline.draw(record(frame));
            threaded.sync();
            threaded.draw(Box::new(move |side: &mut Side| {
                // Slow enough that the loop reaches the next sync before the draw is done.
                thread::sleep(Duration::from_micros(200));
                side.push(frame);
                DrawReport::default()
            }));
        }
        assert_eq!(threaded.sync(), inline.sync());
        assert_eq!(threaded.sync().len(), 50);
    }

    /// What a draw reports reaches the loop at a sync point after it — once.
    #[test]
    fn a_draws_report_reaches_the_loop_once() {
        for mut pipe in [FramePipeline::inline(Side::new()), FramePipeline::threaded(Side::new())] {
            pipe.sync();
            pipe.draw(Box::new(|_: &mut Side| DrawReport { exit: true, ..Default::default() }));
            pipe.sync();
            assert!(pipe.take_report().exit, "threaded: {}", pipe.is_threaded());
            assert!(!pipe.take_report().exit);
        }
    }

    /// The report carries how long the draw's side was busy with the frame, measured around the
    /// job — on the render thread or not.
    #[test]
    fn a_draws_report_says_how_long_it_was_busy() {
        for mut pipe in [FramePipeline::inline(Side::new()), FramePipeline::threaded(Side::new())] {
            pipe.sync();
            pipe.draw(Box::new(|_: &mut Side| {
                thread::sleep(Duration::from_millis(20));
                DrawReport::default()
            }));
            pipe.sync();
            let busy = pipe.take_report().busy;
            assert!(busy >= Duration::from_millis(20), "threaded: {}, busy {busy:?}", pipe.is_threaded());
            assert!(busy < Duration::from_secs(2));
        }
    }

    /// A draw that panics on the render thread panics the loop at the next sync point, with the
    /// draw's message — not a hang, and not a frame taken as drawn.
    ///
    /// With the engine's panic hook installed, the message names where the draw panicked: the
    /// crash dialog opens for this second panic, on the main thread, and would otherwise point at
    /// this file's `sync`.
    #[test]
    fn a_panic_in_the_draw_reaches_the_sync_point() {
        crate::setup_panic_hook();
        let mut pipe = FramePipeline::threaded(Side::new());
        pipe.sync();
        let line = line!() + 1;
        pipe.draw(Box::new(|_: &mut Side| panic!("the draw failed")));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pipe.sync();
        }))
        .expect_err("the sync point went on as though the frame had been drawn");
        let message = caught.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(message.starts_with("the draw panicked on the render thread"), "{message}");
        assert!(message.ends_with(": the draw failed"), "{message}");
        assert!(message.contains(&format!("pipeline.rs:{line}")), "the draw's own line is not named: {message}");
    }

    /// Dropping the pipeline with a frame in flight waits for that frame: the renderer it holds is
    /// dropped before the pipeline is gone, not on a thread left running behind the app.
    #[test]
    fn dropping_the_pipeline_finishes_the_frame_in_flight() {
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut pipe = FramePipeline::threaded(Side::new());
        pipe.sync();
        let flag = finished.clone();
        pipe.draw(Box::new(move |_: &mut Side| {
            thread::sleep(Duration::from_millis(50));
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            DrawReport::default()
        }));
        drop(pipe);
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
    }
}
