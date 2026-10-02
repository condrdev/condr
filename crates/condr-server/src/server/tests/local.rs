//! The detached Server's process state does not come from its launcher (ADR 0031).
#![allow(unsafe_code)] // The mitigation policy calls have no safe wrapper.

use super::super::local::{desktop_shell, spawn_detached};
use super::*;
use std::process::Command as StdCommand;
use windows_spawn::{Command, Stdio};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessMitigationPolicy, ProcessRedirectionTrustPolicy,
    SetProcessMitigationPolicy,
};

const LAUNCHER_REPORT: &str = "CONDR_TEST_REDIRECTION_TRUST_LAUNCHER";
const PROBE_REPORT: &str = "CONDR_TEST_REDIRECTION_TRUST_PROBE";

fn redirection_trust() -> u32 {
    let mut flags = 0u32;
    // SAFETY: the pointer and length describe `flags`, a whole
    // PROCESS_MITIGATION_REDIRECTION_TRUST_POLICY.
    unsafe {
        GetProcessMitigationPolicy(
            GetCurrentProcess(),
            ProcessRedirectionTrustPolicy,
            (&raw mut flags).cast(),
            size_of_val(&flags),
        )
    };
    flags
}

/// Run in a process of its own by the test below: it enforces redirection trust, as an
/// installer or an agent's shell tool would, and starts this binary again the way the
/// detached Server is started.
#[test]
fn redirection_trust_launcher_helper() {
    let Some(report) = std::env::var_os(LAUNCHER_REPORT) else {
        return;
    };
    let enforce = 1u32;
    // SAFETY: the pointer and length describe `enforce`.
    let enforced = unsafe {
        SetProcessMitigationPolicy(
            ProcessRedirectionTrustPolicy,
            (&raw const enforce).cast(),
            size_of_val(&enforce),
        )
    } != 0;
    if !enforced {
        std::fs::write(report, "unsupported").unwrap();
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "server::tests::local::redirection_trust_probe",
            "--nocapture",
        ])
        .env(PROBE_REPORT, &report)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    spawn_detached(&mut command).unwrap().wait().unwrap();
}

/// Started by the helper in the Server's place: reports what it inherited.
#[test]
fn redirection_trust_probe() {
    let Some(report) = std::env::var_os(PROBE_REPORT) else {
        return;
    };
    std::fs::write(report, redirection_trust().to_string()).unwrap();
}

#[test]
fn a_server_started_by_a_launcher_that_enforces_redirection_trust_runs_without_it() {
    if desktop_shell().is_none() {
        eprintln!("no desktop shell here: the Server can only start as its launcher's child");
        return;
    }
    let report = std::env::temp_dir().join(format!("condr-redirection-trust-{}", unique_suffix()));
    let status = StdCommand::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "server::tests::local::redirection_trust_launcher_helper",
            "--nocapture",
        ])
        .env(LAUNCHER_REPORT, &report)
        .status()
        .unwrap();
    assert!(status.success());
    let policy = std::fs::read_to_string(&report).unwrap();
    let _ = std::fs::remove_file(&report);
    if policy == "unsupported" {
        eprintln!("this Windows cannot enforce redirection trust");
        return;
    }
    assert_eq!(
        policy, "0",
        "the Server inherited its launcher's redirection trust"
    );
}
