use super::*;
use std::os::windows::io::AsRawHandle;
use std::process::{Command, Stdio};

fn sleeper() -> std::process::Child {
    // cmd runs ping as its own child, so the Job holds a tree, not one process.
    Command::new("cmd")
        .args(["/d", "/c", "ping -n 30 127.0.0.1 >NUL"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "the Job never got there");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_job_lists_and_terminates_the_tree_its_process_starts() {
    let job = Job::new().unwrap();
    let mut root = sleeper();
    job.assign(AsRawHandle::as_raw_handle(&root)).unwrap();
    wait_until(|| job.process_ids().unwrap().len() >= 2);
    assert!(job.process_ids().unwrap().contains(&root.id()));
    assert!(job.active_processes().unwrap() >= 2);

    job.terminate().unwrap();
    assert!(!root.wait().unwrap().success());
    wait_until(|| job.active_processes().unwrap() == 0);
    assert!(job.process_ids().unwrap().is_empty());
}

#[test]
fn closing_a_job_ends_its_processes() {
    let job = Job::new().unwrap();
    let mut root = sleeper();
    job.assign(AsRawHandle::as_raw_handle(&root)).unwrap();
    drop(job);
    wait_until(|| root.try_wait().unwrap().is_some());
}
