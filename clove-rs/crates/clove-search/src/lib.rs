//! Per-keystroke filtering for Clove.
//!
//! The Swift app filters the in-memory listing on every keystroke instead of maintaining
//! an index; this crate keeps that decision. Names arrive already case-folded from
//! [`clove_fs::Entry`], which the prototype benchmarked at 5–8× faster than folding inside
//! the filter (`docs/rust-port/04-prototypes-and-benchmarks.md`).
//!
//! [`FilterWorker`] runs filters on a worker thread and drops stale results: typing faster
//! than the worker can filter only ever yields the newest query's result.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clove_fs::{Entry, Snapshot};

/// Scores one folded name against one folded query. `None` when it is not a subsequence.
///
/// Bonuses: 10 per matched character, 25 for each consecutive pair (so a run of adjacent
/// characters beats a scattered boundary match), 20 for matching at a word boundary
/// (start, or after space/`_-.`/slash), 25 more for a match at position 0.
pub fn score_name(name_folded: &str, query_folded: &str) -> Option<u32> {
    let query: Vec<char> = query_folded.chars().collect();
    score_chars(name_folded, &query)
}

/// [`score_name`] with the query already split into chars — the form the hot loop uses, so
/// a 100k-entry filter allocates nothing per entry.
fn score_chars(name_folded: &str, query: &[char]) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut score = 0u32;
    let mut matched = 0usize;
    let mut prev_end: Option<usize> = None;
    let mut prev_char: Option<char> = None;

    for (byte, ch) in name_folded.char_indices() {
        if matched == query.len() {
            break;
        }
        if ch == query[matched] {
            score += 10;
            if prev_end == Some(byte) {
                score += 25;
            }
            let at_boundary = match prev_char {
                None => true,
                Some(p) => matches!(p, ' ' | '_' | '-' | '.' | '/' | '\\'),
            };
            if at_boundary {
                score += 20;
            }
            if byte == 0 {
                score += 25;
            }
            prev_end = Some(byte + ch.len_utf8());
            matched += 1;
        }
        prev_char = Some(ch);
    }

    (matched == query.len()).then_some(score)
}

/// Filters `entries`, returning indices ordered by score (desc).
///
/// Ties keep input order, which is listing order — `Snapshot` is already sorted by name, so
/// this is "best match, then name" without comparing names inside the comparator. That
/// matters: the comparator runs ~1.7M times for a 100k-entry listing.
pub fn filter(entries: &[Entry], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let query: Vec<char> = query.chars().collect();

    let mut scored: Vec<(u32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            score_chars(&e.name_folded, &query).map(|s| (s + if e.is_dir { 5 } else { 0 }, i))
        })
        .collect();

    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// The newest filter result; `generation` matches the request that produced it.
#[derive(Debug)]
pub struct FilterResult {
    pub generation: u64,
    pub indices: Vec<usize>,
    pub took: Duration,
}

struct Job {
    generation: u64,
    snapshot: Arc<Snapshot>,
    query: String,
}

/// A cancellable filter worker: `request` from the UI thread, `try_recv` once per frame.
pub struct FilterWorker {
    jobs: mpsc::Sender<Job>,
    results: mpsc::Receiver<FilterResult>,
    latest: u64,
}

impl FilterWorker {
    pub fn new() -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (result_tx, results) = mpsc::channel::<FilterResult>();
        std::thread::Builder::new()
            .name("clove-search".into())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Coalesce: only the newest pending query is worth filtering.
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    let started = Instant::now();
                    let indices = filter(&job.snapshot.entries, &job.query);
                    let _ = result_tx.send(FilterResult {
                        generation: job.generation,
                        indices,
                        took: started.elapsed(),
                    });
                }
            })
            .expect("spawn clove-search worker");
        FilterWorker {
            jobs,
            results,
            latest: 0,
        }
    }

    /// Queues a filter and returns its generation.
    pub fn request(&mut self, snapshot: Arc<Snapshot>, query: impl Into<String>) -> u64 {
        self.latest += 1;
        let _ = self.jobs.send(Job {
            generation: self.latest,
            snapshot,
            query: query.into(),
        });
        self.latest
    }

    /// The newest result, if it has arrived; stale generations are discarded.
    pub fn try_recv(&mut self) -> Option<FilterResult> {
        while let Ok(result) = self.results.try_recv() {
            if result.generation == self.latest {
                return Some(result);
            }
        }
        None
    }

    /// Blocks (for tests/tools) until the newest generation arrives.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Option<FilterResult> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(result) = self.try_recv() {
                return Some(result);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Default for FilterWorker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[(&str, bool)]) -> Vec<Entry> {
        names
            .iter()
            .map(|(n, d)| Entry::new(*n, *d, 0))
            .collect()
    }

    #[test]
    fn prefix_beats_mid_word_and_boundary_beats_plain() {
        let start = score_name("src", "s").unwrap();
        let boundary = score_name("my_source", "s").unwrap();
        let mid = score_name("assembler", "s").unwrap();
        assert!(start > boundary && boundary > mid, "{start} {boundary} {mid}");
    }

    #[test]
    fn consecutive_matches_score_higher_than_scattered() {
        let tight = score_name("abc", "abc").unwrap();
        let loose = score_name("a_b_c", "abc").unwrap();
        assert!(tight > loose, "{tight} <= {loose}");
    }

    #[test]
    fn fuzzy_subsequence_matches() {
        assert!(score_name("clove", "clv").is_some());
        assert!(score_name("clove", "clx").is_none());
    }

    #[test]
    fn empty_query_returns_everything_in_order() {
        let es = entries(&[("b", false), ("a", true)]);
        assert_eq!(filter(&es, "  "), vec![0, 1]);
    }

    #[test]
    fn dirs_win_ties() {
        let es = entries(&[("src", false), ("src", true)]);
        assert_eq!(filter(&es, "src"), vec![1, 0]);
    }

    #[test]
    fn worker_drops_stale_generations() {
        let snap = Arc::new(Snapshot::new(
            ".",
            entries(&[("alpha", false), ("beta", false), ("gamma", false)]),
        ));
        let mut worker = FilterWorker::new();
        worker.request(snap.clone(), "a");
        let newest = worker.request(snap, "mm");
        let result = worker.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(result.generation, newest);
        assert_eq!(result.indices, vec![2], "gamma is the only 'mm' match");
    }
}
