//! 矩阵维度：各维度只声明版本顺序、动作与期望，进程、资料与证据由单元运行器和设备负责。

mod downgrade;
mod mixed_versions;
mod sequential_upgrade;
mod upgrade;

use serde_json::{Value, json};
use uc_testkit::{FailureKind, ScenarioFailure};

use crate::{
    catalog::{CellSpec, Dimension, Versions},
    cell::CellRun,
    device::failure,
};

pub(crate) async fn run(run: &mut CellRun, spec: &CellSpec) -> Result<(), ScenarioFailure> {
    match (&spec.versions, spec.dimension) {
        (Versions::Pair { from, to }, Dimension::Upgrade) => upgrade::pair(run, from, to).await,
        (Versions::Chain(points), Dimension::Upgrade) => upgrade::chain(run, points).await,
        (Versions::Pair { from, to }, Dimension::SequentialUpgrade) => {
            sequential_upgrade::pair(run, from, to).await
        }
        (Versions::Chain(points), Dimension::SequentialUpgrade) => {
            sequential_upgrade::chain(run, points).await
        }
        (Versions::Pair { from, to }, Dimension::MixedVersions) => {
            let inviter = spec
                .inviter
                .ok_or_else(|| failure(FailureKind::FixtureInvalid, "inviter-missing"))?;
            mixed_versions::pair(run, from, to, inviter).await
        }
        (Versions::Pair { from, to }, Dimension::Downgrade) => downgrade::pair(run, from, to).await,
        _ => Err(failure(
            FailureKind::FixtureInvalid,
            "cell-shape-unsupported",
        )),
    }
}

/// 升级、降级前后必须保持一致的可观察字段。
const PRESERVED: [&str; 5] = ["setup", "local_device", "devices", "settings", "history"];

/// 可比较状态的脱敏摘要：条目数与保留字段的哈希，不含原始内容或标识。
pub(crate) fn observation_summary(observed: &Value) -> Value {
    let preserved: Vec<&Value> = PRESERVED.iter().map(|field| &observed[*field]).collect();
    json!({
        "history_entries": observed["history"].as_array().map(Vec::len),
        "devices": observed["devices"].as_array().map(Vec::len),
        "preserved_digest": crate::fixture::digest(Value::from(preserved.into_iter().cloned().collect::<Vec<_>>()).to_string().as_bytes()),
    })
}

/// 比较两次观察；不一致时把不同字段名记入单元事实并以 `condition` 失败。
pub(crate) fn require_preserved(
    run: &mut CellRun,
    before: &Value,
    after: &Value,
    condition: &'static str,
) -> Result<(), ScenarioFailure> {
    let changed: Vec<&str> = PRESERVED
        .iter()
        .copied()
        .filter(|field| before[*field] != after[*field])
        .collect();
    run.fact(
        &format!("{condition}-checked"),
        json!({ "before": observation_summary(before), "after": observation_summary(after) }),
    );
    if changed.is_empty() {
        return Ok(());
    }
    run.fact(condition, json!({ "changed_fields": changed }));
    Err(failure(FailureKind::ProductInvariant, condition))
}
