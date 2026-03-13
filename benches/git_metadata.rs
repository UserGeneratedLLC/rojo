use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion};

fn test_repo() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    let path = PathBuf::from(home).join("workspace/escape-tsunami-for-brainrots");
    assert!(
        path.join(".git").is_dir(),
        "Test repo not found at {}",
        path.display()
    );
    path
}

fn bench_git_status_uall(c: &mut Criterion) {
    let repo = test_repo();

    let mut group = c.benchmark_group("git_changed_files");
    group.sample_size(10);

    group.bench_function("git_status_porcelain_uall", |b| {
        b.iter(|| {
            Command::new("git")
                .args([
                    "status",
                    "--porcelain",
                    "--no-renames",
                    "-uall",
                    "--",
                    "src",
                ])
                .current_dir(&repo)
                .output()
                .unwrap()
        })
    });

    group.bench_function("git_diff_index_HEAD", |b| {
        b.iter(|| {
            Command::new("git")
                .args([
                    "--no-optional-locks",
                    "diff-index",
                    "--name-only",
                    "HEAD",
                    "--",
                    "src",
                ])
                .current_dir(&repo)
                .output()
                .unwrap()
        })
    });

    group.bench_function("git_ls_files_others", |b| {
        b.iter(|| {
            Command::new("git")
                .args([
                    "--no-optional-locks",
                    "ls-files",
                    "--others",
                    "--exclude-standard",
                    "--",
                    "src",
                ])
                .current_dir(&repo)
                .output()
                .unwrap()
        })
    });

    group.bench_function("diff_index_plus_ls_files_parallel", |b| {
        b.iter(|| {
            let repo_a = repo.clone();
            let repo_b = repo.clone();
            std::thread::scope(|s| {
                let h1 = s.spawn(move || {
                    Command::new("git")
                        .args([
                            "--no-optional-locks",
                            "diff-index",
                            "--name-only",
                            "HEAD",
                            "--",
                            "src",
                        ])
                        .current_dir(&repo_a)
                        .output()
                        .unwrap()
                });
                let h2 = s.spawn(move || {
                    Command::new("git")
                        .args([
                            "--no-optional-locks",
                            "ls-files",
                            "--others",
                            "--exclude-standard",
                            "--",
                            "src",
                        ])
                        .current_dir(&repo_b)
                        .output()
                        .unwrap()
                });
                (h1.join().unwrap(), h2.join().unwrap())
            })
        })
    });

    group.bench_function("cache_staleness_check", |b| {
        b.iter(|| {
            let index_path = repo.join(".git/index");
            let mtime = std::fs::metadata(&index_path).unwrap().modified().unwrap();
            let head = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo)
                .output()
                .unwrap();
            (mtime, head)
        })
    });

    group.finish();

    eprintln!("\n--- Quick wall-clock summary ---");
    let runs: &[(&str, &[&str])] = &[
        (
            "git status -uall (current)",
            &[
                "status",
                "--porcelain",
                "--no-renames",
                "-uall",
                "--",
                "src",
            ],
        ),
        (
            "git diff-index HEAD (new)",
            &[
                "--no-optional-locks",
                "diff-index",
                "--name-only",
                "HEAD",
                "--",
                "src",
            ],
        ),
    ];
    for (label, args) in runs {
        let t = Instant::now();
        Command::new("git")
            .args(*args)
            .current_dir(&repo)
            .output()
            .unwrap();
        eprintln!("  {}: {}ms", label, t.elapsed().as_millis());
    }
}

criterion_group!(benches, bench_git_status_uall);
criterion_main!(benches);
