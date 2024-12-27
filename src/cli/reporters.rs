//! Progress reporters for the CLI

use std::{
    future::Future,
    io::{Stdout, Write},
    sync::{Arc, Mutex, Once, OnceLock},
    time::Duration,
};

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use pesde::reporters::{
    DownloadProgressReporter, DownloadsReporter, PatchProgressReporter, PatchesReporter,
};

pub const TICK_CHARS: &str = "⣷⣯⣟⡿⢿⣻⣽⣾";

pub fn root_progress_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:.dim}{msg:>8.green} {spinner} [{elapsed_precise}]")
        .unwrap()
        .tick_chars(TICK_CHARS)
}

pub fn root_progress_style_with_progress() -> ProgressStyle {
    ProgressStyle::with_template(
        "{prefix:.dim}{msg:>8.green} {spinner} [{elapsed_precise}] {bar:20} {pos}/{len}",
    )
    .unwrap()
    .tick_chars(TICK_CHARS)
}

pub async fn run_with_reporter_and_writer<W, F, R, Fut>(writer: W, f: F) -> R
where
    W: Write + Send + Sync + 'static,
    F: FnOnce(MultiProgress, ProgressBar, Arc<CliReporter<W>>) -> Fut,
    Fut: Future<Output = R>,
{
    let multi_progress = MultiProgress::new();
    crate::PROGRESS_BARS
        .lock()
        .unwrap()
        .replace(multi_progress.clone());

    let root_progress = multi_progress.add(ProgressBar::new(0));
    root_progress.set_style(root_progress_style());
    root_progress.enable_steady_tick(Duration::from_millis(100));

    let reporter = Arc::new(CliReporter::with_writer(
        writer,
        multi_progress.clone(),
        root_progress.clone(),
    ));
    let result = f(multi_progress.clone(), root_progress.clone(), reporter).await;

    root_progress.finish();
    multi_progress.clear().unwrap();
    crate::PROGRESS_BARS.lock().unwrap().take();

    result
}

pub async fn run_with_reporter<F, R, Fut>(f: F) -> R
where
    F: FnOnce(MultiProgress, ProgressBar, Arc<CliReporter<Stdout>>) -> Fut,
    Fut: Future<Output = R>,
{
    run_with_reporter_and_writer(std::io::stdout(), f).await
}

pub struct CliReporter<W = Stdout> {
    writer: Mutex<W>,
    child_style: ProgressStyle,
    child_style_with_bytes: ProgressStyle,
    child_style_with_bytes_without_total: ProgressStyle,
    multi_progress: MultiProgress,
    root_progress: ProgressBar,
}

impl<W> CliReporter<W> {
    pub fn with_writer(
        writer: W,
        multi_progress: MultiProgress,
        root_progress: ProgressBar,
    ) -> Self {
        Self {
            writer: Mutex::new(writer),
            child_style: ProgressStyle::with_template(&"{msg}".dimmed().to_string()).unwrap(),
            child_style_with_bytes: ProgressStyle::with_template(
                &"{msg} {bytes}/{total_bytes}".dimmed().to_string(),
            )
            .unwrap(),
            child_style_with_bytes_without_total: ProgressStyle::with_template(
                &"{msg} {bytes}".dimmed().to_string(),
            )
            .unwrap(),
            multi_progress,
            root_progress,
        }
    }
}

pub struct CliDownloadProgressReporter<'a, W> {
    root_reporter: &'a CliReporter<W>,
    name: String,
    progress: OnceLock<ProgressBar>,
    set_progress: Once,
}

impl<'a, W: Write + Send + Sync + 'static> DownloadsReporter<'a> for CliReporter<W> {
    type DownloadProgressReporter = CliDownloadProgressReporter<'a, W>;

    fn report_download<'b>(&'a self, name: &'b str) -> Self::DownloadProgressReporter {
        self.root_progress.inc_length(1);

        CliDownloadProgressReporter {
            root_reporter: self,
            name: name.to_string(),
            progress: OnceLock::new(),
            set_progress: Once::new(),
        }
    }
}

impl<W: Write + Send + Sync + 'static> DownloadProgressReporter
    for CliDownloadProgressReporter<'_, W>
{
    fn report_start(&self) {
        let progress = self.root_reporter.multi_progress.add(ProgressBar::new(0));
        progress.set_style(self.root_reporter.child_style.clone());
        progress.set_message(format!("- {}", self.name));

        self.progress
            .set(progress)
            .expect("report_start called more than once");
    }

    fn report_progress(&self, total: u64, len: u64) {
        if let Some(progress) = self.progress.get() {
            progress.set_length(total);
            progress.set_position(len);

            self.set_progress.call_once(|| {
                if total > 0 {
                    progress.set_style(self.root_reporter.child_style_with_bytes.clone());
                } else {
                    progress.set_style(
                        self.root_reporter
                            .child_style_with_bytes_without_total
                            .clone(),
                    );
                }
            });
        }
    }

    fn report_done(&self) {
        if let Some(progress) = self.progress.get() {
            if progress.is_hidden() {
                writeln!(
                    self.root_reporter.writer.lock().unwrap(),
                    "downloaded {}",
                    self.name
                )
                .unwrap();
            }

            progress.finish();
            self.root_reporter.multi_progress.remove(progress);
            self.root_reporter.root_progress.inc(1);
        }
    }
}

pub struct CliPatchProgressReporter<'a, W> {
    root_reporter: &'a CliReporter<W>,
    name: String,
    progress: ProgressBar,
}

impl<'a, W: Write + Send + Sync + 'static> PatchesReporter<'a> for CliReporter<W> {
    type PatchProgressReporter = CliPatchProgressReporter<'a, W>;

    fn report_patch<'b>(&'a self, name: &'b str) -> Self::PatchProgressReporter {
        let progress = self.multi_progress.add(ProgressBar::new(0));
        progress.set_style(self.child_style.clone());
        progress.set_message(format!("- {name}"));

        self.root_progress.inc_length(1);

        CliPatchProgressReporter {
            root_reporter: self,
            name: name.to_string(),
            progress,
        }
    }
}

impl<W: Write + Send + Sync + 'static> PatchProgressReporter for CliPatchProgressReporter<'_, W> {
    fn report_done(&self) {
        if self.progress.is_hidden() {
            writeln!(
                self.root_reporter.writer.lock().unwrap(),
                "patched {}",
                self.name
            )
            .unwrap();
        }

        self.progress.finish();
        self.root_reporter.multi_progress.remove(&self.progress);
        self.root_reporter.root_progress.inc(1);
    }
}
