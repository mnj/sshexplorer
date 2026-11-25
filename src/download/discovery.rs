//! Depth-first discovery runs beside file copying, with bounded lookahead.
use super::{Entry, Halt, Item, Job, NOT_FOLLOWED, Note, Remote, descendant, join, local};
use std::collections::BTreeSet;
use tokio::sync::{mpsc, oneshot};

pub(super) enum Found {
    Item(Item),
    Failed(Note),
    Skipped(Note),
    Lost(u64, oneshot::Sender<(Remote, u64)>),
}

fn note(rel: &[String], reason: impl Into<String>) -> Note {
    Note {
        path: rel.join("/"),
        reason: reason.into(),
    }
}

async fn send(tx: &mpsc::Sender<Found>, item: Found) -> Result<(), Halt> {
    tx.send(item).await.map_err(|_| Halt::Cancelled)
}

async fn reconnect(tx: &mpsc::Sender<Found>, generation: u64) -> Result<(Remote, u64), Halt> {
    let (reply, resumed) = oneshot::channel();
    send(tx, Found::Lost(generation, reply)).await?;
    resumed.await.map_err(|_| Halt::Cancelled)
}

pub(super) async fn walk(
    job: &Job,
    paths: &[String],
    mut remote_conn: Remote,
    tx: mpsc::Sender<Found>,
) -> Result<(usize, Option<u64>), Halt> {
    let mut generation = 0;
    let mut files = 0;
    let mut bytes = Some(0u64);
    let mut seen = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut stack: Vec<(String, Vec<String>, Entry)> = job
        .entries
        .iter()
        .filter(|e| names.insert(e.name.clone()))
        .rev()
        .map(|e| (join(&job.dir, &e.name), vec![e.name.clone()], e.clone()))
        .collect();
    while let Some((remote, rel, mut entry)) = stack.pop() {
        if !paths.is_empty() {
            let path = rel.join("/");
            if !paths.iter().any(|failed| {
                path == *failed || descendant(failed, &path) || descendant(&path, failed)
            }) {
                continue;
            }
        }
        if !local::valid_name(&entry.name) {
            send(&tx, Found::Failed(note(&rel, "Invalid local filename"))).await?;
            continue;
        }
        if !entry.is_dir {
            if (entry.size.is_none()
                || (job.options.keep_modified && entry.modified.is_none())
                || (job.options.keep_executable && entry.permissions.is_none()))
                && remote_conn
                    .complete_file_metadata(&remote, &mut entry)
                    .await
                    .is_err()
                && remote_conn.closed().await
            {
                stack.push((remote, rel, entry));
                (remote_conn, generation) = reconnect(&tx, generation).await?;
                continue;
            }
            files += 1;
            bytes = bytes.and_then(|sum| sum.checked_add(entry.size?));
            send(&tx, Found::Item(Item::File { remote, rel, entry })).await?;
            continue;
        }
        if entry.link.is_some() && !job.options.follow_links {
            send(&tx, Found::Skipped(note(&rel, NOT_FOLLOWED))).await?;
            continue;
        }
        let real = remote_conn
            .canonicalize(remote.clone())
            .await
            .unwrap_or_else(|_| remote.clone());
        if !seen.insert(real.clone()) {
            continue;
        }
        match remote_conn.list(remote.clone()).await {
            Err(_) if remote_conn.closed().await => {
                // Walked again on the new connection.
                seen.remove(&real);
                stack.push((remote, rel, entry));
                (remote_conn, generation) = reconnect(&tx, generation).await?;
            }
            Ok(listing) => {
                send(&tx, Found::Item(Item::Dir { rel: rel.clone() })).await?;
                let dir = rel.join("/");
                for path in paths {
                    if let Some(rest) = path.strip_prefix(&dir).and_then(|s| s.strip_prefix('/')) {
                        let child = rest.split('/').next().unwrap_or(rest);
                        if !listing.entries.iter().any(|e| e.name == child) {
                            send(
                                &tx,
                                Found::Failed(Note {
                                    path: path.clone(),
                                    reason: "No such file".into(),
                                }),
                            )
                            .await?;
                        }
                    }
                }
                let mut entries = listing.entries;
                entries.sort_by(|a, b| b.name.cmp(&a.name));
                for child in entries {
                    let mut child_rel = rel.clone();
                    child_rel.push(child.name.clone());
                    stack.push((join(&remote, &child.name), child_rel, child));
                }
            }
            Err(reason) => send(&tx, Found::Failed(note(&rel, reason))).await?,
        }
    }
    Ok((files, bytes))
}
