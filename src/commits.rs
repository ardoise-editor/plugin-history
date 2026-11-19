//! The commits made in the folders of the workspace, whoever made them (the
//! Git plugin, a terminal): each one becomes a label of the history, as the
//! editor put one before its tools were plugins. Git writes a line in
//! `.git/logs/HEAD` for each move of `HEAD`; the new ones are read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where each repository's log of `HEAD` was read up to.
pub struct Watch {
    seen: HashMap<PathBuf, u64>,
}

fn log_of(root: &Path) -> PathBuf {
    root.join(".git").join("logs").join("HEAD")
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

impl Watch {
    /// Starts at the end of each log: the commits made before are not new.
    pub fn new(roots: &[PathBuf]) -> Watch {
        Watch {
            seen: roots
                .iter()
                .map(|root| {
                    let log = log_of(root);
                    let at = size(&log);
                    (log, at)
                })
                .collect(),
        }
    }

    /// The times (seconds since 1970) of the commits made since the last look.
    pub fn new_commits(&mut self) -> Vec<i64> {
        let mut times = Vec::new();
        for (log, at) in self.seen.iter_mut() {
            let now = size(log);
            // Cut short (the log expired): read on from its end.
            if now < *at {
                *at = now;
                continue;
            }
            if now == *at {
                continue;
            }
            let Ok(bytes) = std::fs::read(log.as_path()) else {
                continue;
            };
            let start = (*at as usize).min(bytes.len());
            // Only whole lines: a line being written is read next time.
            let Some(end) = bytes[start..].iter().rposition(|b| *b == b'\n') else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes[start..start + end + 1]);
            times.extend(text.lines().filter_map(commit_time));
            *at = (start + end + 1) as u64;
        }
        times.sort_unstable();
        times
    }
}

/// The time of a line of the log of `HEAD` that is a commit (`commit:`,
/// `commit (amend):`, `commit (initial):`, `commit (merge):`).
pub fn commit_time(line: &str) -> Option<i64> {
    let (head, message) = line.split_once('\t')?;
    if !message.starts_with("commit") {
        return None;
    }
    // `<old> <new> <name> <email> <time> <zone>`
    let mut words = head.rsplit(' ');
    let _zone = words.next()?;
    words.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_a_commit_gives_its_time_and_the_other_moves_none() {
        let commit =
            "0000 1111 Ada Lovelace <ada@example.com> 1700000000 +0100\tcommit (initial): first";
        assert_eq!(commit_time(commit), Some(1_700_000_000));
        let amend = "1111 2222 Ada <ada@example.com> 1700000100 +0000\tcommit (amend): fixed";
        assert_eq!(commit_time(amend), Some(1_700_000_100));
        let checkout =
            "2222 3333 Ada <ada@example.com> 1700000200 +0000\tcheckout: moving from main to dev";
        assert_eq!(commit_time(checkout), None);
        assert_eq!(commit_time("nonsense"), None);
    }

    #[test]
    fn only_the_commits_made_since_the_last_look_are_new() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let logs = root.join(".git").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let log = logs.join("HEAD");
        std::fs::write(&log, "0 1 A <a@b> 100 +0000\tcommit (initial): old\n").unwrap();
        let mut watch = Watch::new(std::slice::from_ref(&root));
        assert!(watch.new_commits().is_empty(), "made before");
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        write!(
            file,
            "1 2 A <a@b> 200 +0000\tcommit: new\n2 3 A <a@b> 300 +0000\tcheckout: x\n3 4 A <a@b> 400 +0000\tcommit: hal"
        )
        .unwrap();
        // The last line is not whole yet.
        assert_eq!(watch.new_commits(), vec![200]);
        writeln!(file, "f").unwrap();
        assert_eq!(watch.new_commits(), vec![400]);
        assert!(watch.new_commits().is_empty());
        // Expired: nothing is taken for new.
        std::fs::write(&log, "").unwrap();
        assert!(watch.new_commits().is_empty());
        // No repository at all: nothing.
        let mut none = Watch::new(&[dir.path().join("nowhere")]);
        assert!(none.new_commits().is_empty());
    }
}
