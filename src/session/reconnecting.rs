//! A Session while Disconnected: noticing the drop, retrying on the Reconnect attempts
//! setting, the reconnect's own prompts, and the strip above the listing. It is
//! `Session`'s own code, kept apart from browsing, Search and Download.

use super::*;

impl Session {
    /// Whether the connection has dropped and not come back yet.
    pub fn is_disconnected(&self) -> bool {
        self.lost.is_some()
    }

    /// Why the connection dropped, while Disconnected.
    pub fn lost_reason(&self) -> Option<&str> {
        self.lost.as_ref().map(Lost::reason)
    }

    /// Notices a drop, and runs the reconnect attempts as `attempts` allows. Call it
    /// every frame, on screen or not; `now` is egui's time. Gives the seconds until the
    /// strip's countdown next changes, for a repaint.
    pub fn keep_connected(&mut self, now: f64, attempts: ReconnectAttempts) -> Option<f64> {
        if self.lost.is_none() {
            let reason = self.connection.lost()?;
            self.lost = Some(Lost::new(reason, attempts, now));
            self.info_request = None;
            self.host_info = Default::default();
            self.search_lost();
        }
        if let Some(opening) = self.reconnecting.take() {
            match opening.try_next() {
                None => self.reconnecting = Some(opening),
                Some(ssh::Event::Opened(opened)) => self.reconnected(opened),
                Some(ssh::Event::Failed(ssh::Failure::Connection(_) | ssh::Failure::Sftp(_))) => {
                    if let Some(lost) = &mut self.lost {
                        lost.failed(attempts, now);
                    }
                }
                Some(ssh::Event::Failed(ssh::Failure::Cancelled)) => self.stop_reconnecting(),
                // A prompt, a changed key or a refused login: the operator takes over.
                Some(event) => {
                    self.stop_reconnecting();
                    let id = self.id.with("reconnect");
                    let details = self.details.clone();
                    self.handover = Some(Pending::resume(id, details, opening, event));
                }
            }
        }
        let lost = self.lost.as_mut()?;
        if lost.due(now) {
            self.reconnecting = Some(self.connection.reconnect());
        }
        let wait = self.lost.as_ref()?.wait(now)?;
        // The countdown shows whole seconds.
        Some(wait.min(wait.fract().max(0.05)))
    }

    /// Reconnect now, Reconnect, or F5 while Disconnected: an attempt starts at once,
    /// unless one is running or the operator is being asked.
    pub fn reconnect_now(&mut self) {
        if self.reconnecting.is_some() || self.handover.is_some() {
            return;
        }
        if let Some(lost) = &mut self.lost {
            lost.now();
            self.reconnecting = Some(self.connection.reconnect());
        }
    }

    /// Stop: no more attempts until the operator asks.
    pub fn stop_reconnecting(&mut self) {
        // Dropping the attempt cancels whatever it would ask.
        self.reconnecting = None;
        if let Some(lost) = &mut self.lost {
            lost.stop();
        }
    }

    /// The connection is back: the strip clears, the Current directory is refreshed and
    /// the fetches the drop cut short are made again.
    pub(super) fn reconnected(&mut self, opened: ssh::Opened) {
        self.info_request = Some(opened.session.host_info());
        self.connection = opened.session;
        self.lost = None;
        self.reconnecting = None;
        self.handover = None;
        // A Download the drop paused fetches its file again.
        self.downloads.resume(&self.connection);
        let refetch = std::mem::take(&mut self.refetch);
        self.fetch(refetch);
        self.refresh();
    }

    /// Draws the reconnect's prompt, when one needs the operator. Call it every frame.
    pub fn prompt(&mut self, ctx: &egui::Context) {
        let Some(pending) = &mut self.handover else {
            return;
        };
        match pending.show(ctx) {
            Some(Outcome::Opened(opened)) => self.reconnected(opened),
            Some(Outcome::Ended) => self.handover = None,
            None => {}
        }
    }

    /// Whether the reconnect's prompt is up, which takes the input from the window.
    pub fn is_asking(&self) -> bool {
        self.handover.as_ref().is_some_and(Pending::is_modal)
    }

    /// The strip above the listing while Disconnected: "Connection lost: <reason>", how
    /// reconnecting goes, and Reconnect now and Stop, or Reconnect once stopped.
    pub(super) fn lost_strip(&mut self, ui: &mut egui::Ui, now: f64, attempts: ReconnectAttempts) {
        let Some(lost) = &self.lost else {
            return;
        };
        let headline = lost.headline();
        let status = lost.status(now, attempts);
        let (mut now_clicked, mut stop) = (false, false);
        ui.horizontal(|ui| {
            ui.add(widgets::warning_message(&headline));
            // The buttons come before the status, whose width changes as it counts down,
            // so they stay where they are.
            match &status {
                Some(status) => {
                    now_clicked = ui.add(widgets::button("Reconnect now").flat()).clicked();
                    stop = ui.add(widgets::button("Stop").flat()).clicked();
                    ui.add(widgets::muted(status.as_str()));
                }
                None => {
                    now_clicked = ui
                        .add(widgets::button("Reconnect").flat())
                        .tooltip("Connect again (F5)")
                        .clicked();
                }
            }
        });
        if now_clicked {
            self.reconnect_now();
        }
        if stop {
            self.stop_reconnecting();
        }
    }
}
