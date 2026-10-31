//! The writes of G-12 to G-14 and G-16 through the remote server (ADR-0037): each made in a tab of a server
//! started with `--allow-writes` on a repository `git` made, and by the shared `Host` as the
//! desktop host performs it on the repository's twin, the two ending alike; each that discards
//! work refused without the developer's confirmation and performed with it (Q-7).

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
// The twin is copied with `cp`, as the writer's own tests, which run on unix, copy theirs.
#![cfg(unix)]

mod browser;
#[path = "../../git/tests/shown/mod.rs"]
mod shown;
#[path = "../../git/tests/support/mod.rs"]
mod support;

use browser::{Pair, rev, three_commits, with_changes};
use ocelli_core::port::{Conflict, Operation, ResetMode, Take, Write};
use serde_json::{Value, json};
use shown::{stage_in, unstage_in};
use std::path::Path;
use support::{git, write_file};

/// The history the writes start from, in `root`, by Ada, whom its configuration names:
///
/// - `main`: `one` (a, b, k), `two` (a), `three` (k), checked out
/// - `done` at `two`, merged; `topic` from `two`, adding n, not merged; `clash`, `picked`, and
///   `apart` from `one`, each changing k as `main` and the others do not
/// - the lightweight tag `old` at `one`, the annotated tag `v1` at `two`
/// - the remote `origin`, which no test reaches, with its branch `feature` at `topic`
fn history(root: &Path) {
    git(root, &["init", "--quiet", "-b", "main"]);
    git(root, &["config", "user.name", "Ada"]);
    git(root, &["config", "user.email", "ada@example.invalid"]);
    for f in ["a", "b", "k"] {
        write_file(root, f, "1\n");
    }
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "one"]);
    git(root, &["tag", "old"]);
    write_file(root, "a", "2\n");
    git(root, &["commit", "--quiet", "-am", "two"]);
    git(root, &["branch", "done"]);
    git(root, &["tag", "-a", "v1", "-m", "the first"]);
    git(root, &["switch", "--quiet", "-c", "topic"]);
    write_file(root, "n", "n\n");
    git(root, &["add", "n"]);
    git(root, &["commit", "--quiet", "-m", "topic work"]);
    let topic = rev(root, "topic");
    git(root, &["remote", "add", "origin", "../nowhere"]);
    git(root, &["update-ref", "refs/remotes/origin/feature", &topic]);
    for side in ["clash", "picked", "apart"] {
        git(root, &["switch", "--quiet", "-c", side, "old"]);
        write_file(root, "k", &format!("{side}\n"));
        git(root, &["commit", "--quiet", "-am", &format!("{side} work")]);
    }
    git(root, &["switch", "--quiet", "main"]);
    write_file(root, "k", "main\n");
    git(root, &["commit", "--quiet", "-am", "three"]);
}

#[tokio::test]
// Verifies: ADR-0037, ADR-draft-merge-editor, G-18, G-12
async fn with_allow_writes_staging_unstaging_and_committing_leave_the_repository_as_the_desktop_does()
 {
    let mut pair = Pair::new("staging", |root| {
        three_commits(root);
        with_changes(root);
    })
    .await;
    let head = rev(&pair.root, "HEAD");
    // Each write as the web interface makes it from what it shows now.
    let writes: [fn(&Path) -> Write; 3] = [
        |root| stage_in(root, "f"),
        |root| stage_in(root, "new"),
        |root| unstage_in(root, "new"),
    ];
    for write in writes {
        assert_eq!(pair.alike(write, false).await, Ok(Value::Null));
    }
    let commit = move |_: &Path| Write::Commit {
        message: "from the browser".into(),
        head: Some(head.clone()),
    };
    let (here, there) = pair.answers(commit, false).await;
    // Each side's commit answers the commit it made.
    assert_eq!(here, Ok(json!(rev(&pair.root, "HEAD"))));
    assert_eq!(there, Ok(json!(rev(&pair.other, "HEAD"))));
    let there = there.unwrap().as_str().unwrap().to_string();
    // As the user the server runs as, with the identity the repository's configuration names.
    assert_eq!(
        git(&pair.root, &["log", "-1", "--format=%an <%ae>"]),
        "Ada <ada@example.invalid>"
    );
    // The load after the write selects the commit it made, as the desktop's does (ADR-0033).
    let here = here.unwrap();
    pair.tab.events.clear();
    let reload = json!({ "path": ".", "select": here });
    assert_eq!(pair.tab.invoke("open_repo", reload).await, Ok(Value::Null));
    let grew = pair
        .tab
        .until("history-grew", |p| !p["select"].is_null())
        .await;
    assert_eq!(
        grew["select"],
        pair.desktop.reload(&pair.other, &there).await
    );
    let selected = pair
        .tab
        .invoke("select_row", json!({ "row": grew["select"] }))
        .await
        .unwrap();
    assert_eq!(selected["node"]["sha"], here);
}

