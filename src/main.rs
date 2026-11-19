//! The Local History plugin of SimpleEdit: what files were, kept on this
//! machine whatever Git knows, with labels on moments that matter. Its panel
//! lists the revisions of a file or a folder; one is compared with now, put
//! back, or saved as a patch.

mod commits;
mod store;
mod text;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use simpleedit_plugin::protocol::{Context, InitializeParams, Settings as EditorSettings, UiEvent};
use simpleedit_plugin::ui::*;
use simpleedit_plugin::{run, Host, Plugin};

use store::{Row, Store};
use text::t;

const PANEL: &str = "history";
const SETTINGS: &str = "settings";
/// How often the text being typed is kept, saved or not.
const TICK: Duration = Duration::from_secs(60);
/// How often the logs of the repositories are looked at for new commits.
const COMMIT_TICK: Duration = Duration::from_secs(3);
/// The choices of how long the history goes back, in working days.
const DAYS: [u32; 7] = [0, 1, 5, 14, 30, 90, 365];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Settings {
    /// Working days kept (0: nothing is kept).
    days: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { days: 5 }
    }
}

struct History {
    store: Store,
    data_dir: PathBuf,
    /// Shared with the thread that keeps the text being typed.
    days: Arc<AtomicU32>,
    french: bool,
    /// The file in front.
    active: Option<PathBuf>,
    /// What the panel shows the history of.
    scope: Option<PathBuf>,
    rows: Vec<Row>,
    /// The rows a search kept (none: all).
    found: Option<Vec<usize>>,
    selected: Option<usize>,
    /// The changes between the revision picked and the file now.
    changes: Vec<store::Chunk>,
    query: String,
    label: String,
}

fn now() -> i64 {
    store::now()
}

