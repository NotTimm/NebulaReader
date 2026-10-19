//! "Convert to CBZ" for CBRs on this computer: a confirmation dialog, one
//! file at a time in the background with a progress line in the library,
//! and the library and reading progress moved over to the new files. The
//! CBRs go to the system Trash, so they can be restored.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cosmic::app::Task;
use cosmic::iced::Alignment;
use cosmic::widget::{self, icon};
use cosmic::Element;

use super::downloads::human_size;
use super::{App, AppView, Message};
use crate::{convert, library};

#[derive(Clone, Debug)]
pub enum ConvMsg {
    /// Ask about converting this series' CBRs.
    Ask(String),
    Start,
    CancelAsk,
    Finished(PathBuf, Result<PathBuf, String>),
    Stop,
    Tick,
    DismissNote,
}

/// What the dialog offers to convert.
pub struct ConvertPlan {
    name: String,
    files: Vec<PathBuf>,
    bytes: u64,
}

pub struct Converting {
    queue: VecDeque<PathBuf>,
    total: usize,
    done: usize,
    bytes: u64,
    written: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    trashed: usize,
    errors: Vec<String>,
}

impl App {
    pub(super) fn update_convert(&mut self, msg: ConvMsg) -> Task<Message> {
        match msg {
            ConvMsg::Ask(series) => {
                if self.converting.is_some() {
                    self.convert_note = Some("A conversion is already running.".into());
                    return Task::none();
                }
                let reading = self.app_view == AppView::Reader
                    && self.open_path.as_ref().is_some_and(|p| p.to_string_lossy() == series);
                if reading {
                    self.convert_note = Some("Close this comic before converting it.".into());
                    return Task::none();
                }
                let files: Vec<PathBuf> = self
                    .series_issues(&series)
                    .into_iter()
                    .map(|i| PathBuf::from(i.path))
                    .filter(|p| convert::is_rar(p) && p.is_file())
                    .collect();
                if files.is_empty() {
                    self.convert_note = Some("There are no CBR files here to convert.".into());
                    return Task::none();
                }
                let bytes = files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
                let name = self
                    .db
                    .as_ref()
                    .and_then(|db| library::get_series(db, &series))
                    .map(|e| e.name().to_string())
                    .unwrap_or_default();
                self.convert_ask = Some(ConvertPlan { name, files, bytes });
            }
            ConvMsg::CancelAsk => self.convert_ask = None,
            ConvMsg::Start => {
                let Some(plan) = self.convert_ask.take() else { return Task::none() };
                self.convert_note = None;
                self.converting = Some(Converting {
                    total: plan.files.len(),
                    queue: plan.files.into(),
                    done: 0,
                    bytes: plan.bytes.max(1),
                    written: Arc::new(AtomicU64::new(0)),
                    cancel: Arc::new(AtomicBool::new(false)),
                    trashed: 0,
                    errors: Vec::new(),
                });
                return self.pump_convert();
            }
            ConvMsg::Finished(old, result) => {
                let Some(c) = self.converting.as_mut() else { return Task::none() };
                c.done += 1;
                match result {
                    Ok(new) => {
                        let (old_key, new_key) = (old.to_string_lossy().into_owned(), new.to_string_lossy().into_owned());
                        if let Some(db) = &self.db {
                            if let Err(e) = library::rename_issue_file(db, &old_key, &new_key) {
                                log::warn!("convert: {e}");
                            }
                        }
                        crate::comic::forget_rar_cache(&old);
                        match trash::delete(&old) {
                            Ok(()) => c.trashed += 1,
                            Err(e) => c.errors.push(format!(
                                "{} was converted, but couldn't go to the Trash ({e}); delete it yourself",
                                old.display()
                            )),
                        }
                        // A one-file series is now keyed by the new file.
                        if self.series_page.as_ref().is_some_and(|s| s.path == old_key) {
                            self.series_page = None;
                            let page = self.open_series_page(new_key);
                            return Task::batch([page, self.pump_convert()]);
                        }
                    }
                    Err(e) if e == "cancelled" => {}
                    Err(e) => c.errors.push(format!("{}: {e}", old.display())),
                }
                return self.pump_convert();
            }
            ConvMsg::Stop => {
                if let Some(c) = self.converting.as_mut() {
                    c.queue.clear();
                    c.cancel.store(true, Ordering::Relaxed);
                }
            }
            ConvMsg::Tick => {}
            ConvMsg::DismissNote => self.convert_note = None,
        }
        Task::none()
    }