#[tokio::test]
// Verifies: ADR-0037, ADR-draft-merge-editor, ADR-0034, G-18, G-12, Q-7
async fn branches_written_in_the_browser_end_as_the_desktop_leaves_them() {
    let mut pair = Pair::new("branches", |root| {
        history(root);
        // A change the switches take along: no branch changes b.
        write_file(root, "b", "changed\n");
    })
    .await;
    let made = |answer: Result<Value, String>| assert_eq!(answer, Ok(Value::Null));
    made(
        pair.alike(
            |d| Write::CreateBranch {
                name: "beside".into(),
                commit: rev(d, "done"),
                switch: false,
            },
            false,
        )
        .await,
    );
    made(
        pair.alike(
            |d| Write::CreateBranch {
                name: "here".into(),
                commit: rev(d, "HEAD"),
                switch: true,
            },
            false,
        )
        .await,
    );
    made(
        pair.alike(
            |d| Write::SwitchBranch {
                name: "topic".into(),
                tip: rev(d, "topic"),
            },
            false,
        )
        .await,
    );
    made(
        pair.alike(
            |d| Write::Detach {
                commit: rev(d, "old"),
            },
            false,
        )
        .await,
    );
    made(
        pair.alike(
            |d| Write::TrackRemote {
                remote: "origin".into(),
                branch: "feature".into(),
                tip: rev(d, "origin/feature"),
            },
            false,
        )
        .await,
    );
    // Merged into feature, which is checked out now: deleted without a question.
    made(
        pair.alike(
            |d| Write::DeleteBranch {
                name: "done".into(),
                tip: rev(d, "done"),
                merged: true,
            },
            false,
        )
        .await,
    );
    // main is not merged into feature: deleted only once the developer confirms it.
    made(
        pair.confirmed(|d| Write::DeleteBranch {
            name: "main".into(),
            tip: rev(d, "main"),
            merged: false,
        })
        .await,
    );
    assert_eq!(git(&pair.root, &["branch", "--show-current"]), "feature");
    assert!(!support::git_may_fail(
        &pair.root,
        &["rev-parse", "--verify", "--quiet", "refs/heads/main"]
    ));
}

/// The commit of the stash entry at `index` in `dir`.
fn stash(dir: &Path, index: usize) -> String {
    rev(dir, &format!("stash@{{{index}}}"))
}

#[tokio::test]
// Verifies: ADR-0037, ADR-draft-merge-editor, ADR-0034, G-18, G-12, Q-7
async fn stashes_and_tags_written_in_the_browser_end_as_the_desktop_leaves_them() {
    let mut pair = Pair::new("stashes-tags", |root| {
        history(root);
        write_file(root, "b", "changed\n");
        write_file(root, "u", "untracked\n");
    })
    .await;
    let done = |answer: Result<Value, String>| assert_eq!(answer, Ok(Value::Null));

    // Tags: a lightweight and an annotated one made, a name taken refused, an annotated tag
    // refused where the configuration asks for it to be signed, and each kind deleted once the
    // developer confirms it.
    done(
        pair.alike(
            |d| Write::CreateTag {
                name: "light".into(),
                commit: rev(d, "HEAD"),
                message: None,
            },
            false,
        )
        .await,
    );
    done(
        pair.alike(
            |d| Write::CreateTag {
                name: "v2".into(),
                commit: rev(d, "topic"),
                message: Some("the second".into()),
            },
            false,
        )
        .await,
    );
    let taken = |d: &Path| Write::CreateTag {
        name: "old".into(),
        commit: rev(d, "HEAD"),
        message: None,
    };
    pair.refused(taken, false, "exists already").await;
    pair.on_both(|d| {
        git(d, &["config", "tag.gpgSign", "true"]);
    });
    let signed = |d: &Path| Write::CreateTag {
        name: "v3".into(),
        commit: rev(d, "HEAD"),
        message: Some("signed".into()),
    };
    pair.refused(signed, false, "tag.gpgSign asks for").await;
    pair.on_both(|d| {
        git(d, &["config", "--unset", "tag.gpgSign"]);
    });
    done(
        pair.confirmed(|d| Write::DeleteTag {
            name: "old".into(),
            commit: rev(d, "old"),
            annotated: false,
        })
        .await,
    );
    done(
        pair.confirmed(|d| Write::DeleteTag {
            name: "v1".into(),
            commit: rev(d, "v1"),
            annotated: true,
        })
        .await,
    );

    // Stashes: the change and the untracked file put aside and applied back, the change put
    // aside again, applying the older entry refused while the index holds a staged change,
    // then popped once confirmed, and the newer dropped once confirmed.
    done(
        pair.alike(
            |_| Write::CreateStash {
                message: Some("first".into()),
                untracked: true,
            },
            false,
        )
        .await,
    );
    done(
        pair.alike(
            |d| Write::ApplyStash {
                index: 0,
                commit: stash(d, 0),
                drop: false,
            },
            false,
        )
        .await,
    );
    done(
        pair.alike(
            |_| Write::CreateStash {
                message: None,
                untracked: false,
            },
            false,
        )
        .await,
    );
    pair.on_both(|d| {
        std::fs::remove_file(d.join("u")).unwrap();
        write_file(d, "k", "staged\n");
        git(d, &["add", "k"]);
    });
    let pop = |d: &Path| Write::ApplyStash {
        index: 1,
        commit: stash(d, 1),
        drop: true,
    };
    pair.refused(pop, true, "the index holds staged changes")
        .await;
    pair.on_both(|d| {
        git(d, &["reset", "--quiet", "--hard"]);
    });
    done(pair.confirmed(pop).await);
    done(
        pair.confirmed(|d| Write::DropStash {
            index: 0,
            commit: stash(d, 0),
        })
        .await,
    );
    assert_eq!(git(&pair.root, &["stash", "list"]), "");
    assert_eq!(
        std::fs::read_to_string(pair.root.join("u")).unwrap(),
        "untracked\n"
    );
}

