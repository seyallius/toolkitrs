use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_subcommands() {
    Command::cargo_bin("toolkitrs")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("ts2mp4"))
        .stdout(predicate::str::contains("vidwrap"));
}

#[test]
fn vidwrap_requires_a_video() {
    Command::cargo_bin("toolkitrs")
        .unwrap()
        .arg("vidwrap")
        .assert()
        .failure()
        .stderr(predicate::str::contains("<VIDEO>"));
}
