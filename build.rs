use std::env;
use std::process::Command;

fn main() {
    let package_version = env::var("CARGO_PKG_VERSION").expect("Cargo sets CARGO_PKG_VERSION");
    let mut parts = package_version.split('.');
    let major = parts.next().expect("package version has a major component");
    let minor = parts.next().expect("package version has a minor component");
    let minor_number: u64 = minor.parse().expect("package minor version is numeric");

    let version = if minor_number & 1 == 0 {
        format!("{major}.{minor}")
    } else {
        let commit_count = env::var("MONITOR_DHCP_COMMIT_COUNT")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(git_commit_count)
            .unwrap_or_else(|| "0".to_owned());
        format!("{major}.{minor}.{commit_count}")
    };

    println!("cargo:rustc-env=MONITOR_DHCP_VERSION={version}");
    println!("cargo:rerun-if-env-changed=MONITOR_DHCP_COMMIT_COUNT");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/logs/HEAD");
}

fn git_commit_count() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|count| count.trim().to_owned())
        .filter(|count| !count.is_empty())
}
