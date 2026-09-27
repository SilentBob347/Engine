mod build_support;

use build_support::{git, source_state, BUILD_INPUTS};
use std::{
    env,
    io::{self, Write},
    path::Path,
};

fn valid_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = env::var("CARGO_MANIFEST_DIR")?;
    let root = Path::new(&manifest)
        .parent()
        .and_then(Path::parent)
        .ok_or("missing workspace root")?;
    let mut output = io::stdout().lock();
    for name in ["UC_ENGINE_SOURCE_COMMIT", "UC_ENGINE_SOURCE_STATE"] {
        writeln!(output, "cargo:rerun-if-env-changed={name}")?;
    }
    let explicit = env::var("UC_ENGINE_SOURCE_COMMIT")
        .ok()
        .filter(|value| valid_commit(value));
    // 显式提供来源时，输出只取决于上面两个环境变量；不再跟踪 git 状态与源码目录，
    // 否则源码目录中任何无关文件的修改时间变化都会连锁重编所有依赖本 crate 的包。
    if explicit.is_none() {
        for path in BUILD_INPUTS {
            writeln!(
                output,
                "cargo:rerun-if-changed={}",
                root.join(path).display()
            )?;
        }
        for name in ["HEAD", "index", "packed-refs"] {
            if let Some(path) = git(
                root,
                &["rev-parse", "--path-format=absolute", "--git-path", name],
            ) {
                writeln!(output, "cargo:rerun-if-changed={path}")?;
            }
        }
        if let Some(reference) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
            if let Some(path) = git(
                root,
                &[
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-path",
                    &reference,
                ],
            ) {
                writeln!(output, "cargo:rerun-if-changed={path}")?;
            }
        }
    }
    let commit = explicit
        .clone()
        .or_else(|| git(root, &["rev-parse", "HEAD"]).filter(|value| valid_commit(value)))
        .unwrap_or_else(|| "unknown".into());
    let state = if explicit.is_some() {
        env::var("UC_ENGINE_SOURCE_STATE")
            .ok()
            .filter(|state| matches!(state.as_str(), "clean" | "modified" | "unknown"))
            .unwrap_or_else(|| "unknown".into())
    } else {
        source_state(root)
    };
    writeln!(
        output,
        "cargo:rustc-env=UC_OBSERVABILITY_SOURCE_COMMIT={commit}"
    )?;
    writeln!(
        output,
        "cargo:rustc-env=UC_OBSERVABILITY_SOURCE_STATE={state}"
    )?;
    Ok(())
}