fn format_time(epoch: i64, french: bool) -> String {
    use chrono::TimeZone;
    let format = if french {
        "%d/%m/%Y %H:%M"
    } else {
        "%Y-%m-%d %H:%M"
    };
    chrono::Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|t| t.format(format).to_string())
        .unwrap_or_default()
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl History {
    fn new() -> Self {
        History {
            store: Store::at(PathBuf::new()),
            data_dir: PathBuf::new(),
            days: Arc::new(AtomicU32::new(5)),
            french: false,
            active: None,
            scope: None,
            rows: Vec::new(),
            found: None,
            selected: None,
            changes: Vec::new(),
            query: String::new(),
            label: String::new(),
        }
    }

    fn on(&self) -> bool {
        self.days.load(Ordering::Relaxed) > 0
    }

    fn settings(&self) -> Settings {
        Settings {
            days: self.days.load(Ordering::Relaxed),
        }
    }

    fn save_settings(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.settings()).map_err(std::io::Error::other)?;
        std::fs::write(self.data_dir.join("settings.json"), json)
    }

    fn record_disk(&self, path: &Path) {
        if self.on() {
            let _ = self.store.record_file(path, now());
        }
    }

    fn record_text(&self, path: &Path, text: &str) {
        if self.on() {
            let _ = self.store.record(path, text, now());
        }
    }

    /// The history of `scope` (a file or a folder), shown in the panel.
    fn show(&mut self, scope: PathBuf) {
        self.rows = self.store.rows(&scope);
        self.scope = Some(scope);
        self.found = None;
        self.selected = None;
        self.changes.clear();
        self.query.clear();
    }

    fn reload(&mut self) {
        if let Some(scope) = self.scope.clone() {
            let selected = self.selected;
            self.rows = self.store.rows(&scope);
            self.selected = selected.filter(|i| *i < self.rows.len());
        }
    }

    /// The rows whose text has the query (labels are kept).
    fn search(&mut self) {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            self.found = None;
            return;
        }
        self.found = Some(
            self.rows
                .iter()
                .enumerate()
                .filter(|(_, row)| match row {
                    Row::Revision { blob, .. } => self
                        .store
                        .text(blob)
                        .is_some_and(|text| text.to_lowercase().contains(&query)),
                    Row::Label(_) => true,
                })
                .map(|(i, _)| i)
                .collect(),
        );
    }

    /// What a file holds now: the editor's text for the file in front, the
    /// disk's otherwise.
    fn current_text(&self, host: &Host, path: &Path) -> String {
        if self.active.as_deref() == Some(path) {
            if let Ok(active) = host.active_file() {
                if active.path.as_deref() == Some(path) {
                    return active.text;
                }
            }
        }
        std::fs::read_to_string(path).unwrap_or_default()
    }

    /// Puts a file back to `text`: in the editor for the file in front (the
    /// user can undo it), on the disk otherwise.
    fn put_back(&self, host: &Host, path: &Path, text: &str) -> Result<(), String> {
        self.record_text(path, &self.current_text(host, path));
        if self.active.as_deref() == Some(path) && host.replace_text(path, text).is_ok() {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected?)
    }

    /// Picks a row, and finds what changed since for a revision.
    fn select(&mut self, host: &Host, index: Option<usize>) {
        self.selected = index.filter(|i| *i < self.rows.len());
        self.changes = match self.selected_row() {
            Some(Row::Revision { path, blob, .. }) => match self.store.text(blob) {
                Some(old) => store::chunks(&old, &self.current_text(host, path)),
                None => Vec::new(),
            },
            _ => Vec::new(),
        };
    }

    /// Puts one change back as the revision had it.
    fn revert_change(&mut self, host: &Host, index: usize) {
        let (Some(Row::Revision { path, blob, .. }), Some(chunk)) = (
            self.selected_row().cloned(),
            self.changes.get(index).cloned(),
        ) else {
            return;
        };
        let Some(old) = self.store.text(&blob) else {
            return;
        };
        let now = self.current_text(host, &path);
        let back = store::revert_chunk(&old, &now, &chunk);
        match self.put_back(host, &path, &back) {
            Ok(()) => host.notify(t(self.french, "reverted")),
            Err(e) => host.error(e),
        }
        let selected = self.selected;
        self.reload();
        self.select(host, selected);
    }

    fn compare(&self, host: &Host) {
        let Some(Row::Revision { path, time, blob }) = self.selected_row() else {
            return;
        };
        let Some(old) = self.store.text(blob) else {
            host.error(t(self.french, "gone"));
            return;
        };
        let new = self.current_text(host, path);
        let left = format!("{} ({})", name_of(path), format_time(*time, self.french));
        let right = format!("{} ({})", name_of(path), t(self.french, "now"));
        if host.open_diff((left, old), (right, new)).is_err() {
            host.error(t(self.french, "same"));
        }
    }

    fn revert(&mut self, host: &Host) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        if !host.confirm(t(self.french, "revert_ask")).unwrap_or(false) {
            return;
        }
        let files: Vec<(PathBuf, String)> = match (&row, &self.scope) {
            (Row::Revision { path, blob, .. }, _) => match self.store.text(blob) {
                Some(text) => vec![(path.clone(), text)],
                None => Vec::new(),
            },
            (Row::Label(label), Some(scope)) => self.store.state_at(scope, label.time),
            (Row::Label(_), None) => Vec::new(),
        };
        let mut failed = Vec::new();
        for (path, text) in &files {
            if let Err(e) = self.put_back(host, path, text) {
                failed.push(format!("{}: {e}", path.display()));
            }
        }
        match failed.first() {
            Some(e) => host.error(e.clone()),
            None => host.notify(t(self.french, "reverted")),
        }
        self.reload();
    }

    fn patch(&self, host: &Host) {
        let Some(Row::Revision { path, blob, .. }) = self.selected_row() else {
            return;
        };
        let Some(old) = self.store.text(blob) else {
            return;
        };
        let name = name_of(path);
        let text = store::patch(&name, &old, &self.current_text(host, path));
        if let Err(e) = host.save_file(format!("{name}.patch"), text) {
            host.error(e.message);
        }
    }

    fn put_label(&mut self, host: &Host) {
        let name = self.label.trim().to_string();
        if name.is_empty() || !self.on() {
            return;
        }
        // What is in front is kept first, as it is now.
        if let Ok(active) = host.active_file() {
            if let Some(path) = active.path {
                self.record_text(&path, &active.text);
            }
        }
        let _ = self.store.put_label(&name, false, now());
        self.label.clear();
        self.reload();
        host.notify(t(self.french, "labelled"));
    }

    fn view(&self) -> Node {
        let f = self.french;
        let Some(scope) = &self.scope else {
            return text(t(f, "pick")).style(TextStyle::Muted);
        };
        let shown: Vec<usize> = match &self.found {
            Some(found) => found.clone(),
            None => (0..self.rows.len()).collect(),
        };
        let items = shown.iter().filter_map(|i| {
            let row = self.rows.get(*i)?;
            Some(match row {
                Row::Revision { path, time, .. } => {
                    let mut it = item(i.to_string(), format_time(*time, f)).icon("\u{f017}");
                    if scope.is_dir() {
                        it = it.detail(
                            path.strip_prefix(scope)
                                .unwrap_or(path)
                                .display()
                                .to_string(),
                        );
                    }
                    it
                }
                Row::Label(label) => item(i.to_string(), label.name.clone())
                    .icon("\u{f02b}")
                    .detail(format_time(label.time, f)),
            })
        });
        let mut list_node = list("rows", items);
        if let Some(selected) = self.selected {
            list_node = list_node.selected(selected.to_string());
        }
        let is_revision = matches!(self.selected_row(), Some(Row::Revision { .. }));
        let picked = self.selected.is_some();
        let mut body = vec![row([
            text(format!("{} — {}", t(f, "title"), name_of(scope))).style(TextStyle::Bold),
            input("query", &self.query, t(f, "search")).on_submit("search"),
        ])
        .spacing(12.0)];
        if !self.on() {
            body.push(text(t(f, "off")).style(TextStyle::Muted));
        }
        if self.rows.is_empty() {
            body.push(text(t(f, "empty")).style(TextStyle::Muted));
        }
        body.push(
            row([
                button(t(f, "compare"), "compare").enabled(is_revision),
                button(t(f, "revert"), "revert")
                    .button_style(ButtonStyle::Danger)
                    .enabled(picked),
                button(t(f, "patch"), "patch").enabled(is_revision),
                input("label", &self.label, t(f, "label_hint")).on_submit("put_label"),
                button(t(f, "label"), "put_label").enabled(self.on()),
            ])
            .spacing(8.0),
        );
        let mut lower = vec![list_node];
        for (i, chunk) in self.changes.iter().enumerate() {
            let shown: Vec<String> = chunk
                .lines
                .iter()
                .filter(|(kind, _)| *kind != store::LineKind::Same)
                .take(12)
                .map(|(kind, line)| {
                    let mark = if *kind == store::LineKind::Old {
                        '-'
                    } else {
                        '+'
                    };
                    format!("{mark} {line}")
                })
                .collect();
            lower.push(separator());
            lower.push(
                row([
                    text(format!("{} {}", t(f, "change"), chunk.new.start + 1))
                        .style(TextStyle::Muted),
                    button(t(f, "revert_change"), format!("chunk:{i}"))
                        .button_style(ButtonStyle::Ghost),
                ])
                .spacing(8.0),
            );
            lower.push(text(shown.join("\n")).style(TextStyle::Mono));
        }
        body.push(scroll(column(lower).spacing(6.0)));
        column(body).spacing(8.0)
    }

    fn settings_view(&self) -> Node {
        let f = self.french;
        let options: Vec<String> = DAYS.iter().map(|d| days_label(*d, f)).collect();
        column([
            text(t(f, "days")),
            select(
                "days",
                options,
                Some(days_label(self.days.load(Ordering::Relaxed), f)),
            ),
        ])
        .spacing(8.0)
    }

    fn render(&self, host: &Host) {
        host.render(PANEL, self.view());
    }
}

