use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let repo = gix::discover(".").ok();

    let git_hash = match repo {
        Some(repo) => repo
            .head()
            .ok()
            .and_then(|mut head| head.peel_to_commit().ok())
            .and_then(|commit| commit.short_id().ok())
            .map(|prefix| prefix.to_string()),
        None => None,
    };

    // If no Git repository available, read REV from Nix definition
    let git_hash = git_hash.clone().unwrap_or_else(|| {
        option_env!("PROMETHEUS_SD_NEXTHOP_NIX_BUILD_REV")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    });

    let git_hash_short = git_hash.get(0..10).unwrap_or_else(|| &git_hash);

    println!("cargo:rustc-env=BUILD_GIT_HASH={}", git_hash_short);

    Ok(())
}
