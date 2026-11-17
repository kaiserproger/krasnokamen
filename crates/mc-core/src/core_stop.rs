//! Stop-path diagnostics shared by the native core owners.

use std::any::Any;
use std::error::Error;
use std::fmt;

pub(super) fn install_panic_hook() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            tracing::error!(
                thread = thread.name().unwrap_or("unnamed"),
                panic = %info,
                "core process panic"
            );
            // Preserve the application's stderr/backtrace or custom panic hook.
            previous(info);
        }));
    });
}

pub(super) fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload")
}

pub(super) fn join(handle: std::thread::JoinHandle<()>) {
    let thread = handle.thread().clone();
    if let Err(payload) = handle.join() {
        tracing::error!(
            thread = thread.name().unwrap_or("unnamed"),
            panic = panic_message(&*payload),
            "core owner join failed"
        );
    }
}

/// Owners [`Core::open`](super::Core::open) has started but not yet handed to
/// the live handle.
///
/// `open` declares this after the world lease, so on any error return (or a
/// panic during startup) the guard drops first: it stops and joins the owners
/// while the lease is still held, and only then is the flock released. Until
/// they have ended the world still has a writer, so a lease released before
/// the join would let a second server lease a world an owner thread is still
/// writing.
pub(super) struct StartupOwners {
    shutdown: Option<std::sync::mpsc::Sender<()>>,
    work: Option<std::sync::mpsc::SyncSender<super::JournalWork>>,
    tick: Option<std::thread::JoinHandle<()>>,
    journal: Option<std::thread::JoinHandle<()>>,
}

impl StartupOwners {
    /// Guard the journal owner alone, before the tick owner's channels exist.
    pub(super) fn new(
        shutdown: std::sync::mpsc::Sender<()>,
        work: std::sync::mpsc::SyncSender<super::JournalWork>,
        journal: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            shutdown: Some(shutdown),
            work: Some(work),
            tick: None,
            journal: Some(journal),
        }
    }

    /// Hand the journal work queue to the tick owner's channels.
    pub(super) fn take_work(&mut self) -> std::sync::mpsc::SyncSender<super::JournalWork> {
        self.work.take().expect("journal work queue ownership")
    }

    /// Take ownership of the tick owner once its thread has started.
    pub(super) fn set_tick(&mut self, tick: std::thread::JoinHandle<()>) {
        self.tick = Some(tick);
    }

    /// The tick owner's thread, for wiring the precommit gateway.
    pub(super) fn tick_thread(&self) -> std::thread::Thread {
        self.tick
            .as_ref()
            .expect("tick owner started")
            .thread()
            .clone()
    }

    /// Hand the started owners to the live handle: every startup step passed.
    pub(super) fn started(mut self) -> StartedOwners {
        self.work.take();
        StartedOwners {
            shutdown: self.shutdown.take().expect("core shutdown sender"),
            tick: self.tick.take().expect("tick owner handle"),
            journal: self.journal.take().expect("journal owner handle"),
        }
    }

    /// Stop both owners and join them in the order the live shutdown uses: the
    /// tick first (its final checkpoint and `Shutdown` item), then the journal
    /// (final WAL flush).
    fn abort(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        // A started tick owner took the queue with it and its `Shutdown` item
        // ends the journal owner. Without one (the tick never started, or its
        // spawn failed and dropped `channels`), closing our sender is what makes
        // the journal owner's `recv` fail so it exits.
        if self.tick.is_none() {
            self.work.take();
        }
        if let Some(tick) = self.tick.take() {
            join(tick);
        }
        self.work.take();
        if let Some(journal) = self.journal.take() {
            join(journal);
        }
    }
}

impl Drop for StartupOwners {
    fn drop(&mut self) {
        // `started` took everything; nothing is running behind this guard.
        if self.tick.is_none() && self.journal.is_none() {
            return;
        }
        self.abort();
    }
}

/// Owners a successful [`super::Core::open`] hands to [`super::CoreInner`].
pub(super) struct StartedOwners {
    pub(super) shutdown: std::sync::mpsc::Sender<()>,
    pub(super) tick: std::thread::JoinHandle<()>,
    pub(super) journal: std::thread::JoinHandle<()>,
}

/// Display the complete causal chain without allocating an intermediate string.
pub(super) struct Chain<'a>(pub(super) &'a (dyn Error + 'static));

impl fmt::Display for Chain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(error) = source {
            write!(f, ": {error}")?;
            source = error.source();
        }
        Ok(())
    }
}
