//! D4 降级回退：旧版资料升到新版并写入新内容后，再用旧版打开同一资料。
//!
//! 期望（计划 051 已确认的决定第 1 项）：旧版明确报告需要处理，或在能读懂资料时正常工作；
//! 两种情况下都不得改写资料，再次用新版打开时内容与降级前一致。
//! “明确报告”指拒绝启动、启动终态未就绪或解锁返回错误码；启动后部分查询失败既不是明确报告，
//! 也不是正常工作，按失败记录。

use serde_json::{Value, json};
use uc_testkit::{FailureKind, ScenarioFailure};

use crate::{
    catalog::Point,
    cell::CellRun,
    device::{Device, Launch, failure, history_contains, write_representative_content},
    dimensions::{require_preserved, upgrade::upgrade_and_verify},
    fixture::{Content, content_digest, representative},
};

pub(crate) async fn pair(
    run: &mut CellRun,
    from: &Point,
    to: &Point,
) -> Result<(), ScenarioFailure> {
    let mut device = Device::new(run, "a", "Device A")?;
    let before = {
        let _stage = run.stage("old-write");
        device.start(run, from).await?;
        device.create_space().await?;
        write_representative_content(&mut device, &representative("a-0")).await?;
        let observed = device.unlocked_observation().await?;
        device.stop(run).await?;
        observed
    };
    upgrade_and_verify(run, &mut device, to, &before).await?;
    let newer_content = representative("a-1");
    let upgraded = {
        let _stage = run.stage("continue");
        for content in &newer_content {
            device.capture(content).await?;
        }
        let observed = device.unlocked_observation().await?;
        device.stop(run).await?;
        observed
    };
    let downgraded = {
        let _stage = run.stage("downgrade");
        let outcome = open_older(run, &mut device, from, &upgraded, &newer_content).await;
        if let Ok(outcome) = &outcome {
            run.fact("downgrade_outcome", outcome.clone());
        }
        device.stop(run).await?;
        outcome
    };
    // 降级阶段失败时仍核对新版再次打开的结果作为附加证据；单元结论保留首次失败。
    let reopened = reopen(run, &mut device, to, &upgraded).await;
    match (downgraded, reopened) {
        (Err(first), reopened) => {
            run.fact("reopen_after_failed_downgrade", json!(reopened.is_ok()));
            run.restore_failed_stage("downgrade");
            Err(first)
        }
        (Ok(_), reopened) => reopened,
    }
}

async fn reopen(
    run: &mut CellRun,
    device: &mut Device,
    to: &Point,
    upgraded: &Value,
) -> Result<(), ScenarioFailure> {
    let _stage = run.stage("reopen");
    device.start(run, to).await?;
    let reopened = device.unlocked_observation().await?;
    require_preserved(run, upgraded, &reopened, "state-changed-by-downgrade")?;
    device.stop(run).await
}

/// 旧版打开较新资料：拒绝启动、启动未就绪或公开操作返回错误码都属于明确报告；
/// 启动成功且内容完整可读属于正常工作；其余（静默缺失或改变内容）为失败。
async fn open_older(
    run: &mut CellRun,
    device: &mut Device,
    from: &Point,
    upgraded: &Value,
    newer_content: &[Content],
) -> Result<Value, ScenarioFailure> {
    let startup = match device.launch(run, from).await? {
        Launch::Refused { reply } => {
            return Ok(json!({
                "kind": "refused",
                "error": reply["error"],
                "code": reply["code"],
                "startup": reply["startup"],
            }));
        }
        Launch::Ready { startup } => startup,
    };
    if !startup.is_null() && startup["state"] != "ready" {
        return Ok(json!({ "kind": "startup_not_ready", "startup": startup }));
    }
    let observed = device.observe().await?;
    let unlocked = if observed["encryption"]["session_ready"] == true {
        observed
    } else {
        let reply = device.raw("unlock", Value::Null).await?;
        if reply.get("ok").is_none() {
            return Ok(json!({ "kind": "unlock_reported", "code": reply["code"] }));
        }
        device.observe().await?
    };
    // 启动成功后个别查询返回错误码不算“明确报告需要处理”，也不算正常工作：如实记录并失败。
    let failed: serde_json::Map<String, Value> =
        ["setup", "local_device", "devices", "settings", "history"]
            .into_iter()
            .filter(|field| unlocked[*field].get("error").is_some())
            .map(|field| (field.to_owned(), unlocked[field]["code"].clone()))
            .collect();
    if !failed.is_empty() {
        run.fact("downgrade_failed_queries", Value::Object(failed));
        return Err(failure(
            FailureKind::ProductInvariant,
            "downgrade-queries-failed",
        ));
    }
    let readable = newer_content
        .iter()
        .all(|content| history_contains(&unlocked, &content_digest(content)));
    let unchanged = ["setup", "local_device", "settings"]
        .into_iter()
        .all(|field| unlocked[field] == upgraded[field]);
    if readable && unchanged {
        return Ok(json!({ "kind": "working", "upgrade_status": unlocked["upgrade"] }));
    }
    run.fact(
        "downgrade_observation",
        json!({ "newer_content_readable": readable, "identity_and_settings_unchanged": unchanged }),
    );
    Err(failure(
        FailureKind::ProductInvariant,
        "downgrade-silently-inconsistent",
    ))
}
