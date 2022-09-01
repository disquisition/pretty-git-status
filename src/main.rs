use colored_truecolor::Colorize;
use git2::{
    Branch, Cred, ErrorCode, FetchOptions, RemoteCallbacks, Repository, RepositoryState, Status,
    Statuses,
};
use std::fs;
use std::io::{stdout, Write};
use std::time::Duration;

fn main() {
    let repo = match Repository::open_from_env() {
        Ok(repo) => repo,
        _ => return,
    };

    try_fetch_current_branch(&repo);

    let statuses = match repo.statuses(None) {
        Ok(statuses) => statuses,
        _ => return,
    };

    let head_name = get_head_name(&repo).unwrap_or(String::from("<unknown>"));
    let repo_state = match repo.state() {
        RepositoryState::Merge | RepositoryState::RebaseMerge => Some("MERGING"),
        RepositoryState::Rebase | RepositoryState::RebaseInteractive => Some("REBASING"),
        _ => None,
    };

    let head_label = match repo_state {
        Some(state) => format!("{}|{}", head_name, state),
        _ => head_name,
    };

    let (is_local_only_branch, ahead, behind) = get_head_info(&repo);

    let total_untracked = count_by_status(&statuses, Status::WT_NEW);

    let total_changed = count_by_status(
        &statuses,
        Status::WT_DELETED | Status::WT_MODIFIED | Status::WT_RENAMED | Status::WT_TYPECHANGE,
    );

    let total_staged = count_by_status(
        &statuses,
        Status::INDEX_MODIFIED
            | Status::INDEX_NEW
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE,
    );

    let total_conflicted = count_by_status(&statuses, Status::CONFLICTED);

    let total_stashed = count_stash();

    let mut git_status = String::new();

    git_status.push_str(format!(" on {}", head_label.bold().magenta()).as_str());

    if is_local_only_branch {
        git_status.push_str(" ⬨")
    } else if ahead > 0 || behind > 0 {
        git_status.push(' ');

        if ahead > 0 {
            git_status.push_str(format!("↑{}", ahead).as_str());
        }

        if behind > 0 {
            git_status.push_str(format!("↓{}", behind).as_str());
        }
    }

    if total_untracked > 0 || total_changed > 0 || total_staged > 0 || total_conflicted > 0 {
        git_status.push_str(" (");

        if total_untracked > 0 {
            git_status.push_str(format!("+{}", total_untracked).cyan().to_string().as_str());
        }

        if total_changed > 0 {
            git_status.push_str(
                format!("Δ{}", total_changed)
                    .bright_magenta()
                    .to_string()
                    .as_str(),
            );
        }

        if total_staged > 0 {
            git_status.push_str(format!("●{}", total_staged).red().to_string().as_str());
        }

        if total_conflicted > 0 {
            git_status.push_str(
                format!("✖{}", total_conflicted)
                    .yellow()
                    .to_string()
                    .as_str(),
            );
        }

        git_status.push(')');
    }

    if total_stashed > 0 {
        git_status.push_str(format!(" ⚑{}", total_stashed).as_str());
    }

    stdout().write(git_status.as_bytes()).unwrap();
}

fn try_fetch_current_branch(repo: &Repository) -> Option<()> {
    let head = repo.head().ok()?;

    // If we're not on a branch, don't bother
    if !head.is_branch() {
        return None;
    }

    let mut fetch_head_path = repo.path().to_owned();

    fetch_head_path.push("FETCH_HEAD");

    // If we already fetched in the last 15 minutes, don't bother
    if let Ok(metadata) = fs::metadata(fetch_head_path) {
        let elapsed = metadata.modified().ok()?.elapsed().ok()?;
        let fifteen_minutes = Duration::from_secs(60 * 15);

        if elapsed < fifteen_minutes {
            return None;
        }
    }

    let refname = head.name()?;
    let branch_upstream_remote_buf = repo.branch_upstream_remote(refname).ok()?;
    let branch_upstream_remote = branch_upstream_remote_buf.as_str()?;

    let mut remote = repo.find_remote(branch_upstream_remote).ok()?;

    let branch_upstream_name_buf = repo.branch_upstream_name(refname).ok()?;
    let branch_upstream_name = branch_upstream_name_buf
        .as_str()
        .map(|str| str.split('/').last().to_owned())??;

    let mut callbacks = RemoteCallbacks::new();

    // Look for credentials on the ssh-agent
    callbacks.credentials(
        |_url, username_from_url, _allowed_types| match username_from_url {
            Some(username) => Cred::ssh_key_from_agent(username),
            None => Cred::default(),
        },
    );

    let mut options = FetchOptions::new();

    options.remote_callbacks(callbacks);

    remote
        .fetch(&[branch_upstream_name], Some(&mut options), None)
        .ok()
}

fn get_head_name(repo: &Repository) -> Option<String> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(e) => {
            return if e.code() == ErrorCode::UnbornBranch {
                // HEAD should only be an unborn branch if the repository is fresh,
                // in that case read directly from `.git/HEAD`
                let mut head_path = repo.path().to_path_buf();
                head_path.push("HEAD");

                // get first line, then last path segment
                fs::read_to_string(&head_path)
                    .ok()?
                    .lines()
                    .next()?
                    .trim()
                    .split('/')
                    .last()
                    .map(|r| r.to_owned())
            } else {
                None
            };
        }
    };

    if head.is_branch() {
        return Some(head.shorthand()?.to_owned());
    }

    let mut sha = head.target()?.to_string();

    sha.truncate(7);

    return Some(format!(":{}", sha));
}

fn get_head_info(repo: &Repository) -> (bool, usize, usize) {
    let head = match repo.head() {
        Ok(head) => head,
        _ => return (false, 0, 0),
    };

    if !head.is_branch() {
        return (false, 0, 0);
    }

    let branch = Branch::wrap(head);

    let upstream = match branch.upstream() {
        Ok(upstream) => upstream,
        _ => return (true, 0, 0),
    };

    let branch_oid = match branch.get().target() {
        Some(branch_oid) => branch_oid,
        _ => return (false, 0, 0),
    };

    let upstream_oid = match upstream.get().target() {
        Some(upstream_oid) => upstream_oid,
        _ => return (false, 0, 0),
    };

    let (ahead, behind) = repo
        .graph_ahead_behind(branch_oid, upstream_oid)
        .unwrap_or((0, 0));

    return (false, ahead, behind);
}

fn count_by_status(statuses: &Statuses, status: Status) -> i32 {
    let mut counter = 0;

    for entry in statuses.iter() {
        if entry.status().intersects(status) {
            counter += 1;
        }
    }

    return counter;
}

fn count_stash() -> i32 {
    let mut repo = match Repository::open_from_env() {
        Ok(repo) => repo,
        _ => return 0,
    };

    let mut counter = 0;

    repo.stash_foreach(|_one, _two, _three| {
        counter += 1;

        return true;
    })
    .ok();

    return counter;
}