fn days_label(days: u32, french: bool) -> String {
    match (days, french) {
        (0, true) => "Désactivé".into(),
        (0, false) => "Off".into(),
        (1, true) => "1 jour de travail".into(),
        (1, false) => "1 working day".into(),
        (n, true) => format!("{n} jours de travail"),
        (n, false) => format!("{n} working days"),
    }
}

impl Plugin for History {
    fn initialize(&mut self, host: &Host, params: &InitializeParams) {
        self.data_dir = params.data_dir.clone();
        self.store = Store::at(self.data_dir.join("local-history"));
        self.french = params.settings.locale.starts_with("fr");
        let settings: Settings = std::fs::read_to_string(self.data_dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        self.days.store(settings.days, Ordering::Relaxed);
        // Each commit made in the workspace puts a label, at its time.
        let mut watch = commits::Watch::new(&params.workspace);
        let (store, days, french) = (self.store.clone(), self.days.clone(), self.french);
        std::thread::spawn(move || loop {
            std::thread::sleep(COMMIT_TICK);
            let times = watch.new_commits();
            if days.load(Ordering::Relaxed) == 0 {
                continue;
            }
            for time in times {
                let _ = store.put_label(t(french, "auto_commit"), true, time);
            }
        });
        // What is too old goes, once, then the text typed is kept now and then.
        let (store, days, host) = (self.store.clone(), self.days.clone(), host.clone());
        std::thread::spawn(move || {
            let kept = days.load(Ordering::Relaxed);
            if kept > 0 {
                store.prune(kept, store::MAX_BYTES);
            }
            loop {
                std::thread::sleep(TICK);
                if days.load(Ordering::Relaxed) == 0 {
                    continue;
                }
                match host.active_file() {
                    Ok(active) => {
                        if let Some(path) = active.path {
                            let _ = store.record(&path, &active.text, now());
                        }
                    }
                    // The editor is gone.
                    Err(_) => break,
                }
            }
        });
    }

    fn command(&mut self, host: &Host, command: &str, context: &Context) {
        let scope = context.file.clone().or_else(|| self.active.clone());
        match command {
            "history.show" => {
                let Some(scope) = scope else {
                    host.notify(t(self.french, "pick"));
                    return;
                };
                self.show(scope);
                host.show_panel(PANEL);
                self.render(host);
            }
            "history.label" => {
                if self.scope.is_none() {
                    if let Some(scope) = scope {
                        self.show(scope);
                    }
                }
                host.show_panel(PANEL);
                self.render(host);
            }
            _ => {}
        }
    }

    fn panel_opened(&mut self, host: &Host, panel: &str) {
        match panel {
            PANEL => {
                if self.scope.is_none() {
                    if let Some(active) = self.active.clone() {
                        self.show(active);
                    }
                } else {
                    self.reload();
                }
                self.render(host);
            }
            SETTINGS => host.render(SETTINGS, self.settings_view()),
            _ => {}
        }
    }

    fn ui_event(&mut self, host: &Host, event: &UiEvent) {
        if event.panel == SETTINGS {
            if event.action == "days" {
                let picked = event.text().unwrap_or_default();
                if let Some(days) = DAYS.iter().find(|d| days_label(**d, self.french) == picked) {
                    self.days.store(*days, Ordering::Relaxed);
                    if self.save_settings().is_err() {
                        host.error(t(self.french, "not_saved"));
                    }
                }
                host.render(SETTINGS, self.settings_view());
            }
            return;
        }
        match event.action.as_str() {
            "rows" => {
                let index = event.text().and_then(|s| s.parse().ok());
                self.select(host, index);
            }
            "query" => {
                self.query = event.text().unwrap_or_default().to_string();
                if self.query.is_empty() {
                    self.found = None;
                } else {
                    return;
                }
            }
            "search" => self.search(),
            "label" => {
                self.label = event.text().unwrap_or_default().to_string();
                return;
            }
            "put_label" => self.put_label(host),
            "compare" => self.compare(host),
            "revert" => self.revert(host),
            "patch" => self.patch(host),
            action => match action.strip_prefix("chunk:").and_then(|i| i.parse().ok()) {
                Some(i) => self.revert_change(host, i),
                None => return,
            },
        }
        self.render(host);
    }

    fn file_opened(&mut self, _: &Host, path: &Path) {
        self.record_disk(path);
    }

    fn file_saved_from(&mut self, host: &Host, path: &Path, previous: Option<&str>) {
        if let Some(previous) = previous {
            self.record_text(path, previous);
        }
        self.record_disk(path);
        if self.scope.as_deref().is_some_and(|s| path.starts_with(s)) {
            self.reload();
            self.render(host);
        }
    }

    fn file_deleted(&mut self, _: &Host, path: &Path, text: Option<&str>) {
        if let Some(text) = text {
            self.record_text(path, text);
        }
    }

    fn file_renamed(&mut self, _: &Host, from: &Path, to: &Path) {
        if self.on() {
            self.store.rename_tree(from, to);
        }
    }

    fn active_file_changed(&mut self, _: &Host, path: Option<&Path>) {
        self.active = path.map(Path::to_path_buf);
        if let Some(path) = path {
            self.record_disk(path);
        }
    }

    fn settings_changed(&mut self, host: &Host, settings: &EditorSettings) {
        self.french = settings.locale.starts_with("fr");
        self.render(host);
        host.render(SETTINGS, self.settings_view());
    }
}

fn main() -> std::io::Result<()> {
    run(History::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> (History, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::new();
        h.data_dir = dir.path().to_path_buf();
        h.store = Store::at(dir.path().join("local-history"));
        (h, dir)
    }

    #[test]
    fn what_a_file_was_is_kept_and_listed_latest_first() {
        let (h, dir) = history();
        let file = dir.path().join("a.txt");
        h.record_text(&file, "one");
        std::thread::sleep(Duration::from_millis(1100));
        h.record_text(&file, "two");
        let mut h = h;
        h.show(file.clone());
        assert_eq!(h.rows.len(), 2);
        let Row::Revision { blob, .. } = &h.rows[0] else {
            panic!("a revision");
        };
        assert_eq!(h.store.text(blob).as_deref(), Some("two"));
        // A search keeps the revisions that have the text.
        h.query = "ONE".into();
        h.search();
        assert_eq!(h.found.as_ref().unwrap().len(), 1);
        let page = serde_json::to_string(&h.view()).unwrap();
        assert!(page.contains("\"rows\"") && page.contains("a.txt"));
    }

    #[test]
    fn nothing_is_kept_when_it_is_off() {
        let (h, dir) = history();
        h.days.store(0, Ordering::Relaxed);
        let file = dir.path().join("a.txt");
        h.record_text(&file, "one");
        assert!(h.store.rows(&file).is_empty());
        let page = serde_json::to_string(&h.settings_view()).unwrap();
        assert!(page.contains("Off"));
    }

    #[test]
    fn the_days_kept_are_saved() {
        let (h, dir) = history();
        h.days.store(30, Ordering::Relaxed);
        h.save_settings().unwrap();
        let read: Settings = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(read.days, 30);
        assert_eq!(days_label(30, true), "30 jours de travail");
    }
}
