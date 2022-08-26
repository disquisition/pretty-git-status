use colored_truecolor::Colorize;
use git2::{Branch, ErrorCode, Repository, RepositoryState, Status, Statuses};
use std::env;
use std::fs;
use std::io::{stdout, Write};
use std::path::PathBuf;

fn main() {
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(_e) => return,
    };

    let repo = match Repository::open(&cwd) {
        Ok(repo) => repo,
        Err(_e) => return,
    };

    let statuses = match repo.statuses(None) {
        Ok(statuses) => statuses,
        Err(_e) => return,
    };

    let repo_state = match repo.state() {
        RepositoryState::Merge | RepositoryState::RebaseMerge => String::from("MERGING"),
        RepositoryState::Rebase | RepositoryState::RebaseInteractive => String::from("REBASING"),
        _ => String::new(),
    };

    let head_label = match get_head_name(&repo) {
        Some(name) => {
            if repo_state.is_empty() {
                name
            } else {
                format!("{}|{}", name, repo_state)
            }
        }
        None => String::from("<unknown>"),
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

    let total_stashed = count_stash(&cwd);

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
        Ok(head_ref) => head_ref,
        Err(_e) => return (false, 0, 0),
    };

    if !head.is_branch() {
        return (false, 0, 0);
    }

    let branch = Branch::wrap(head);

    let upstream = match branch.upstream() {
        Ok(upstream) => upstream,
        Err(_e) => return (true, 0, 0),
    };

    return match repo.graph_ahead_behind(
        branch.get().target().unwrap(),
        upstream.get().target().unwrap(),
    ) {
        Ok((ahead, behind)) => (false, ahead, behind),
        Err(_e) => (false, 0, 0),
    };
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

fn count_stash(path: &PathBuf) -> i32 {
    let mut repo = match Repository::open(path) {
        Ok(repo) => repo,
        Err(_e) => return 0,
    };

    let mut counter = 0;

    return match repo.stash_foreach(|_one, _two, _three| {
        counter += 1;

        return true;
    }) {
        Ok(_value) => counter,
        Err(_e) => 0,
    };
}