/// The conflict on `path` in `dir` as the web interface shows it: the blob ids of the index's
/// stages, read off `git`.
fn conflict(dir: &Path, path: &str) -> Conflict {
    let mut stages = [None, None, None];
    for line in git(dir, &["ls-files", "-u", "--", path]).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let stage: usize = fields[2].parse().unwrap();
        stages[stage - 1] = Some(fields[1].to_string());
    }
    let [ancestor, ours, theirs] = stages;
    Conflict {
        path: path.into(),
        ancestor,
        ours,
        theirs,
    }
}

/// The write the toolbar of the checked-out `main` makes from what `dir` shows.
fn on_main(dir: &Path) -> (String, String) {
    ("main".into(), rev(dir, "main"))
}

#[tokio::test]
// Verifies: ADR-0037, ADR-draft-merge-editor, ADR-0034, G-18, G-13, G-14, Q-7
async fn history_operations_and_their_conflicts_in_the_browser_end_as_the_desktop_leaves_them() {
    let mut pair = Pair::new("operations", history).await;
    let made = |answer: &Result<Value, String>| {
        assert!(matches!(answer, Ok(Value::String(_))), "{answer:?}")
    };
    let stopped = |answer: Result<Value, String>| assert_eq!(answer, Ok(Value::Null));
    let resolve = |take: Take| {
        move |d: &Path| Write::Resolve {
            path: "k".into(),
            conflict: conflict(d, "k"),
            take,
        }
    };
    let go_on = |operation: Operation| {
        move |d: &Path| Write::Continue {
            operation,
            head: rev(d, "HEAD"),
        }
    };

    // A rebase stops on main's change to k, which clash changed too; continuing is refused
    // while k conflicts, and goes on once k is resolved as edited.
    stopped(
        pair.alike(
            |d| {
                let (branch, tip) = on_main(d);
                Write::Rebase {
                    branch,
                    tip,
                    onto: rev(d, "clash"),
                }
            },
            false,
        )
        .await,
    );
    pair.refused(go_on(Operation::Rebase), false, "files still conflict")
        .await;
    pair.on_both(|d| write_file(d, "k", "resolved\n"));
    stopped(pair.alike(resolve(Take::AsEdited), false).await);
    made(&pair.alike(go_on(Operation::Rebase), false).await);

    // A merge of topic, which makes a merge commit; reverting that commit is git's to do.
    let merge = |name: &'static str| {
        move |d: &Path| {
            let (branch, tip) = on_main(d);
            Write::Merge {
                branch,
                tip,
                commit: rev(d, name),
                name: Some(name.into()),
            }
        }
    };
    made(&pair.alike(merge("topic"), false).await);
    let revert = |of: &'static str| {
        move |d: &Path| {
            let (branch, tip) = on_main(d);
            Write::Revert {
                branch,
                tip,
                commit: rev(d, of),
            }
        }
    };
    pair.refused(revert("main"), false, "is a merge commit")
        .await;
    made(&pair.alike(revert("main^1"), false).await);

    // A cherry-pick stops on k; taking their side discards what the working directory holds,
    // so it runs once confirmed, and the pick then commits.
    stopped(
        pair.alike(
            |d| {
                let (branch, tip) = on_main(d);
                Write::CherryPick {
                    branch,
                    tip,
                    commit: rev(d, "picked"),
                }
            },
            false,
        )
        .await,
    );
    stopped(pair.confirmed(resolve(Take::Theirs)).await);
    made(&pair.alike(go_on(Operation::CherryPick), false).await);

    // A merge that stops, aborted once confirmed: back to where it started.
    let before = git(&pair.root, &["rev-parse", "HEAD"]);
    stopped(pair.alike(merge("apart"), false).await);
    stopped(
        pair.confirmed(|d| Write::Abort {
            operation: Operation::Merge,
            head: rev(d, "HEAD"),
        })
        .await,
    );
    assert_eq!(git(&pair.root, &["rev-parse", "HEAD"]), before);

    // Resets: soft and mixed without a question, hard only once confirmed.
    let reset = |mode: ResetMode| {
        move |d: &Path| {
            let (branch, tip) = on_main(d);
            Write::Reset {
                branch,
                tip,
                commit: rev(d, "main~1"),
                mode,
            }
        }
    };
    made(&pair.alike(reset(ResetMode::Soft), false).await);
    made(&pair.alike(reset(ResetMode::Mixed), false).await);
    made(&pair.confirmed(reset(ResetMode::Hard)).await);
}