    /// Starts the next file, or wraps up when there's none left.
    fn pump_convert(&mut self) -> Task<Message> {
        let Some(c) = self.converting.as_mut() else { return Task::none() };
        let Some(file) = c.queue.pop_front() else {
            let c = self.converting.take().unwrap();
            let mut note = match c.trashed {
                0 => String::new(),
                1 => "Converted 1 file to CBZ; the CBR is in the Trash.".to_string(),
                n => format!("Converted {n} files to CBZ; the CBRs are in the Trash."),
            };
            if let Some(first) = c.errors.first() {
                note = format!("{note} {} didn't convert: {first}", c.errors.len()).trim().to_string();
            }
            self.convert_note = (!note.is_empty()).then_some(note);
            if let Some(db) = &self.db {
                self.rar_series = library::series_with_rar(db);
            }
            return self.refresh_views();
        };
        let (cancel, written) = (c.cancel.clone(), c.written.clone());
        cosmic::task::future(async move {
            let source = file.clone();
            let result = tokio::task::spawn_blocking(move || convert::to_cbz(&source, &cancel, &written))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            Message::Convert(ConvMsg::Finished(file, result))
        })
    }

    /// Redraws the progress line while converting.
    pub(super) fn convert_subscription(&self) -> cosmic::iced::Subscription<Message> {
        if self.converting.is_some() {
            cosmic::iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::Convert(ConvMsg::Tick))
        } else {
            cosmic::iced::Subscription::none()
        }
    }

    /// The library's line for a running conversion, or how one went.
    pub(super) fn convert_status(&self) -> Option<Element<'_, Message>> {
        if let Some(c) = &self.converting {
            let fraction = (c.written.load(Ordering::Relaxed) as f32 / c.bytes as f32).min(1.0);
            return Some(
                widget::row::with_capacity(3)
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(widget::determinate_circular(fraction).size(14.0).bar_height(2.0))
                    .push(widget::text::caption(format!(
                        "Converting {} of {} to CBZ… {:.0}%",
                        (c.done + 1).min(c.total),
                        c.total,
                        fraction * 100.0
                    )))
                    .push(widget::button::link("Stop").on_press(Message::Convert(ConvMsg::Stop)))
                    .into(),
            );
        }
        let note = self.convert_note.as_deref()?;
        Some(
            widget::row::with_capacity(3)
                .spacing(8)
                .align_y(Alignment::Center)
                .push(icon::from_name("emblem-ok-symbolic").size(16))
                .push(widget::text::caption(note.to_string()))
                .push(
                    widget::button::icon(icon::from_name("window-close-symbolic"))
                        .on_press(Message::Convert(ConvMsg::DismissNote)),
                )
                .into(),
        )
    }

    /// The "Convert to CBZ?" dialog.
    pub(super) fn convert_dialog(&self) -> Option<Element<'_, Message>> {
        let plan = self.convert_ask.as_ref()?;
        let count = plan.files.len();
        let what = if count == 1 { "this CBR".to_string() } else { format!("these {count} CBR files") };
        let body = format!(
            "Repacks {what} ({}) as CBZ, page for page with nothing recompressed, so they open faster and stream from Jellyfin. \
             Reading progress carries over. Each new file is written in full and checked before the original goes to the Trash, \
             where you can restore it. Until the Trash is emptied that takes about {} more on that drive.",
            human_size(plan.bytes),
            human_size(plan.bytes),
        );
        Some(
            widget::dialog()
                .title(format!("Convert “{}” to CBZ?", plan.name))
                .icon(icon::from_name("package-x-generic-symbolic").size(64))
                .body(body)
                .primary_action(widget::button::suggested("Convert").on_press(Message::Convert(ConvMsg::Start)))
                .secondary_action(widget::button::standard("Cancel").on_press(Message::Convert(ConvMsg::CancelAsk)))
                .into(),
        )
    }
}
